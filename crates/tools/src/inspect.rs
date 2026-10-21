use crate::{
    Error, Result, SafeWorkspace,
    io::{Budget, cut, read_text, scan},
    registry::{limit_output, text},
};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}
fn is_test(p: &str) -> bool {
    let n = basename(p).to_ascii_lowercase();
    let p = p.to_ascii_lowercase();
    (n.starts_with("test_") && n.ends_with(".py"))
        || n.ends_with("_test.py")
        || n.ends_with("_test.go")
        || (crate::source::source_file(p.as_str())
            && (n.contains(".test.")
                || n.contains(".spec.")
                || p.split('/')
                    .any(|s| ["__tests__", "test", "tests"].contains(&s))))
}
fn package(ws: &SafeWorkspace, path: &str, b: &Budget) -> Result<Value> {
    let (raw, _) = read_text(ws, path, 262_144, b)?;
    let pkg: Value = serde_json::from_str(&raw).map_err(|_| Error::Package)?;
    let pkg = pkg.as_object().ok_or(Error::Package)?;
    let mut truncated = false;
    let mut keys = |key: &str| -> Vec<String> {
        let Some(o) = pkg.get(key).and_then(Value::as_object) else {
            return vec![];
        };
        truncated |= o.len() > 100 || o.keys().any(|s| crate::io::units(s) > 200);
        o.keys().take(100).map(|s| cut(s, 200)).collect()
    };
    let scripts = keys("scripts");
    let mut deps = keys("dependencies");
    deps.extend(keys("devDependencies"));
    let mut seen = HashSet::new();
    deps.retain(|v| seen.insert(v.clone()));
    truncated |= deps.len() > 100;
    deps.truncate(100);
    Ok(json!({"path":path,"scripts":scripts,"dependencies":deps,"truncated":truncated}))
}
fn config(name: &str) -> bool {
    (name.starts_with("tsconfig") && name.ends_with(".json"))
        || [
            "bun.config.",
            "vite.config.",
            "vitest.config.",
            "jest.config.",
            "eslint.config.",
        ]
        .iter()
        .any(|p| name.starts_with(p))
        || ["pyproject.toml", "Cargo.toml", "go.mod", "Dockerfile"].contains(&name)
}
pub(crate) fn execute(
    ws: &SafeWorkspace,
    name: &str,
    a: &Map<String, Value>,
    b: &Budget,
) -> Result<Value> {
    let root = ws.resolve_safe_path(text(a, "path", "."), true)?;
    let scan = scan(ws, &root, b, true, |_, _| false)?;
    let manifests: Vec<_> = scan
        .files
        .iter()
        .filter(|p| basename(p) == "package.json")
        .collect();
    let mut packages = vec![];
    let mut errors = vec![];
    let mut truncated = scan.truncated || manifests.len() > 20;
    for manifest in manifests.into_iter().take(20) {
        b.check()?;
        match package(ws, manifest, b) {
            Ok(mut value) => {
                truncated |= value["truncated"] == true;
                if name == "test_discover" {
                    let scripts: Vec<_> = value["scripts"]
                        .as_array()
                        .ok_or(Error::Internal)?
                        .iter()
                        .filter(|s| {
                            s.as_str()
                                .is_some_and(|s| s == "test" || s.starts_with("test:"))
                        })
                        .cloned()
                        .collect();
                    let runners: Vec<_> = value["dependencies"]
                        .as_array()
                        .ok_or(Error::Internal)?
                        .iter()
                        .filter(|s| {
                            s.as_str().is_some_and(|s| {
                                ["vitest", "jest", "@jest/core", "bun-types"].contains(&s)
                            })
                        })
                        .cloned()
                        .collect();
                    value = json!({"path":manifest,"scripts":scripts,"runners":runners,"truncated":value["truncated"]});
                }
                packages.push(value);
            }
            Err(e) => {
                b.check()?;
                errors.push(json!({"path":manifest,"error":e.to_string()}));
            }
        }
    }
    let tests: Vec<_> = scan.files.iter().filter(|p| is_test(p)).collect();
    if name == "test_discover" {
        let configs: Vec<_> = scan
            .files
            .iter()
            .filter(|p| {
                let n = basename(p);
                n.starts_with("vitest.config.")
                    || n.starts_with("jest.config.")
                    || [
                        "pytest.ini",
                        "pyproject.toml",
                        "bun.lock",
                        "bun.lockb",
                        "go.mod",
                    ]
                    .contains(&n)
            })
            .collect();
        truncated |= tests.len() > 100 || configs.len() > 30;
        return limit_output(
            json!({"tests":tests.into_iter().take(100).collect::<Vec<_>>(),"packages":packages,"errors":errors,"configurationFiles":configs.into_iter().take(30).collect::<Vec<_>>(),"discovery":"static-conventions","truncated":truncated}),
        );
    }
    let configs: Vec<_> = scan.files.iter().filter(|p| config(basename(p))).collect();
    let locks: Vec<_> = scan
        .files
        .iter()
        .filter(|p| {
            [
                "bun.lock",
                "bun.lockb",
                "package-lock.json",
                "yarn.lock",
                "pnpm-lock.yaml",
                "Cargo.lock",
                "uv.lock",
            ]
            .contains(&basename(p))
        })
        .collect();
    truncated |=
        configs.len() > 50 || locks.len() > 30 || tests.len() > 50 || scan.files.len() > 100;
    limit_output(
        json!({"packages":packages,"errors":errors,"configurationFiles":configs.into_iter().take(50).collect::<Vec<_>>(),"lockfiles":locks.into_iter().take(30).collect::<Vec<_>>(),"testFiles":tests.into_iter().take(50).collect::<Vec<_>>(),"files":scan.files.into_iter().take(100).collect::<Vec<_>>(),"truncated":truncated}),
    )
}
