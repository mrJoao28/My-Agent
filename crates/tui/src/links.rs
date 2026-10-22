//! Desktop integration for ephemeral login links; no shell interpolation or logs.
use myagent_core::CancellationToken;
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkResult {
    Opened,
    Copied,
    TerminalCopy,
    Unavailable,
    Cancelled,
}

pub fn valid_login_url(value: &str) -> bool {
    if value.len() > 32_768 || value.chars().any(char::is_control) {
        return false;
    }
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("auth.openai.com")
            && url.path() == "/api/accounts/authorize"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && url.fragment().is_none()
            && url.query().is_some()
    })
}
#[derive(Clone, Copy)]
enum Desktop {
    Linux { wsl: bool },
    Mac,
    Windows,
}
impl Desktop {
    fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Linux {
                wsl: std::env::var_os("WSL_INTEROP").is_some()
                    || std::env::var_os("WSL_DISTRO_NAME").is_some(),
            }
        }
    }
}
struct Helper {
    program: &'static str,
    args: Vec<String>,
    input: bool,
}
fn helper(program: &'static str, args: &[&str], input: bool) -> Helper {
    Helper {
        program,
        args: args.iter().map(|arg| (*arg).into()).collect(),
        input,
    }
}
fn powershell(copy: bool) -> Helper {
    // The URL travels only on stdin; PowerShell never parses it as source code.
    let script = if copy {
        "[Console]::InputEncoding=[System.Text.UTF8Encoding]::new(); Set-Clipboard -Value ([Console]::In.ReadToEnd())"
    } else {
        "[Console]::InputEncoding=[System.Text.UTF8Encoding]::new(); Start-Process -FilePath ([Console]::In.ReadToEnd())"
    };
    helper(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ],
        true,
    )
}
fn helpers(desktop: Desktop, copy: bool, url: &str) -> Vec<Helper> {
    match (desktop, copy) {
        (Desktop::Windows, _) => vec![powershell(copy)],
        (Desktop::Mac, false) => vec![helper("open", &[url], false)],
        (Desktop::Mac, true) => vec![helper("pbcopy", &[], true)],
        (Desktop::Linux { wsl }, false) => {
            let mut commands = vec![];
            if wsl {
                commands.push(helper("wslview", &[url], false));
                commands.push(powershell(false));
            }
            commands.push(helper("xdg-open", &[url], false));
            commands.push(helper("gio", &["open", url], false));
            commands
        }
        (Desktop::Linux { wsl }, true) => {
            let mut commands = vec![];
            if wsl {
                commands.push(powershell(true));
            }
            commands.push(helper(
                "wl-copy",
                &["--type", "text/plain;charset=utf-8"],
                true,
            ));
            commands.push(helper("xclip", &["-selection", "clipboard", "-in"], true));
            commands.push(helper("xsel", &["--clipboard", "--input"], true));
            commands
        }
    }
}
async fn run(helper: Helper, url: &str, copy: bool, cancel: &CancellationToken) -> bool {
    let mut command = Command::new(helper.program);
    command
        .args(helper.args)
        .stdin(if helper.input {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(copy);
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let work = async {
        if let Some(mut input) = child.stdin.take() {
            input.write_all(url.as_bytes()).await.ok()?;
            input.shutdown().await.ok()?;
        }
        child.wait().await.ok()
    };
    let outcome = tokio::select! {
        biased;
        _=cancel.cancelled()=>None,
        result=tokio::time::timeout(Duration::from_secs(4), work)=>Some(result),
    };
    match outcome {
        Some(Ok(Some(status))) => status.success(),
        Some(Err(_)) if !copy => {
            // A desktop launcher may remain alive for the browser lifetime.
            // Reap it without blocking the TUI or killing the user's browser.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            true
        }
        _ => {
            let _ = child.kill().await;
            false
        }
    }
}
pub async fn perform(url: &str, copy: bool, cancel: &CancellationToken) -> LinkResult {
    if !valid_login_url(url) {
        return LinkResult::Unavailable;
    }
    for helper in helpers(Desktop::current(), copy, url) {
        if cancel.is_cancelled() {
            return LinkResult::Cancelled;
        }
        if run(helper, url, copy, cancel).await {
            return if copy {
                LinkResult::Copied
            } else {
                LinkResult::Opened
            };
        }
    }
    if cancel.is_cancelled() {
        LinkResult::Cancelled
    } else if copy {
        LinkResult::TerminalCopy
    } else {
        LinkResult::Unavailable
    }
}

/// OSC 52 is a best-effort request, not confirmation of clipboard contents.
pub fn terminal_copy(url: &str, output: &mut impl std::io::Write) -> std::io::Result<()> {
    use base64::Engine;
    if !valid_login_url(url) {
        return Err(std::io::Error::other("invalid login link"));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(url);
    write!(output, "\x1b]52;c;{encoded}\x07")?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    const URL: &str = "https://auth.openai.com/api/accounts/authorize?state=fixture&nonce=a%26b";
    #[test]
    fn login_urls_are_restricted_and_clipboard_preserves_every_byte() {
        assert!(valid_login_url(URL));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://auth.openai.com.evil/api/accounts/authorize?a=b",
            "https://user@auth.openai.com/api/accounts/authorize?a=b",
            "https://auth.openai.com:444/api/accounts/authorize?a=b",
            "https://auth.openai.com/api/accounts/authorize?a=b#fragment",
            "https://auth.openai.com/api/accounts/authorize?a=\nvalue",
        ] {
            assert!(!valid_login_url(bad));
        }
        let mut bytes = vec![];
        terminal_copy(URL, &mut bytes).unwrap();
        let sequence = String::from_utf8(bytes).unwrap();
        use base64::Engine;
        let data = sequence
            .strip_prefix("\x1b]52;c;")
            .unwrap()
            .strip_suffix('\x07')
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap(),
            URL.as_bytes()
        );
    }
    #[test]
    fn desktop_helpers_pass_urls_as_data_including_wsl() {
        for desktop in [
            Desktop::Linux { wsl: false },
            Desktop::Linux { wsl: true },
            Desktop::Mac,
            Desktop::Windows,
        ] {
            for copy in [false, true] {
                let commands = helpers(desktop, copy, URL);
                assert!(!commands.is_empty());
                for command in commands {
                    if command.input {
                        assert!(!command.args.iter().any(|arg| arg.contains(URL)));
                    } else {
                        assert_eq!(command.args.iter().filter(|arg| *arg == URL).count(), 1);
                    }
                    assert!(!matches!(command.program, "sh" | "bash" | "cmd.exe"));
                }
            }
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn helpers_receive_exact_stdin_and_failure_is_not_reported_as_success() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard helper");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nIFS= read -r data || :\n[ \"$data\" = '{}' ]\n",
                URL
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        // The fake helper is trusted test source, not an authorization URL.
        let executable: &'static str =
            Box::leak(path.to_string_lossy().into_owned().into_boxed_str());
        assert!(
            run(
                helper(executable, &[], true),
                URL,
                true,
                &CancellationToken::new()
            )
            .await
        );
        assert!(
            !run(
                helper(executable, &[], true),
                "different",
                true,
                &CancellationToken::new()
            )
            .await
        );
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(perform(URL, false, &token).await, LinkResult::Cancelled);
    }
}
