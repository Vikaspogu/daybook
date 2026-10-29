use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "daybook")]
#[command(about = "Daily Context Archive System for AI agents")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Config file path
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start web dashboard server
    Show {
        /// Port to listen on (default: 31456, auto-increment if occupied)
        #[arg(short, long)]
        port: Option<u16>,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Do not open browser automatically
        #[arg(long)]
        no_open: bool,
    },

    /// Run the headless ledger ingest server (no dashboard, no browser)
    Serve {
        /// Port (default: config server.port)
        #[arg(short, long)]
        port: Option<u16>,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Manage ingest tokens (mint by default)
    Token {
        /// List tokens
        #[arg(long)]
        list: bool,

        /// Revoke the token with this label
        #[arg(long)]
        revoke: Option<String>,

        /// Label for a newly minted token (default: machine hostname)
        #[arg(long)]
        label: Option<String>,

        /// Use a known token value (dbk_... 68 hex chars) instead of minting a
        /// random one. For GitOps init containers: ensures the token from the
        /// secret store exists in the ledger. Idempotent (upsert by label).
        /// Reads DAYBOOK_TOKEN_VALUE from the environment when --value is absent.
        #[arg(long, requires = "label", conflicts_with_all = ["list", "revoke"], env = "DAYBOOK_TOKEN_VALUE")]
        value: Option<String>,
    },

    /// View archives (interactive date selection if no date specified)
    View {
        /// Date to view (format: yyyy-mm-dd)
        #[arg(short, long)]
        date: Option<String>,

        /// Show daily summary only
        #[arg(long)]
        summary_only: bool,

        /// List all sessions for the day
        #[arg(long)]
        list: bool,
    },

    /// View today's archive
    Today {
        /// Show daily summary only
        #[arg(long)]
        summary_only: bool,

        /// List all sessions
        #[arg(long)]
        list: bool,
    },

    /// View yesterday's archive
    Yest {
        /// Show daily summary only
        #[arg(long)]
        summary_only: bool,

        /// List all sessions
        #[arg(long)]
        list: bool,
    },

    /// Manually trigger summarization
    Summarize {
        /// Session transcript path
        #[arg(short, long)]
        transcript: PathBuf,

        /// Task name for the archive
        #[arg(short = 'n', long)]
        task_name: Option<String>,

        /// Working directory of the session
        #[arg(long)]
        cwd: Option<PathBuf>,

        /// Run in foreground (not background)
        #[arg(long)]
        foreground: bool,

        /// Job ID for tracking (internal use)
        #[arg(long)]
        job_id: Option<String>,
    },

    /// Print one day's work overview + session summaries from the ledger server
    Query {
        /// Date to query (format: yyyy-mm-dd, default: today UTC)
        #[arg(value_name = "DATE", default_value_t = chrono::Utc::now().format("%Y-%m-%d").to_string())]
        date: String,
    },
    /// Discover sessions from all agent runtimes and list them (no archiving)
    Scan {
        /// Accepted for gate compatibility; S2 scans are always non-LLM
        #[arg(long)]
        no_llm: bool,

        /// Accepted for gate compatibility; S2 scans are always local
        #[arg(long)]
        local: bool,

        /// Which day to list: today, yesterday, all, or YYYY-MM-DD (default: today)
        #[arg(long)]
        day: Option<String>,
    },
    /// Stats & insights over the ledger (sessions, tokens, cost, friction)
    Stats {
        /// Last 7 days
        #[arg(long, conflicts_with = "days")]
        week: bool,

        /// How many days back (default 7; --month = 30)
        #[arg(long)]
        days: Option<usize>,

        /// Last 30 days
        #[arg(long, conflicts_with_all = ["week", "days"])]
        month: bool,
    },

    /// Generate daily digest from sessions (consolidate sessions into daily.md)
    Digest {
        /// Relative date (e.g., "yest" or "yesterday" for yesterday)
        #[arg(value_name = "RELATIVE_DATE")]
        relative_date: Option<String>,

        /// Date to digest (format: yyyy-mm-dd, default: today)
        #[arg(short, long)]
        date: Option<String>,

        /// Run in background (default: foreground)
        #[arg(long)]
        background: bool,

        /// Force regenerate daily summary even without session files (re-process existing daily.md)
        #[arg(short, long)]
        force: bool,
    },

    /// Extract skill from archive
    ExtractSkill {
        /// Date to search (default: today)
        #[arg(short, long)]
        date: Option<String>,

        /// Session to extract from
        #[arg(short, long)]
        session: Option<String>,

        /// Output directory for skill
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Extract command from archive
    ExtractCommand {
        /// Date to search (default: today)
        #[arg(short, long)]
        date: Option<String>,

        /// Session to extract from
        #[arg(short, long)]
        session: Option<String>,

        /// Output directory for command
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Review and manage pending skills
    ReviewSkills {
        /// Install a pending skill (format: YYYY-MM-DD/skill-name)
        #[arg(long)]
        install: Option<String>,

        /// Delete a pending skill (format: YYYY-MM-DD/skill-name)
        #[arg(long)]
        delete: Option<String>,
    },

    /// Manage background jobs
    Jobs {
        #[command(subcommand)]
        action: JobsAction,
    },

    /// Initialize configuration (interactive by default)
    Init {
        /// Storage path (default: ~/.daybook)
        #[arg(short, long)]
        storage_path: Option<PathBuf>,

        /// Skip interactive prompts, use defaults
        #[arg(short = 'y', long = "yes")]
        yes: bool,

        /// Use haiku model for summarization (default: sonnet)
        #[arg(long)]
        haiku: bool,
    },

    /// Show or update configuration
    Config {
        /// Set storage path
        #[arg(long)]
        set_storage: Option<PathBuf>,

        /// Show current config
        #[arg(long)]
        show: bool,

        /// Interactive configuration mode
        #[arg(short, long)]
        interactive: bool,
    },

    /// Generate insights and trend analysis from archives
    Insights {
        /// Number of days to analyze (default: 30)
        #[arg(short, long, default_value = "30")]
        days: usize,
    },

    /// Handle Claude Code hooks (internal use)
    Hook {
        #[command(subcommand)]
        hook_type: HookType,
    },
}

#[derive(Subcommand)]
pub enum JobsAction {
    /// List background jobs
    List {
        /// Show all jobs (including completed)
        #[arg(short, long)]
        all: bool,
    },

    /// Show job log
    Log {
        /// Job ID
        job_id: String,

        /// Show only last N lines
        #[arg(short, long)]
        tail: Option<usize>,

        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,
    },

    /// Kill a running job
    Kill {
        /// Job ID
        job_id: String,
    },

    /// Cleanup old jobs
    Cleanup {
        /// Keep jobs from last N days (default: 7)
        #[arg(short, long, default_value = "7")]
        days: u32,

        /// Show what would be removed without removing
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum HookType {
    /// SessionStart hook handler
    SessionStart,

    /// SessionEnd hook handler
    SessionEnd,
}
