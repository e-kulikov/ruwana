use chrono::{DateTime, FixedOffset};
use ruwana_core::model::{Status, SubTask};
use ruwana_core::ops::ActionReport;
use ruwana_core::store::TaskRecord;
use serde_json::json;

/// Render stored free text in a single physical terminal line. Stored data,
/// JSON, and verbatim TOML deliberately retain their original line endings.
fn text_line(text: &str) -> String {
    text.replace('\r', "\\r").replace('\n', "\\n")
}

/// One line per task, column-aligned:
/// `a3bc9f2e  godel/ai-practice  Review Q3 OKRs        due: 2024-03-01`
/// Due column: `due: YYYY-MM-DD` (calendar date, read in the stored
/// instant's own offset), `overdue` for past-due open tasks, empty when no
/// due date.
///
/// `due` is rendered from its own stored offset — the offset that was
/// local at the moment it was saved — never reprojected through `now`'s.
/// This avoids DST reprojection errors: reprojecting through `now`'s
/// offset shifts the calendar date whenever `due` and `now` straddle a DST
/// boundary (e.g. a winter due date queried in summer) — that was a real
/// bug here; don't reintroduce it. When a due date was saved from a
/// different timezone than the querying machine's current one, the
/// rendered date reflects where/when it was set, not the querying
/// machine's current local date.
pub fn list_text(records: &[TaskRecord], now: DateTime<FixedOffset>) -> String {
    let rows: Vec<(String, String, String, String)> = records
        .iter()
        .map(|r| {
            let due_col = match r.task.due {
                Some(due) if r.task.status == Status::Open && due < now => "overdue".to_string(),
                Some(due) => format!("due: {}", due.format("%Y-%m-%d")),
                None => String::new(),
            };
            (
                r.task.id.clone(),
                r.project.clone(),
                text_line(&r.task.title),
                due_col,
            )
        })
        .collect();
    let id_width = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    let project_width = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
    let title_width = rows.iter().map(|r| r.2.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(id, project, title, due)| {
            format!("{id:<id_width$}  {project:<project_width$}  {title:<title_width$}  {due}")
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Confirmation line for done/undone/edit/rm:
/// `done a3bc9f2e  Review Q3 OKRs`
/// `done a3bc9f2e:gh7f  Sub-task one` (when a sub-task was addressed)
pub fn action_line(verb: &str, report: &ActionReport) -> String {
    match &report.subtask {
        Some(sub) => format!(
            "{verb} {}:{}  {}",
            report.task_id,
            sub.id,
            text_line(&sub.text)
        ),
        None => format!("{verb} {}  {}", report.task_id, text_line(&report.title)),
    }
}

/// Sub-task listing: `gh7f  [ ]  Sub-task one`
pub fn subtasks_text(subs: &[SubTask]) -> String {
    subs.iter()
        .map(|s| {
            format!(
                "{}  [{}]  {}",
                s.id,
                if s.done { "x" } else { " " },
                text_line(&s.text)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn record_json(record: &TaskRecord) -> serde_json::Value {
    json!({
        "id": record.task.id,
        "project": record.project,
        "title": record.task.title,
        "status": record.task.status.to_string(),
        "due": record.task.due.map(|d| d.to_rfc3339()),
        "tags": record.task.tags,
        "source": record.task.source,
        "created": record.task.created.to_rfc3339(),
        "modified": record.task.modified.to_rfc3339(),
    })
}

/// JSON array for `list --format json` (spec: list JSON output — no
/// description/related/sub-tasks; use show for the full record).
pub fn list_json(records: &[TaskRecord]) -> String {
    serde_json::Value::Array(records.iter().map(record_json).collect()).to_string()
}

/// Full single-task object for `show --format json`.
pub fn show_json(record: &TaskRecord) -> String {
    let mut value = record_json(record);
    let obj = value.as_object_mut().expect("record_json is an object");
    obj.insert("description".into(), json!(record.task.description));
    obj.insert("related".into(), json!(record.task.related));
    obj.insert(
        "tasks".into(),
        json!(
            record
                .task
                .tasks
                .iter()
                .map(|s| json!({
                    "id": s.id, "text": s.text, "done": s.done,
                }))
                .collect::<Vec<_>>()
        ),
    );
    obj.insert("file_path".into(), json!(record.path.display().to_string()));
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use ruwana_core::model::Task;

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2024-03-13T10:00:00+01:00").unwrap()
    }

    fn record(title: &str, tasks: Vec<SubTask>) -> TaskRecord {
        TaskRecord {
            task: Task {
                id: "abc1de2f".into(),
                title: title.into(),
                status: Status::Open,
                due: None,
                tags: vec![],
                source: vec![],
                created: now(),
                modified: now(),
                related: vec![],
                description: None,
                tasks,
            },
            project: "proj".into(),
            path: std::path::PathBuf::from("proj/.ruwana/abc1de2f.toml"),
        }
    }

    #[test]
    fn human_text_output_escapes_embedded_line_endings() {
        let raw = "first\nsecond\rthird\r\nfourth";
        let escaped = "first\\nsecond\\rthird\\r\\nfourth";
        let list = list_text(&[record(raw, vec![])], now());
        assert_eq!(list, format!("abc1de2f  proj  {escaped}"));
        assert_eq!(list.lines().count(), 1);

        let list_json_value: serde_json::Value =
            serde_json::from_str(&list_json(&[record(raw, vec![])])).unwrap();
        assert_eq!(list_json_value[0]["title"], raw);

        let subs = subtasks_text(&[SubTask {
            id: "gh7f".into(),
            text: raw.into(),
            done: false,
        }]);
        assert_eq!(subs, format!("gh7f  [ ]  {escaped}"));
        assert_eq!(subs.lines().count(), 1);

        let task_report = ActionReport {
            task_id: "abc1de2f".into(),
            title: raw.into(),
            subtask: None,
            warnings: vec![],
        };
        assert_eq!(
            action_line("done", &task_report),
            format!("done abc1de2f  {escaped}")
        );

        let subtask_report = ActionReport {
            task_id: "abc1de2f".into(),
            title: "Parent".into(),
            subtask: Some(SubTask {
                id: "gh7f".into(),
                text: raw.into(),
                done: false,
            }),
            warnings: vec![],
        };
        let action = action_line("done", &subtask_report);
        assert_eq!(action, format!("done abc1de2f:gh7f  {escaped}"));
        assert_eq!(action.lines().count(), 1);

        let full_json: serde_json::Value = serde_json::from_str(&show_json(&record(
            raw,
            vec![SubTask {
                id: "gh7f".into(),
                text: raw.into(),
                done: false,
            }],
        )))
        .unwrap();
        assert_eq!(full_json["title"], raw);
        assert_eq!(full_json["tasks"][0]["text"], raw);
    }
}
