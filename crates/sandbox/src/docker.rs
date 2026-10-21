use crate::{ProcessError, supervise_child};
use std::{future::Future, path::Path, pin::Pin, process::Stdio, sync::Arc, time::Duration};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_SANDBOX_IMAGE: &str = "rust:1.99.1-slim-bookworm";
pub type DockerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<DockerOutput, SandboxError>> + Send + 'a>>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    #[error("DOCKER_UNAVAILABLE_NO_HOST_FALLBACK")]
    Unavailable,
    #[error("INVALID_SANDBOX_CONFIGURATION")]
    Configuration,
    #[error("SANDBOX_CANCELLED")]
    Cancelled,
    #[error("SANDBOX_TIMEOUT")]
    Timeout,
    #[error("SANDBOX_OUTPUT_LIMIT")]
    Limit,
    #[error("DOCKER_FAILED")]
    Docker,
    #[error("CONTAINER_CLEANUP_FAILED")]
    Cleanup,
}
#[derive(Clone, Debug)]
pub struct DockerOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}
/// Porta confiável de infraestrutura; recebe exclusivamente argumentos da CLI Docker.
pub trait DockerDriver: Send + Sync {
    fn invoke(
        &self,
        args: Vec<String>,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> DockerFuture<'_>;
}
pub struct NativeDocker;
impl DockerDriver for NativeDocker {
    fn invoke(
        &self,
        args: Vec<String>,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> DockerFuture<'_> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(SandboxError::Cancelled);
            }
            let mut cmd = Command::new("docker");
            cmd.args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            // O ambiente serve somente à CLI Docker; não há -e/--env-file no container.
            #[cfg(windows)]
            cmd.creation_flags(0x0800_0000);
            let child = cmd.spawn().map_err(|_| SandboxError::Unavailable)?;
            let token = cancel.child_token();
            let deadline_token = token.clone();
            let timer = tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                deadline_token.cancel();
            });
            let result = supervise_child(child, token, 80_000).await;
            timer.abort();
            let result = result.map_err(|e| match e {
                ProcessError::Cancelled if cancel.is_cancelled() => SandboxError::Cancelled,
                ProcessError::Cancelled => SandboxError::Timeout,
                ProcessError::Limit => SandboxError::Limit,
                _ => SandboxError::Docker,
            })?;
            Ok(DockerOutput {
                stdout: String::from_utf8_lossy(&result.stdout).into(),
                stderr: String::from_utf8_lossy(&result.stderr).into(),
                exit_code: result.status.code().unwrap_or(1),
            })
        })
    }
}
#[derive(Clone, Debug)]
pub struct DockerConfig {
    pub image: String,
    pub tmpfs_mib: u32,
    pub dependencies: bool,
}
impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            image: DEFAULT_SANDBOX_IMAGE.into(),
            tmpfs_mib: 64,
            dependencies: false,
        }
    }
}
impl DockerConfig {
    pub fn from_lookup(
        mut lookup: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, SandboxError> {
        let mut config = Self::default();
        if let Some(image) = lookup("MYAGENT_SANDBOX_IMAGE") {
            config.image = image;
        }
        if let Some(value) = lookup("MYAGENT_SANDBOX_TMPFS_MIB") {
            if !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(SandboxError::Configuration);
            }
            config.tmpfs_mib = value.parse().map_err(|_| SandboxError::Configuration)?;
        }
        if let Some(value) = lookup("MYAGENT_SANDBOX_DEPENDENCIES") {
            config.dependencies = match value.as_str() {
                "0" => false,
                "1" => true,
                _ => return Err(SandboxError::Configuration),
            };
        }
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), SandboxError> {
        if self.image.is_empty()
            || self.image.len() > 512
            || self.image.starts_with('-')
            || !self
                .image
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._:@-".contains(&b))
            || !(1..=4096).contains(&self.tmpfs_mib)
        {
            return Err(SandboxError::Configuration);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Docker {
    driver: Arc<dyn DockerDriver>,
    config: DockerConfig,
}
impl Docker {
    pub async fn detect(
        config: DockerConfig,
        driver: Arc<dyn DockerDriver>,
        cancel: CancellationToken,
    ) -> Result<Option<Self>, SandboxError> {
        config.validate()?;
        let check = driver.invoke(
            vec!["info".into(), "--format".into(), "{{.OSType}}".into()],
            Duration::from_secs(3),
            cancel.clone(),
        );
        let result = tokio::select! { biased;
            _=cancel.cancelled()=>return Err(SandboxError::Cancelled),
            result=tokio::time::timeout(Duration::from_secs(3),check)=>result,
        };
        if cancel.is_cancelled() {
            return Err(SandboxError::Cancelled);
        }
        Ok(match result {
            Ok(Ok(output)) if output.exit_code == 0 && output.stdout.trim() == "linux" => {
                Some(Self { driver, config })
            }
            _ => None,
        })
    }
    pub fn config(&self) -> &DockerConfig {
        &self.config
    }
    pub fn arguments(
        &self,
        snapshot: &Path,
        name: &str,
        script: &str,
    ) -> Result<Vec<String>, SandboxError> {
        self.config.validate()?;
        let snapshot = snapshot.to_str().ok_or(SandboxError::Configuration)?;
        let snapshot = snapshot.strip_prefix("\\\\?\\").unwrap_or(snapshot);
        if snapshot.contains([',', '\n', '\r'])
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(SandboxError::Configuration);
        }
        let values = vec![
            "run".into(),
            "--pull=never".into(),
            "--name".into(),
            name.into(),
            "--init".into(),
            "--network=none".into(),
            "--read-only".into(),
            "--cap-drop=ALL".into(),
            "--security-opt=no-new-privileges".into(),
            "--pids-limit=128".into(),
            "--memory=512m".into(),
            "--memory-swap=512m".into(),
            "--cpus=1".into(),
            "--no-healthcheck".into(),
            "--user=65534:65534".into(),
            "--tmpfs".into(),
            format!(
                "/tmp:rw,nosuid,nodev,size={}m,mode=1777",
                self.config.tmpfs_mib
            ),
            "--mount".into(),
            format!(
                "type=bind,source={snapshot},target=/workspace,readonly,bind-recursive=disabled"
            ),
            "--workdir=/workspace".into(),
            "--entrypoint=/bin/sh".into(),
            self.config.image.clone(),
            "-c".into(),
            script.into(),
        ];
        Ok(values)
    }
    /// O chamador mantém o snapshot vivo até a remoção confirmada do container.
    pub async fn run(
        &self,
        snapshot: &Path,
        script: &str,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<DockerOutput, SandboxError> {
        if cancel.is_cancelled() {
            return Err(SandboxError::Cancelled);
        }
        let present = self
            .driver
            .invoke(
                vec!["image".into(), "inspect".into(), self.config.image.clone()],
                Duration::from_secs(3),
                cancel.clone(),
            )
            .await?;
        if present.exit_code != 0 {
            let pull = self
                .driver
                .invoke(
                    vec!["pull".into(), self.config.image.clone()],
                    Duration::from_secs(180),
                    cancel.clone(),
                )
                .await?;
            if pull.exit_code != 0 {
                return Err(SandboxError::Docker);
            }
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| SandboxError::Configuration)?
            .as_nanos();
        let name = format!(
            "myagent-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let result = self
            .driver
            .invoke(self.arguments(snapshot, &name, script)?, timeout, cancel)
            .await;
        let cleanup = self
            .driver
            .invoke(
                vec!["rm".into(), "--force".into(), name],
                Duration::from_secs(10),
                CancellationToken::new(),
            )
            .await;
        // Remoção explícita antes de liberar o snapshot; falhas não são ignoradas.
        if !matches!(cleanup,Ok(ref o) if o.exit_code==0) {
            // Não liberar o snapshot de um container possivelmente ainda ativo.
            return Err(SandboxError::Cleanup);
        }
        result
    }
}
