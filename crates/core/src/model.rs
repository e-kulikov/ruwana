use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Done,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Status::Open => "open",
            Status::Done => "done",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubTask {
    pub id: String,
    pub text: String,
    pub done: bool,
}

/// Field order here IS the canonical on-disk order (serde serializes in
/// declaration order; `tasks` is last because TOML arrays-of-tables must
/// follow all top-level keys).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<DateTime<FixedOffset>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<String>,
    pub created: DateTime<FixedOffset>,
    pub modified: DateTime<FixedOffset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<SubTask>,
}

pub fn to_toml(task: &Task) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(task)
}

pub fn from_toml(input: &str) -> Result<Task, toml::de::Error> {
    toml::from_str(input)
}

pub fn validate(task: &Task) -> Result<(), String> {
    if !crate::ids::is_task_id_shaped(&task.id) {
        return Err(format!("invalid task id {:?}", task.id));
    }
    if task
        .description
        .as_deref()
        .is_some_and(|description| description.trim().is_empty())
    {
        return Err("description must not be empty".into());
    }
    let mut seen = HashSet::new();
    for subtask in &task.tasks {
        if !crate::ids::is_subtask_id_shaped(&subtask.id) {
            return Err(format!("invalid sub-task id {:?}", subtask.id));
        }
        if !seen.insert(&subtask.id) {
            return Err(format!("duplicate sub-task id {:?}", subtask.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn dt(s: &str) -> chrono::DateTime<chrono::FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    fn sample_task() -> Task {
        Task {
            id: "abc1de2f".into(),
            title: "Write the onboarding doc".into(),
            status: Status::Open,
            due: Some(dt("2024-03-15T23:59:59+01:00")),
            tags: vec!["review".into(), "okr".into()],
            source: vec!["meeting-2024-01-15".into()],
            created: dt("2024-01-15T09:32:11+01:00"),
            modified: dt("2024-01-20T14:05:47+01:00"),
            related: vec!["https://example.com".into()],
            description: Some("A description.\nSecond line.".into()),
            tasks: vec![SubTask {
                id: "gh7f".into(),
                text: "Sub one".into(),
                done: false,
            }],
        }
    }

    #[test]
    fn toml_round_trip_is_fixpoint() {
        let task = sample_task();
        let once = to_toml(&task).unwrap();
        let parsed = from_toml(&once).unwrap();
        let twice = to_toml(&parsed).unwrap();
        assert_eq!(task, parsed);
        assert_eq!(once, twice);
    }

    #[test]
    fn canonical_field_order_and_rfc3339() {
        let toml_str = to_toml(&sample_task()).unwrap();
        let pos = |needle: &str| {
            toml_str
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"))
        };
        assert!(pos("id =") < pos("title ="));
        assert!(pos("title =") < pos("status ="));
        assert!(pos("status =") < pos("due ="));
        assert!(pos("due =") < pos("tags ="));
        assert!(pos("tags =") < pos("source ="));
        assert!(pos("source =") < pos("created ="));
        assert!(pos("created =") < pos("modified ="));
        assert!(pos("modified =") < pos("related ="));
        assert!(pos("related =") < pos("description ="));
        assert!(pos("description =") < pos("[[tasks]]"));
        assert!(toml_str.contains("\"2024-03-15T23:59:59+01:00\""));
        assert!(toml_str.contains("status = \"open\""));
    }

    #[test]
    fn optional_and_empty_fields_are_omitted() {
        let task = Task {
            id: "a3bc9f2e".into(),
            title: "Bare".into(),
            status: Status::Done,
            due: None,
            tags: vec![],
            source: vec![],
            created: dt("2024-01-15T09:32:11+01:00"),
            modified: dt("2024-01-15T09:32:11+01:00"),
            related: vec![],
            description: None,
            tasks: vec![],
        };
        let toml_str = to_toml(&task).unwrap();
        for absent in [
            "due",
            "tags",
            "source",
            "related",
            "description",
            "[[tasks]]",
        ] {
            assert!(
                !toml_str.contains(absent),
                "should omit {absent}:\n{toml_str}"
            );
        }
        let parsed = from_toml(&toml_str).unwrap();
        assert_eq!(task, parsed);
    }

    #[test]
    fn blank_description_is_invalid() {
        let mut task = sample_task();
        task.description = Some(" \t\n ".into());
        assert_eq!(validate(&task), Err("description must not be empty".into()));
    }

    #[test]
    fn unknown_status_is_a_parse_error() {
        let bad = to_toml(&sample_task())
            .unwrap()
            .replace("\"open\"", "\"pending\"");
        assert!(from_toml(&bad).is_err());
    }

    #[test]
    fn missing_required_field_is_a_parse_error() {
        // Drop the `title` line entirely.
        let toml_str = to_toml(&sample_task()).unwrap();
        let without_title: String = toml_str
            .lines()
            .filter(|l| !l.starts_with("title "))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(from_toml(&without_title).is_err());
    }
}
