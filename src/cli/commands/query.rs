use anyhow::{Context, Result};

use crate::config::load_config;

/// Query one day, or recall matching project sessions without knowing their dates.
pub async fn run(
    date: Option<String>,
    project: Option<String>,
    search: Option<String>,
    limit: usize,
) -> Result<()> {
    let config = load_config()?;
    if date.is_some() && (project.is_some() || search.is_some()) {
        anyhow::bail!("a positional date cannot be combined with --project or --search");
    }
    if project.is_some() || search.is_some() {
        let cwd = std::env::current_dir()?;
        let current_project = crate::transcript::adapters::project_for_cwd(&cwd);
        let project = project.or(current_project);
        let rows =
            crate::push::recall(&config, project.as_deref(), search.as_deref(), limit).await?;
        if rows.is_empty() {
            println!("No matching sessions recorded.");
            return Ok(());
        }
        for row in rows {
            println!("## {} · {} · {}", row.project, row.runtime, row.source());
            println!("{}\n", row.summary_md.trim());
        }
        return Ok(());
    }

    let date =
        date.unwrap_or_else(|| crate::ledger::journal_date_at(chrono::Utc::now()).to_string());
    let parsed = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .with_context(|| format!("date must be YYYY-MM-DD, got {date:?}"))?;
    let day = crate::push::fetch_daily(&config, &parsed.to_string()).await?;
    println!("{}\n", day["overview"].as_str().unwrap_or("(no overview)"));
    let raw = day["raw_content"].as_str().unwrap_or("");
    if raw.is_empty() {
        println!("No sessions recorded for this date.");
    } else {
        println!("{raw}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_date_parse_rejects_garbage() {
        assert!(chrono::NaiveDate::parse_from_str("tuesday", "%Y-%m-%d").is_err());
        assert!(chrono::NaiveDate::parse_from_str("2024-03-05", "%Y-%m-%d").is_ok());
    }
}
