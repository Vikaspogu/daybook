use anyhow::{Context, Result};

use crate::config::load_config;

/// `daybook query <date>` — recall surface for agents (R2/R3).
/// Prints one day's overview plus every session's summary from the ledger.
pub async fn run(date: String) -> Result<()> {
    let parsed = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .with_context(|| format!("date must be YYYY-MM-DD, got {date:?}"))?;
    let config = load_config()?;
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
