use clap::{Args, Parser, Subcommand};
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

    /// Query a day, or recall project sessions without knowing their dates
    Query {
        /// Date to query (format: yyyy-mm-dd, default: today)
        #[arg(value_name = "DATE")]
        date: Option<String>,

        /// Restrict recall to this project
        #[arg(long)]
        project: Option<String>,

        /// Words that must all appear in the session summary
        #[arg(long)]
        search: Option<String>,

        /// Maximum recall results (1-20)
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Discover sessions from all agent runtimes and list them (no archiving)
    Scan {
        /// Accepted for gate compatibility; S2 scans are always non-LLM
        #[arg(long)]
        no_llm: bool,

        /// Accepted for gate compatibility; S2 scans are always local
        #[arg(long)]
        local: bool,

        /// Which day to list: today, yesterday, all, or YYYY-MM-DD (default: today and yesterday)
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

        /// Configure agent coordination board (.mcp.json + hook snippets)
        #[arg(long)]
        board: bool,
    },

    /// Multi-agent worktree coordination board (locks, tasks, messages, MCP)
    Board(BoardArgs),

    /// Inspect archive, ledger, queue, and runtime integration health
    Doctor,

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

    /// PreToolUse hook handler (advisory board lock check before file edits)
    PreToolUse,

    /// PostToolUse hook handler (auto-lock touch + unread @mention alert after file edits)
    PostToolUse,
}

#[derive(Args, Debug, Clone)]
pub struct BoardArgs {
    /// Override project name (defaults to git parent repository name)
    #[arg(long, global = true)]
    pub project: Option<String>,

    /// Override agent handle (defaults to DAYBOOK_AGENT or <runtime>-<worktree>)
    #[arg(long, global = true)]
    pub agent: Option<String>,

    /// Output machine-readable JSON
    #[arg(long, global = true)]
    pub json: bool,

    /// Force local SQLite ledger (~/.daybook/ledger.db3) instead of remote push.url
    #[arg(long, global = true)]
    pub local: bool,

    #[command(subcommand)]
    pub action: Option<BoardAction>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BoardAction {
    /// Show active agents, file locks, tasks, unread @mentions, and recent messages
    Status {
        /// Only show messages/mentions after this message id
        #[arg(long)]
        since_id: Option<i64>,
    },

    /// Claim advisory file or glob locks
    Claim {
        /// Repo-relative file paths or globs (e.g. "src/server/**")
        #[arg(required = true, num_args = 1..)]
        paths: Vec<String>,

        /// Note explaining planned changes
        #[arg(short = 'm', long)]
        note: Option<String>,

        /// Lock TTL in minutes (1..=240, default 30)
        #[arg(long, default_value_t = 30)]
        ttl: u32,
    },

    /// Auto-claim or refresh a single file lock (used by PostToolUse hooks)
    Touch {
        /// File path edited
        path: String,

        /// Lock TTL in minutes (1..=240, default 30)
        #[arg(long, default_value_t = 30)]
        ttl: u32,
    },

    /// Check if a file path is locked by another agent
    Check {
        /// File path to check
        path: String,

        /// Exit with code 2 when locked by another agent
        #[arg(long)]
        strict: bool,
    },

    /// Release locks held by this agent
    Release {
        /// Specific lock id to release
        #[arg(long)]
        id: Option<String>,

        /// Release all locks held by this agent
        #[arg(long)]
        all: bool,

        /// Release only auto-claimed locks
        #[arg(long)]
        auto_only: bool,

        /// Force release another agent's lock on the same machine
        #[arg(long)]
        force: bool,
    },

    /// Post a message to the project board
    Post {
        /// Message body (supports @agent-handle mentions)
        body: String,

        /// Channel: general, handoff, blocker, or review
        #[arg(long, default_value = "general")]
        channel: String,

        /// Parent message id for threaded replies
        #[arg(long)]
        reply_to: Option<i64>,
    },

    /// List board messages
    Messages {
        /// Filter by channel (general, handoff, blocker, review)
        #[arg(long)]
        channel: Option<String>,

        /// Filter by @mentioned agent handle
        #[arg(long)]
        mention: Option<String>,

        /// Return messages with id > since_id
        #[arg(long)]
        since_id: Option<i64>,

        /// Max messages to return (1..=200)
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,
    },

    /// Manage shared tasks on the project board
    Task {
        #[command(subcommand)]
        action: BoardTaskAction,
    },

    /// Record agent heartbeat and current branch/worktree status
    Heartbeat {
        /// Status: active, idle, blocked, or done
        #[arg(long, default_value = "active")]
        status: String,

        /// Current task id or summary
        #[arg(long)]
        task: Option<String>,
    },

    /// Start the MCP JSON-RPC 2.0 stdio server for Claude Code, Codex, Pi, and T3 Code
    Mcp,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BoardTaskAction {
    /// Create or claim a task
    Claim {
        /// Task identifier (e.g. "AUTH-1")
        id: String,

        /// Human-readable task title (defaults to id on creation)
        #[arg(long)]
        title: Option<String>,

        /// Priority: low, medium, high, critical
        #[arg(long)]
        priority: Option<String>,

        /// Override git branch name (defaults to current worktree branch)
        #[arg(long)]
        branch: Option<String>,

        /// Status note
        #[arg(short = 'm', long)]
        note: Option<String>,

        /// Force takeover if already claimed on this machine
        #[arg(long)]
        force: bool,
    },

    /// Update task status, note, or PR URL
    Update {
        /// Task identifier
        id: String,

        /// Status: open, claimed, review, blocked, done
        #[arg(long)]
        status: String,

        /// Status note
        #[arg(short = 'm', long)]
        note: Option<String>,

        /// Pull request URL (http:// or https://)
        #[arg(long)]
        pr: Option<String>,

        /// Release locks held by this agent
        #[arg(long)]
        release_locks: Option<bool>,

        /// Force update on this machine
        #[arg(long)]
        force: bool,
    },

    /// Mark a task as done and release agent locks
    Done {
        /// Task identifier
        id: String,

        /// Completion note
        #[arg(short = 'm', long)]
        note: Option<String>,

        /// Pull request URL
        #[arg(long)]
        pr: Option<String>,

        /// Release locks held by this agent (default: true)
        #[arg(long)]
        release_locks: bool,
    },

    /// List tasks on the project board
    List,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_debug_assert_has_no_flag_collisions() {
        Cli::command().debug_assert();
    }
}
