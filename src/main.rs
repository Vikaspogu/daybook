mod archive;
mod auto_summarize;
mod cli;
mod config;
mod hooks;
mod insights;
mod jobs;
mod ledger;
mod push;
mod server;
mod summarizer;
mod transcript;
mod usage;

use anyhow::Result;
use clap::Parser;
use cli::args::{Cli, Commands, HookType, JobsAction};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    config::set_config_path(cli.config.clone());
    match cli.command {
        Commands::Init {
            storage_path,
            yes,
            haiku,
        } => cli::commands::init::run(storage_path, !yes, haiku).await,
        Commands::Hook { hook_type } => match hook_type {
            HookType::SessionStart => hooks::session_start::handle().await,
            HookType::SessionEnd => hooks::session_end::handle().await,
        },
        Commands::View {
            date,
            summary_only,
            list,
        } => cli::commands::view::run(date, summary_only, list).await,
        Commands::Today { summary_only, list } => {
            cli::commands::view::run_today(summary_only, list).await
        }
        Commands::Yest { summary_only, list } => {
            cli::commands::view::run_yesterday(summary_only, list).await
        }
        Commands::Summarize {
            transcript,
            task_name,
            cwd,
            foreground,
            job_id,
        } => cli::commands::summarize::run(transcript, task_name, cwd, foreground, job_id).await,
        Commands::Scan { no_llm, local, day } => cli::commands::scan::run(day, no_llm, local).await,
        Commands::Stats { week, days, month } => cli::commands::stats::run(week, days, month).await,
        Commands::Digest {
            relative_date,
            date,
            background,
            force,
        } => cli::commands::digest::run(relative_date, date, background, force).await,
        Commands::ExtractSkill {
            date,
            session,
            output,
        } => cli::commands::extract::run_skill(date, session, output).await,
        Commands::ExtractCommand {
            date,
            session,
            output,
        } => cli::commands::extract::run_command(date, session, output).await,
        Commands::ReviewSkills { install, delete } => {
            cli::commands::skills::run_review(install, delete).await
        }
        Commands::Config {
            set_storage,
            show,
            interactive,
        } => cli::commands::config::run(set_storage, show, interactive).await,
        Commands::Jobs { action } => match action {
            JobsAction::List { all } => cli::commands::jobs::list(all).await,
            JobsAction::Log {
                job_id,
                tail,
                follow,
            } => cli::commands::jobs::log(job_id, tail, follow).await,
            JobsAction::Kill { job_id } => cli::commands::jobs::kill(job_id).await,
            JobsAction::Cleanup { days, dry_run } => {
                cli::commands::jobs::cleanup(days, dry_run).await
            }
        },
        Commands::Insights { days } => cli::commands::insights::run(days).await,
        Commands::Show {
            port,
            host,
            no_open,
        } => cli::commands::show::run(port, host, !no_open).await,
        Commands::Serve { port, host } => cli::commands::show::run_serve(port, host).await,
        Commands::Token {
            list,
            revoke,
            label,
        } => cli::commands::token::run(list, revoke, label),
    }
}
