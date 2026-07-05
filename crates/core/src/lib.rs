pub mod dates;
pub mod discover;
pub mod ids;
pub mod model;
pub mod query;
pub mod store;

// Modules added by later tasks (uncomment as they land):
// pub mod resolve;
// pub mod ops;

/// Crate-wide error type. Display strings are the CLI's user-facing
/// messages, verbatim from the spec's Error Handling table — the CLI
/// prints `Display` to stderr and exits 1.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("project path not found: {0}")]
    ProjectNotFound(String),
    #[error("invalid project path: must be relative and inside WIKI_ROOT")]
    InvalidProjectPath,
    #[error("no task found with id: {0}")]
    IdNotFound(String),
    #[error("no task found with title: \"{0}\"")]
    TitleNotFound(String),
    #[error("ambiguous title; use --project to narrow or use ID\n{candidates}")]
    AmbiguousTitle { candidates: String },
    #[error("invalid --id format, expected <task-id> or <task-id>:<subtask-id>")]
    MalformedIdSelector,
    #[error("no sub-task found with id: {subtask} in task {task}")]
    SubtaskNotFound { subtask: String, task: String },
    #[error("--{flag} is not valid when editing a sub-task")]
    FlagInvalidForSubtask { flag: String },
    #[error("cannot parse date: \"{0}\"")]
    DateParse(String),
    #[error("invalid {field}: {reason}")]
    InvalidField { field: String, reason: String },
    #[error("edit requires at least one field to change")]
    EmptyEdit,
    #[error("cannot parse task file: {path}: {message}")]
    TaskFileParse { path: String, message: String },
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Serialize(#[from] toml::ser::Error),
}
