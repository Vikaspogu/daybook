use anyhow::Result;
use std::path::PathBuf;

use crate::board::client::BoardClient;
use crate::board::globs::normalize_repo_path;
use crate::board::identity::resolve_board_identity;
use crate::cli::args::{BoardAction, BoardArgs, BoardTaskAction};
use crate::config::load_config;
use crate::ledger::{
    BoardClaimLocksReq, BoardClaimTaskReq, BoardHeartbeatReq, BoardMessagesQuery,
    BoardPostMessageReq, BoardReleaseLocksReq, BoardTouchLockReq, BoardUpdateTaskReq,
};

pub async fn run(args: BoardArgs) -> Result<()> {
    let config = load_config()?;
    let client = BoardClient::from_config(&config, args.local)?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let identity = resolve_board_identity(&cwd, args.project.as_deref(), args.agent.as_deref());

    let action = args
        .action
        .unwrap_or(BoardAction::Status { since_id: None });

    match action {
        BoardAction::Status { since_id } => {
            let snap = client
                .status(Some(&identity.project), Some(&identity.agent), since_id)
                .await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&snap)?);
                return Ok(());
            }
            println!(
                "=== Daybook Board: {} (you: {} @ {}) ===",
                identity.project, identity.agent, identity.branch
            );
            println!("\nActive Agents ({}):", snap.agents.len());
            if snap.agents.is_empty() {
                println!("  (none)");
            } else {
                for a in &snap.agents {
                    let task = a
                        .current_task
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|t| format!(" [task: {t}]"))
                        .unwrap_or_default();
                    println!(
                        "  - {} ({}, branch: {}, status: {}){}",
                        a.agent,
                        a.runtime,
                        if a.branch.is_empty() { "-" } else { &a.branch },
                        a.status,
                        task
                    );
                }
            }

            println!("\nActive File Locks ({}):", snap.locks.len());
            if snap.locks.is_empty() {
                println!("  (none)");
            } else {
                for l in &snap.locks {
                    let kind = if l.auto { "auto" } else { "manual" };
                    let note = if l.note.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", l.note)
                    };
                    println!(
                        "  - [{}] {} -> {} ({}){}",
                        l.id,
                        l.agent,
                        l.paths.join(", "),
                        kind,
                        note
                    );
                }
            }

            println!("\nTasks ({}):", snap.tasks.len());
            if snap.tasks.is_empty() {
                println!("  (none)");
            } else {
                for t in &snap.tasks {
                    let owner = t.agent.as_deref().unwrap_or("unassigned");
                    println!(
                        "  - [{}] {} ({}, priority: {}, owner: {})",
                        t.task_id, t.title, t.status, t.priority, owner
                    );
                }
            }

            if !snap.unread_mentions.is_empty() {
                println!(
                    "\nUnread @mentions for {} ({}):",
                    identity.agent,
                    snap.unread_mentions.len()
                );
                for m in &snap.unread_mentions {
                    println!("  - #{} [{}] @{}: {}", m.id, m.channel, m.agent, m.body);
                }
            }

            println!("\nRecent Messages ({}):", snap.messages.len());
            if snap.messages.is_empty() {
                println!("  (none)");
            } else {
                for m in snap.messages.iter().rev().take(10).rev() {
                    println!("  - #{} [{}] @{}: {}", m.id, m.channel, m.agent, m.body);
                }
            }
        }
        BoardAction::Claim { paths, note, ttl } => {
            let norm_paths: Vec<String> = paths
                .iter()
                .map(|p| {
                    normalize_repo_path(p, Some(&identity.worktree_root))
                        .unwrap_or_else(|_| p.clone())
                })
                .collect();
            let res = client
                .claim_locks(&BoardClaimLocksReq {
                    project: identity.project.clone(),
                    agent: identity.agent.clone(),
                    paths: norm_paths,
                    note: note.unwrap_or_default(),
                    ttl_minutes: Some(ttl),
                })
                .await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&res)?);
                return Ok(());
            }
            println!(
                "[daybook board] Lock {} claimed by {} for: {}",
                res.lock.id,
                res.lock.agent,
                res.lock.paths.join(", ")
            );
            if !res.conflicts.is_empty() {
                eprintln!(
                    "[daybook board] WARNING: overlaps with {} existing lock(s):",
                    res.conflicts.len()
                );
                for c in &res.conflicts {
                    eprintln!(
                        "  - [{}] held by {} on {} ({})",
                        c.id,
                        c.agent,
                        c.paths.join(", "),
                        c.note
                    );
                }
            }
        }
        BoardAction::Touch { path, ttl } => {
            let norm = normalize_repo_path(&path, Some(&identity.worktree_root))?;
            let res = client
                .touch_lock(&BoardTouchLockReq {
                    project: identity.project.clone(),
                    agent: identity.agent.clone(),
                    path: norm,
                    ttl_minutes: Some(ttl),
                    since_id: None,
                })
                .await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&res)?);
                return Ok(());
            }
            println!(
                "[daybook board] Auto-lock {} refreshed for {}",
                res.lock.id,
                res.lock.paths.join(", ")
            );
            for m in &res.unread_mentions {
                println!(
                    "[daybook board] Mention #{} [{}] from @{}: {}",
                    m.id, m.channel, m.agent, m.body
                );
            }
        }
        BoardAction::Check { path, strict } => {
            let norm = normalize_repo_path(&path, Some(&identity.worktree_root))?;
            let res = client
                .check_path(&identity.project, &norm, Some(&identity.agent))
                .await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else if res.blocked {
                eprintln!("[daybook board] BLOCKED: '{}' is locked:", norm);
                for l in &res.blocking_locks {
                    eprintln!(
                        "  - [{}] held by {} on {} ({})",
                        l.id,
                        l.agent,
                        l.paths.join(", "),
                        l.note
                    );
                }
            } else {
                println!("[daybook board] OK: '{}' is unlocked", norm);
            }
            if strict && res.blocked {
                std::process::exit(2);
            }
        }
        BoardAction::Release {
            id,
            all,
            auto_only,
            force,
        } => {
            if id.is_none() && !all && !auto_only {
                anyhow::bail!("specify --id <LOCK_ID>, --all, or --auto-only to release locks");
            }
            let release_all = all;
            let released = client
                .release_locks(&BoardReleaseLocksReq {
                    project: identity.project.clone(),
                    agent: identity.agent.clone(),
                    lock_id: id,
                    all: release_all,
                    auto_only,
                    force,
                })
                .await?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "released": released }))?
                );
            } else {
                println!("[daybook board] Released {released} lock(s)");
            }
        }
        BoardAction::Post {
            body,
            channel,
            reply_to,
        } => {
            let msg = client
                .post_message(&BoardPostMessageReq {
                    project: identity.project.clone(),
                    channel: Some(channel),
                    agent: identity.agent.clone(),
                    body,
                    parent_id: reply_to,
                })
                .await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&msg)?);
            } else {
                println!(
                    "[daybook board] Posted #{} to [{}] as @{}",
                    msg.id, msg.channel, msg.agent
                );
            }
        }
        BoardAction::Messages {
            channel,
            mention,
            since_id,
            limit,
        } => {
            let page = client
                .list_messages(&BoardMessagesQuery {
                    project: identity.project.clone(),
                    channel,
                    mention,
                    since_id,
                    limit: Some(limit),
                })
                .await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&page)?);
            } else if page.messages.is_empty() {
                println!("(no messages)");
            } else {
                for m in &page.messages {
                    println!("#{} [{}] @{}: {}", m.id, m.channel, m.agent, m.body);
                }
            }
        }
        BoardAction::Task { action } => match action {
            BoardTaskAction::Claim {
                id,
                title,
                priority,
                branch,
                note,
                force,
            } => {
                let branch = branch.or_else(|| {
                    if identity.branch.is_empty() {
                        None
                    } else {
                        Some(identity.branch.clone())
                    }
                });
                let res = client
                    .claim_task(&BoardClaimTaskReq {
                        project: identity.project.clone(),
                        task_id: id,
                        title,
                        priority,
                        agent: identity.agent.clone(),
                        branch,
                        note,
                        force,
                    })
                    .await?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&res)?);
                } else if res.ok {
                    println!(
                        "[daybook board] Claimed task [{}] '{}' as {}",
                        res.task.task_id, res.task.title, identity.agent
                    );
                } else {
                    eprintln!(
                        "[daybook board] Task [{}] is already held by {}",
                        res.task.task_id,
                        res.held_by.as_deref().unwrap_or("another agent")
                    );
                }
            }
            BoardTaskAction::Update {
                id,
                status,
                note,
                pr,
                release_locks,
                force,
            } => {
                let rel = release_locks.unwrap_or(status == "done");
                let task = client
                    .update_task(&BoardUpdateTaskReq {
                        project: identity.project.clone(),
                        task_id: id,
                        agent: identity.agent.clone(),
                        status,
                        note,
                        pr_url: pr,
                        release_locks: rel,
                        force,
                    })
                    .await?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&task)?);
                } else {
                    println!(
                        "[daybook board] Updated task [{}] -> status={}",
                        task.task_id, task.status
                    );
                }
            }
            BoardTaskAction::Done {
                id,
                note,
                pr,
                release_locks: _,
            } => {
                let task = client
                    .update_task(&BoardUpdateTaskReq {
                        project: identity.project.clone(),
                        task_id: id,
                        agent: identity.agent.clone(),
                        status: "done".to_string(),
                        note,
                        pr_url: pr,
                        release_locks: true,
                        force: false,
                    })
                    .await?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&task)?);
                } else {
                    println!(
                        "[daybook board] Marked task [{}] done and released locks",
                        task.task_id
                    );
                }
            }
            BoardTaskAction::List => {
                let snap = client
                    .status(Some(&identity.project), Some(&identity.agent), None)
                    .await?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&snap.tasks)?);
                } else if snap.tasks.is_empty() {
                    println!("(no tasks)");
                } else {
                    for t in &snap.tasks {
                        println!(
                            "[{}] {} (status: {}, priority: {}, owner: {})",
                            t.task_id,
                            t.title,
                            t.status,
                            t.priority,
                            t.agent.as_deref().unwrap_or("unassigned")
                        );
                    }
                }
            }
        },
        BoardAction::Heartbeat { status, task } => {
            let row = client
                .heartbeat(&BoardHeartbeatReq {
                    project: identity.project.clone(),
                    agent: identity.agent.clone(),
                    runtime: identity.runtime.clone(),
                    branch: identity.branch.clone(),
                    cwd: identity.worktree_root.to_string_lossy().to_string(),
                    status: Some(status),
                    current_task: task,
                })
                .await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&row)?);
            } else {
                println!(
                    "[daybook board] Heartbeat recorded for {} on {}",
                    row.agent, row.project
                );
            }
        }
        BoardAction::Mcp => {
            crate::board::mcp::run_mcp_stdio(&client, &cwd).await?;
        }
    }

    Ok(())
}
