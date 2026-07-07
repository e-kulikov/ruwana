use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "ruwana",
    version,
    about = "Agent-facing task tracker for a personal wiki"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new task
    #[command(visible_alias = "a", alias = "+")]
    Add(AddArgs),
    /// Mark a task (or one sub-task) as done
    #[command(visible_alias = "do")]
    Done(TargetArgs),
    /// Reopen a task (or one sub-task)
    #[command(visible_alias = "undo")]
    Undone(TargetArgs),
    /// List tasks with filters
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Show one task in full
    Show(ShowArgs),
    /// List the sub-tasks of one task
    Tasks(LookupArgs),
    /// Update task fields, or rename one sub-task
    #[command(visible_alias = "e")]
    Edit(EditArgs),
    /// Delete a task, or one sub-task, permanently
    #[command(visible_aliases = ["remove", "delete"], alias = "-")]
    Rm(RmArgs),
}

/// Positional `<id-or-title>` vs `--id` target. Mutual exclusion and
/// "at least one" are checked in main (not clap groups) so the error
/// message and exit code match the spec exactly.
#[derive(Args)]
pub struct TargetArgs {
    /// Task ID or exact task title
    pub id_or_title: Option<String>,
    /// <task-id> or <task-id>:<subtask-id>
    #[arg(long)]
    pub id: Option<String>,
    /// Project path relative to WIKI_ROOT
    #[arg(long)]
    pub project: Option<String>,
}

/// Positional-only target for `show` and `tasks` — the spec gives `--id`
/// to done/undone/rm/edit only, so these two commands must not accept it.
#[derive(Args)]
pub struct LookupArgs {
    /// Task ID or exact task title
    pub id_or_title: String,
    /// Project path relative to WIKI_ROOT
    #[arg(long)]
    pub project: Option<String>,
}

#[derive(Args)]
pub struct AddArgs {
    /// Project path relative to WIKI_ROOT (required for add)
    #[arg(long)]
    pub project: String,
    /// Task title
    pub title: String,
    #[arg(long)]
    pub description: Option<String>,
    /// Add a sub-task (repeatable)
    #[arg(long = "task")]
    pub subtasks: Vec<String>,
    /// Add a tag (repeatable)
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Due date (human-friendly or explicit)
    #[arg(long)]
    pub due: Option<String>,
    /// Add a source (repeatable)
    #[arg(long = "source")]
    pub sources: Vec<String>,
}

#[derive(Args)]
pub struct ListArgs {
    #[arg(long)]
    pub project: Option<String>,
    /// Open tasks only (default)
    #[arg(long, visible_alias = "incomplete", group = "status")]
    pub open: bool,
    /// Done tasks only
    #[arg(long, visible_aliases = ["closed", "completed"], group = "status")]
    pub done: bool,
    /// All tasks regardless of status
    #[arg(long, group = "status")]
    pub all: bool,
    /// Open tasks whose due date is in the past
    #[arg(long, group = "status")]
    pub overdue: bool,
    /// Open tasks due today or tomorrow
    #[arg(long, group = "status")]
    pub urgent: bool,
    /// Exact due-date match
    #[arg(long)]
    pub due: Option<String>,
    #[arg(long)]
    pub created_before: Option<String>,
    #[arg(long)]
    pub created_after: Option<String>,
    #[arg(long)]
    pub modified_before: Option<String>,
    #[arg(long)]
    pub modified_after: Option<String>,
    /// Filter by tag (repeatable, AND)
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Filter by source (repeatable, OR)
    #[arg(long = "source")]
    pub sources: Vec<String>,
    #[arg(long, value_enum, default_value_t = SortArg::Due)]
    pub sort: SortArg,
    #[arg(long, value_enum, default_value_t = FormatArg::Text)]
    pub format: FormatArg,
}

#[derive(Args)]
pub struct ShowArgs {
    #[command(flatten)]
    pub lookup: LookupArgs,
    #[arg(long, value_enum, default_value_t = FormatArg::Text)]
    pub format: FormatArg,
}

#[derive(Args)]
pub struct EditArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    /// New title (or new sub-task text with a compound --id)
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub due: Option<String>,
    #[arg(long = "tag")]
    pub add_tags: Vec<String>,
    #[arg(long = "remove-tag")]
    pub remove_tags: Vec<String>,
    #[arg(long = "source")]
    pub add_sources: Vec<String>,
    #[arg(long = "remove-source")]
    pub remove_sources: Vec<String>,
}

#[derive(Args)]
pub struct RmArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    /// Skip the confirmation prompt (required for non-interactive use)
    #[arg(long, visible_alias = "yes")]
    pub force: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum SortArg {
    Due,
    Created,
    Modified,
    Title,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
pub enum FormatArg {
    Text,
    Json,
}
