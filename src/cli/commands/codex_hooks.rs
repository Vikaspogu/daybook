use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

// SessionEnd joined the tool hooks in rust-v0.145.0.
pub const MIN_VERSION: &str = "0.145.0";
const EVENTS: [(&str, &str); 4] = [
    ("SessionStart", "session-start"),
    ("PreToolUse", "pre-tool-use"),
    ("PostToolUse", "post-tool-use"),
    ("SessionEnd", "session-end"),
];

pub fn hooks_path(home: &Path) -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"))
        .join("hooks.json")
}

pub fn configure(home: &Path) -> Result<()> {
    match detected_version().and_then(|version| {
        check_version(&version)?;
        Ok(version)
    }) {
        Ok(version) => {
            let path = hooks_path(home);
            install(&path, &version)?;
            println!("[daybook] Installed Codex hooks: {}", path.display());
            println!("[daybook] Review and trust these hooks in Codex /hooks. Hooks must be enabled; idle Codex sessions are not woken.");
        }
        Err(error) => println!(
            "[daybook] Skipping Codex hooks: {error}; requires Codex CLI >= {}",
            MIN_VERSION
        ),
    }
    Ok(())
}

pub fn detected_version() -> Result<String> {
    let output = std::process::Command::new("codex")
        .arg("--version")
        .output()
        .context("cannot run codex --version")?;
    anyhow::ensure!(output.status.success(), "codex --version failed");
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn check_version(version: &str) -> Result<()> {
    let raw = version.trim().strip_prefix("codex-cli ").unwrap_or("");
    let numbers = raw
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok();
    anyhow::ensure!(
        numbers.as_ref().is_some_and(|v| v.len() == 3),
        "unrecognized Codex version {version:?}; requires Codex CLI >= {MIN_VERSION}"
    );
    let v = numbers.unwrap();
    anyhow::ensure!(
        v >= MIN_VERSION
            .split('.')
            .map(|part| part.parse::<u64>().unwrap())
            .collect::<Vec<_>>(),
        "{version} is unsupported; requires Codex CLI >= {MIN_VERSION}"
    );
    Ok(())
}

fn command(hook: &str) -> String {
    format!("DAYBOOK_RUNTIME=codex daybook hook {hook}")
}

pub fn install(path: &Path, version: &str) -> Result<()> {
    check_version(version)?;
    let mut root: Value = match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).context("Cannot update invalid Codex hooks.json")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.into()),
    };
    let hooks = root
        .as_object_mut()
        .context("Codex hooks.json root must be an object")?
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("Codex hooks must be an object")?;
    for (event, hook) in EVENTS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .with_context(|| format!("Codex {event} hooks must be an array"))?;
        let command = command(hook);
        if !has_hook(groups, hook) {
            groups.push(json!({"hooks": [{"type": "command", "command": command, "timeout": 30}]}));
        }
    }
    super::init::atomic_write(path, &(serde_json::to_string_pretty(&root)? + "\n"))
}

fn has_hook(groups: &[Value], hook: &str) -> bool {
    groups.iter().any(|g| {
        g.get("matcher").is_none()
            && g["hooks"].as_array().is_some_and(|hs| {
                hs.iter().any(|h| {
                    h["type"] == "command"
                        && h["command"] == command(hook)
                        && h.get("async").is_none_or(|v| v == false)
                })
            })
    })
}

pub fn inspect(path: &Path, version: &str) -> Result<()> {
    check_version(version)?;
    let root: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    for (event, hook) in EVENTS {
        anyhow::ensure!(
            root["hooks"][event]
                .as_array()
                .is_some_and(|groups| has_hook(groups, hook)),
            "missing or customized Codex {event} hook; run `daybook init --board`"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_version_gate_fails_closed_before_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hooks.json");
        for v in [
            "codex-cli 0.144.0",
            "codex-cli 0.114.0",
            "",
            "codex-cli unknown",
            "codex-cli 0.145",
            "codex-cli 0.145.0-alpha.1",
            "other 9.0.0",
        ] {
            let error = install(&path, v).unwrap_err().to_string();
            assert!(error.contains(MIN_VERSION), "{error}");
            assert!(!path.exists());
        }
        for v in ["codex-cli 0.145.0", "codex-cli 0.169.0", "codex-cli 1.0.0"] {
            check_version(v).unwrap();
        }
    }

    #[test]
    fn codex_install_preserves_other_hooks_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hooks.json");
        fs::write(&path, r#"{"description":"keep","hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}],"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"other"}]}]}}"#).unwrap();
        install(&path, "codex-cli 0.169.0").unwrap();
        let first = fs::read_to_string(&path).unwrap();
        install(&path, "codex-cli 0.169.0").unwrap();
        assert_eq!(first, fs::read_to_string(&path).unwrap());
        inspect(&path, "codex-cli 0.169.0").unwrap();
        let root: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(root["description"], "keep");
        assert_eq!(root["hooks"]["Stop"][0]["hooks"][0]["command"], "other");
        assert_eq!(root["hooks"]["PostToolUse"].as_array().unwrap().len(), 2);
        assert!(root["hooks"].get("PostToolUseFailure").is_none());
    }

    #[test]
    fn codex_install_rejects_invalid_config_without_overwriting() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("hooks.json");
        for raw in [
            "invalid",
            "[]",
            r#"{"hooks":[]}"#,
            r#"{"hooks":{"PostToolUse":{}}}"#,
        ] {
            fs::write(&path, raw).unwrap();
            assert!(install(&path, "codex-cli 0.169.0").is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), raw);
        }
    }
}
