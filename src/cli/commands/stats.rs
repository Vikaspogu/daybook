use anyhow::Result;
use colored::Colorize;

use crate::config::load_config;
use crate::ledger::StatsReport;

/// `daybook stats [--week | --month | --days N]` — the R4 surface.
/// Fetches the aggregate report from the ledger server and prints it.
pub async fn run(week: bool, days: Option<usize>, month: bool) -> Result<()> {
    let days = if month {
        30
    } else if week {
        7
    } else {
        days.unwrap_or(7)
    };
    let config = load_config()?;
    let report = crate::push::fetch_stats(&config, days).await?;
    print_report(&report, days);
    Ok(())
}

pub fn print_report(r: &StatsReport, days: usize) {
    println!(
        "\n{}",
        format!("  Work Stats (last {} days)", days)
            .bold()
            .bright_yellow()
    );
    println!("{}", "  ─────────────────────────────".dimmed());

    if r.days.is_empty() {
        println!("\n  No sessions in the ledger for this window.");
        println!("  (Push machines: `daybook scan` with push.url + push.token configured)");
        return;
    }

    println!(
        "\n  {} {} sessions, {} in / {} out tokens, ${:.2}",
        "Window:".bold(),
        r.totals.sessions.to_string().bright_yellow(),
        format_tokens(r.totals.input).bright_yellow(),
        format_tokens(r.totals.output).bright_yellow(),
        r.totals.cost_usd
    );
    if let Some(rate) = r.totals.cache_hit_rate {
        println!("  Cache hit rate: {:.0}%", rate * 100.0);
    }

    println!("\n  {}", "Per day:".bold());
    println!(
        "  {}",
        "date         sessions    input     output    cost    tests   friction".dimmed()
    );
    for d in &r.days {
        println!(
            "  {} {:>8} {:>9} {:>9} {:>7.2} {:>8} {:>10}",
            d.date,
            d.sessions,
            format_tokens(d.input),
            format_tokens(d.output),
            d.cost_usd,
            d.test_runs,
            d.failed_tool_calls + d.interrupted,
        );
    }

    println!("\n  {}", "Session trend:".bold());
    let max = r.trend.iter().map(|t| t.sessions).max().unwrap_or(1);
    for t in &r.trend {
        // max >= 1 via unwrap_or(1), so no divide-by-zero guard needed
        let bar_len = (t.sessions * 20 / max) as usize;
        println!(
            "  {} {} {}",
            t.date.dimmed(),
            "\u{2588}".repeat(bar_len).bright_yellow(),
            t.sessions
        );
    }

    if !r.friction_distribution.is_empty() {
        println!("\n  {}", "Friction distribution:".bold());
        let mut items: Vec<_> = r.friction_distribution.iter().collect();
        items.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        for (k, v) in items {
            println!("  {:>2}  {}", v.to_string().bright_red(), k);
        }
    }

    let tests_per_day = r.days.len().max(1);
    println!(
        "\n  {} {} | {} | interrupted {}",
        "Averages:".bold(),
        format!(
            "{:.1} tests/day",
            r.totals.test_runs as f64 / tests_per_day as f64
        )
        .bright_yellow(),
        format!(
            "{:.1} sessions/day",
            r.totals.sessions as f64 / tests_per_day as f64
        )
        .bright_yellow(),
        r.totals.interrupted,
    );
    println!();
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::DailyStats;

    #[test]
    fn formats_token_counts_compactly() {
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(1_234), "1.2k");
        assert_eq!(format_tokens(2_500_000), "2.5M");
    }

    #[test]
    fn empty_report_prints_without_panic() {
        let r = StatsReport::default();
        print_report(&r, 7); // smoke: must not panic on empty data
    }

    #[test]
    fn report_with_days_prints_them() {
        let r = StatsReport {
            days: vec![DailyStats {
                date: "2026-10-27".into(),
                sessions: 3,
                input: 10_000,
                output: 2_000,
                cache_read: 90_000,
                cache_creation: 0,
                cost_usd: 0.42,
                test_runs: 4,
                failed_tool_calls: 1,
                interrupted: 2,
                cache_hit_rate: Some(0.9),
            }],
            trend: vec![crate::ledger::TrendPoint {
                date: "2026-10-27".into(),
                sessions: 3,
            }],
            ..Default::default()
        };
        print_report(&r, 7);
    }
}
