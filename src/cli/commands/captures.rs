use anyhow::{Context, Result};

use crate::config::{load_config, Config};

/// Print recent structured captures (A1/A2/A4/A5/handoffs) from the ledger.
/// Agents use this for the morning brief and weekly Hindsight sync.
pub async fn run(kind: Option<String>, hours: u32, limit: usize) -> Result<()> {
    let config = load_config()?;
    let since = (chrono::Utc::now() - chrono::Duration::hours(hours as i64)).to_rfc3339();
    let rows = fetch_captures(&config, kind.as_deref(), &since, limit).await?;
    if rows.is_empty() {
        println!("No captures in the last {hours}h.");
        return Ok(());
    }
    for row in rows {
        let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let project = row.get("project").and_then(|v| v.as_str()).unwrap_or("");
        let captured = row
            .get("captured_utc")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let payload = row
            .get("payload_json")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        println!("## {kind} · {project} · {captured}\n{payload}\n");
    }
    Ok(())
}

async fn fetch_captures(
    config: &Config,
    kind: Option<&str>,
    since: &str,
    limit: usize,
) -> Result<Vec<serde_json::Value>> {
    let url = config.push.url.clone().context(
        "push.url is not configured — `daybook serve` on some machine, then set push.url",
    )?;
    let token = config
        .push
        .token
        .clone()
        .context("push.token is not configured — mint one with `daybook token`")?;
    let mut req = crate::push::client()
        .get(format!("{}/api/captures", url.trim_end_matches('/')))
        .bearer_auth(token)
        .query(&[("since", since), ("limit", &limit.to_string())]);
    if let Some(kind) = kind {
        req = req.query(&[("kind", kind)]);
    }
    let resp = req.send().await.context("GET /api/captures")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.context("parse /api/captures response")?;
    if !status.is_success() {
        anyhow::bail!("captures returned {status}: {body}");
    }
    Ok(body
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default())
}
