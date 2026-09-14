pub mod dates;
mod discover;
pub mod ids;
pub mod model;
pub mod ops;
pub mod query;
pub mod resolve;
pub mod store;

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
    #[error("ambiguous task id; use --project to narrow: {id} ({projects})")]
    AmbiguousId { id: String, projects: String },
    #[error("no task found with title: \"{0}\"")]
    TitleNotFound(String),
    #[error("ambiguous title; use --project to narrow or use ID\n{candidates}")]
    AmbiguousTitle { candidates: String },
    #[error("{error}")]
    Resolution {
        error: Box<Error>,
        warnings: Vec<store::Warning>,
    },
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

impl Error {
    pub fn warnings(&self) -> &[store::Warning] {
        match self {
            Self::Resolution { warnings, .. } => warnings,
            _ => &[],
        }
    }

    pub(crate) fn with_warnings(error: Self, warnings: Vec<store::Warning>) -> Self {
        if warnings.is_empty() {
            return error;
        }
        match error {
            Self::Resolution {
                error,
                warnings: mut later_warnings,
            } => {
                let mut warnings = warnings;
                warnings.append(&mut later_warnings);
                Self::Resolution { error, warnings }
            }
            error => Self::Resolution {
                error: Box::new(error),
                warnings,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_warnings_flattens_existing_resolution_in_encounter_order() {
        let later = Error::with_warnings(
            Error::Io(std::io::Error::other("later failure")),
            vec![store::Warning("later warning".into())],
        );
        let merged = Error::with_warnings(later, vec![store::Warning("earlier warning".into())]);

        assert_eq!(
            merged
                .warnings()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["earlier warning", "later warning"]
        );
        assert!(
            matches!(merged, Error::Resolution { error, .. } if matches!(*error, Error::Io(_)))
        );
    }
}
