use anyhow::{Context, Result};
use dialoguer::{theme::ColorfulTheme, Confirm, FuzzySelect, Input, Select};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::archive::ArchiveManager;
use crate::config::{get_config_path, load_config, save_config, Config};

/// Initialize the daily archive system
pub async fn run(
    storage_path: Option<PathBuf>,
    interactive: bool,
    use_haiku: bool,
    configure_board: bool,
) -> Result<()> {
    println!("[daybook] Initializing Daily Context Archive System...");

    // Load or create config
    let mut config = load_config().unwrap_or_else(|_| Config::default());

    // Set model based on flag
    if use_haiku {
        config.summarization.model = "haiku".into();
        println!("[daybook] Using haiku model for summarization");
    } else {
        println!("[daybook] Using sonnet model for summarization (default)");
    }

    // Determine storage path based on options
    let final_path = if let Some(path) = storage_path {
        // Direct path specified
        Some(expand_path(&path))
    } else if interactive && !configure_board {
        // Interactive mode with fuzzy search
        select_directory_interactive()?
    } else {
        // Use default
        None
    };

    // Update storage path if determined
    if let Some(path) = final_path {
        config.storage.path = path;
    }

    // Configure language and digest settings in interactive mode
    if interactive && !configure_board {
        configure_language_interactive(&mut config)?;
        configure_digest_interactive(&mut config)?;
    }

    // Save config
    save_config(&config)?;
    let config_path = get_config_path()?;
    println!(
        "[daybook] Configuration saved to: {}",
        config_path.display()
    );

    // Create storage directory
    let manager = ArchiveManager::new(config.clone());
    let storage_dir = manager.ensure_storage_dir()?;
    println!("[daybook] Storage directory: {}", storage_dir.display());

    // Create today's directory
    let today_dir = manager.ensure_today_dir()?;
    println!("[daybook] Today's archive: {}", today_dir.display());

    let home = dirs::home_dir().context("Could not determine home directory")?;
    install_recall_skills(&home)?;

    if configure_board {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        configure_board_mcp_and_hooks(&cwd, &home)?;
    }

    println!();
    println!("[daybook] Initialization complete!");

    Ok(())
}

const RECALL_SKILL: &str = include_str!("../../../skills/daybook-recall/SKILL.md");
const PI_EXTENSION: &str = include_str!("../../../hooks/daybook-pi.ts");
pub(crate) const CLAUDE_EDIT_MATCHER: &str =
    "Edit|Write|MultiEdit|NotebookEdit|replace|replace_match|insert";

pub(crate) fn stage_write(path: &Path, content: &str) -> Result<PathBuf> {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        anyhow::bail!("Refusing to write through symlink {}", path.display());
    }
    let parent = path.parent().context("Destination has no parent")?;
    fs::create_dir_all(parent)?;
    if fs::symlink_metadata(parent).is_ok_and(|meta| meta.file_type().is_symlink()) {
        anyhow::bail!("Refusing to write inside symlink {}", parent.display());
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp = parent.join(format!(".daybook-{}-{nonce}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(&temp)?;
    if let Ok(meta) = fs::metadata(path) {
        fs::set_permissions(&temp, meta.permissions())?;
    }
    file.write_all(content.as_bytes())?;
    Ok(temp)
}

pub(crate) fn finish_write(temp: &Path, path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        let _ = fs::remove_file(temp);
        anyhow::bail!("Refusing to write through symlink {}", path.display());
    }
    fs::rename(temp, path)?;
    Ok(())
}

pub(crate) fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let temp = stage_write(path, content)?;
    if let Err(error) = finish_write(&temp, path) {
        let _ = fs::remove_file(temp);
        return Err(error);
    }
    Ok(())
}

fn install_recall_skills(home: &Path) -> Result<()> {
    for root in [home.join(".agents/skills"), home.join(".claude/skills")] {
        install_runtime_file(
            &root.join("daybook-recall/SKILL.md"),
            RECALL_SKILL,
            "recall skill",
        )?;
    }
    Ok(())
}

fn install_runtime_file(path: &Path, expected: &str, label: &str) -> Result<()> {
    match fs::read_to_string(path) {
        Ok(actual) if actual == expected => {
            println!("[daybook] Current {label}: {}", path.display());
            return Ok(());
        }
        Ok(_) => {
            println!(
                "[daybook] Leaving edited or outdated {label} unchanged: {}",
                path.display()
            );
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(path, expected)?;
    println!("[daybook] Installed {label}: {}", path.display());
    Ok(())
}

fn configure_board_mcp_and_hooks(cwd: &Path, home: &Path) -> Result<()> {
    let mcp_path = cwd.join(".mcp.json");
    let mut root: serde_json::Value = if mcp_path.exists() {
        let raw = fs::read_to_string(&mcp_path)?;
        serde_json::from_str(&raw).context("Cannot update invalid .mcp.json")?
    } else {
        serde_json::json!({})
    };
    let root_object = root
        .as_object_mut()
        .context(".mcp.json root must be an object")?;
    let servers = root_object
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let map = servers
        .as_object_mut()
        .context(".mcp.json mcpServers must be an object")?;
    map.insert(
        "daybook-board".to_string(),
        serde_json::json!({"command": "daybook", "args": ["board", "mcp"]}),
    );
    atomic_write(&mcp_path, &(serde_json::to_string_pretty(&root)? + "\n"))?;
    println!(
        "[daybook] Registered daybook-board MCP server in {}",
        mcp_path.display()
    );

    let settings_path = home.join(".claude/settings.json");
    let mut settings: serde_json::Value = if settings_path.exists() {
        serde_json::from_str(&fs::read_to_string(&settings_path)?)
            .context("Cannot update invalid Claude settings.json")?
    } else {
        serde_json::json!({})
    };
    let settings_object = settings
        .as_object_mut()
        .context("Claude settings.json root must be an object")?;
    let hooks = settings_object
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let hooks_object = hooks
        .as_object_mut()
        .context("Claude settings hooks must be an object")?;
    for (event, hook_name, matcher) in [
        ("SessionStart", "session-start", None),
        ("PreToolUse", "pre-tool-use", Some(CLAUDE_EDIT_MATCHER)),
        ("PostToolUse", "post-tool-use", Some(CLAUDE_EDIT_MATCHER)),
        ("SessionEnd", "session-end", None),
    ] {
        let command = format!("daybook hook {hook_name}");
        let groups = hooks_object
            .entry(event)
            .or_insert_with(|| serde_json::json!([]));
        let groups = groups
            .as_array_mut()
            .with_context(|| format!("Claude {event} hooks must be an array"))?;
        let exists = groups.iter_mut().any(|group| {
            let matches = group
                .get("hooks")
                .and_then(|v| v.as_array())
                .is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry.get("command").and_then(|v| v.as_str()) == Some(&command)
                    })
                });
            if matches {
                if let Some(matcher) = matcher {
                    group["matcher"] = serde_json::Value::String(matcher.into());
                } else if let Some(object) = group.as_object_mut() {
                    object.remove("matcher");
                }
            }
            matches
        });
        if !exists {
            let mut group = serde_json::json!({
                "hooks": [{"type": "command", "command": command}]
            });
            if let Some(matcher) = matcher {
                group["matcher"] = serde_json::Value::String(matcher.into());
            }
            groups.push(group);
        }
    }
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(
        &settings_path,
        &(serde_json::to_string_pretty(&settings)? + "\n"),
    )?;
    println!(
        "[daybook] Installed Claude hooks: {}",
        settings_path.display()
    );

    let extension_path = home.join(".pi/agent/extensions/daybook-pi.ts");
    install_runtime_file(&extension_path, PI_EXTENSION, "Pi extension")?;
    Ok(())
}

/// Expand ~ in path
fn expand_path(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str.starts_with("~") {
        if let Some(home) = dirs::home_dir() {
            return home.join(path_str.trim_start_matches("~/").trim_start_matches("~"));
        }
    }
    path.to_path_buf()
}

/// Interactive directory selection with keyword search
fn select_directory_interactive() -> Result<Option<PathBuf>> {
    let theme = ColorfulTheme::default();

    // Ask user for storage path with default
    let input: String = dialoguer::Input::with_theme(&theme)
        .with_prompt("Daily archive folder (e.g., '~/obsidian-vault/daily', '~/.daybook')")
        .default("~/.daybook".into())
        .interact_text()
        .context("Failed to read input")?;

    let input = input.trim();

    // If user entered the default or empty, use default path
    if input.is_empty() || input == "~/.daybook" {
        return Ok(Some(expand_path(Path::new("~/.daybook"))));
    }

    // Check if input looks like a path (contains / or ~)
    if input.contains('/') || input.starts_with('~') {
        // User entered a direct path
        return Ok(Some(expand_path(Path::new(input))));
    }

    // Otherwise treat as keyword search
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let candidates = search_directories_recursive(&home, &input.to_lowercase(), 10);

    if candidates.is_empty() {
        println!("[daybook] No matching directories found. Using default path.");
        return Ok(None);
    }

    // Build display items with path info
    let display_items: Vec<String> = candidates
        .iter()
        .map(|p| format_path_display(p.as_path()))
        .collect();

    // Use FuzzySelect for selection (user can type to filter)
    let selection = FuzzySelect::with_theme(&theme)
        .with_prompt("Select archive directory (type to filter)")
        .items(&display_items)
        .default(0)
        .max_length(15)
        .highlight_matches(true)
        .interact_opt()
        .context("Failed to select directory")?;

    match selection {
        Some(idx) => {
            let selected = &candidates[idx];
            // If selected path doesn't end with "daily", append it
            let final_path = if !selected.ends_with("daily") {
                selected.join("daily")
            } else {
                selected.clone()
            };
            Ok(Some(final_path))
        }
        None => {
            println!("[daybook] Selection cancelled. Using default path.");
            Ok(None)
        }
    }
}

/// Recursively search for directories matching the keyword
fn search_directories_recursive(root: &Path, keyword: &str, max_depth: usize) -> Vec<PathBuf> {
    let mut results = Vec::new();
    search_recursive_helper(root, keyword, max_depth, 0, &mut results);

    // Sort: prefer shorter paths and paths containing "daily"
    results.sort_by(|a, b| {
        let a_has_daily = a.to_string_lossy().to_lowercase().contains("daily");
        let b_has_daily = b.to_string_lossy().to_lowercase().contains("daily");
        match (a_has_daily, b_has_daily) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.to_string_lossy().len().cmp(&b.to_string_lossy().len()),
        }
    });

    results.truncate(100);
    results
}

fn search_recursive_helper(
    dir: &Path,
    keyword: &str,
    max_depth: usize,
    current_depth: usize,
    results: &mut Vec<PathBuf>,
) {
    if current_depth > max_depth || results.len() >= 200 {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Skip most hidden directories but allow some special ones
        if name.starts_with('.') && name != ".claude" {
            continue;
        }

        // Skip system directories that are unlikely to contain user data
        if matches!(
            name,
            "node_modules"
                | "target"
                | "build"
                | "dist"
                | ".git"
                | "vendor"
                | "cache"
                | "Cache"
                | "Caches"
        ) {
            continue;
        }

        // Check if directory name matches keyword
        if name.to_lowercase().contains(keyword) && !results.contains(&path) {
            results.push(path.clone());
        }

        // Continue searching deeper
        search_recursive_helper(&path, keyword, max_depth, current_depth + 1, results);
    }
}

/// Format path for display, showing abbreviated home path
fn format_path_display(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path_str.starts_with(home_str.as_ref()) {
            let relative = path_str.replacen(home_str.as_ref(), "~", 1);
            let exists = if path.exists() { "" } else { " (will create)" };
            return format!("{}{}", relative, exists);
        }
    }

    let exists = if path.exists() { "" } else { " (will create)" };
    format!("{}{}", path_str, exists)
}

/// Interactive configuration for language settings
fn configure_language_interactive(config: &mut Config) -> Result<()> {
    let theme = ColorfulTheme::default();

    println!();
    println!("[daybook] Language Configuration");

    let languages = ["English", "中文 (Chinese)"];
    let language_codes = ["en", "zh"];

    let selection = Select::with_theme(&theme)
        .with_prompt("Select summary language")
        .items(&languages)
        .default(0)
        .interact()
        .context("Failed to select language")?;

    config.summarization.summary_language = language_codes[selection].to_string();
    println!(
        "[daybook] Summary language set to: {}",
        languages[selection]
    );

    Ok(())
}

/// Interactive configuration for digest settings
fn configure_digest_interactive(config: &mut Config) -> Result<()> {
    let theme = ColorfulTheme::default();

    println!();
    println!("[daybook] Digest Configuration");
    println!("  Sessions are archived individually. Digest consolidates them into daily.md.");
    println!();

    // Ask about auto-digest
    let auto_digest = Confirm::with_theme(&theme)
        .with_prompt("Enable auto-digest of previous day's sessions?")
        .default(true)
        .interact()
        .unwrap_or(true);

    config.summarization.auto_digest_enabled = auto_digest;

    if auto_digest {
        // Ask for digest time
        let digest_time: String = Input::with_theme(&theme)
            .with_prompt("Digest time (HH:MM format, when to consolidate yesterday's sessions)")
            .default("06:00".into())
            .validate_with(|input: &String| -> std::result::Result<(), &str> {
                let parts: Vec<&str> = input.split(':').collect();
                if parts.len() != 2 {
                    return Err("Format must be HH:MM");
                }
                match (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                    (Ok(h), Ok(m)) if h < 24 && m < 60 => Ok(()),
                    _ => Err("Invalid time (use 00-23 for hour, 00-59 for minute)"),
                }
            })
            .interact_text()
            .context("Failed to read digest time")?;

        config.summarization.digest_time = digest_time;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_file_install_preserves_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skill/SKILL.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "local edits").unwrap();
        install_runtime_file(&path, "shipped version", "skill").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "local edits");
    }

    #[test]
    fn installs_recall_and_board_integrations_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"model":"keep","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"daybook hook session-start"}]}]}}"#,
        )
        .unwrap();

        install_recall_skills(&home).unwrap();
        configure_board_mcp_and_hooks(&project, &home).unwrap();
        configure_board_mcp_and_hooks(&project, &home).unwrap();

        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/daybook-recall/SKILL.md")).unwrap(),
            RECALL_SKILL
        );
        assert_eq!(
            fs::read_to_string(home.join(".pi/agent/extensions/daybook-pi.ts")).unwrap(),
            PI_EXTENSION
        );
        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(home.join(".claude/settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["model"], "keep");
        assert_eq!(
            settings["hooks"]["SessionStart"].as_array().unwrap().len(),
            1
        );
        for event in ["PreToolUse", "PostToolUse", "SessionEnd"] {
            assert_eq!(settings["hooks"][event].as_array().unwrap().len(), 1);
        }
        let mcp: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["daybook-board"]["command"], "daybook");
    }
}
