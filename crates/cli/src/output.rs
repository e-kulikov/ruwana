use chrono::{DateTime, FixedOffset};
use ruwana_core::model::{Status, SubTask};
use ruwana_core::ops::ActionReport;
use ruwana_core::store::TaskRecord;
use serde_json::json;

/// One line per task, column-aligned:
/// `a3bc9f2e  godel/ai-practice  Review Q3 OKRs        due: 2024-03-01`
/// Due column: `due: YYYY-MM-DD` (calendar date in the querying tz),
/// `overdue` for past-due open tasks, empty when no due date.
pub fn list_text(records: &[TaskRecord], now: DateTime<FixedOffset>) -> String {
    let rows: Vec<(String, String, String, String)> = records
        .iter()
        .map(|r| {
            let due_col = match r.task.due {
                Some(due) if r.task.status == Status::Open && due < now => "overdue".to_string(),
                Some(due) => format!("due: {}", due.with_timezone(now.offset()).format("%Y-%m-%d")),
                None => String::new(),
            };
            (r.task.id.clone(), r.project.clone(), r.task.title.clone(), due_col)
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
        Some(sub) => format!("{verb} {}:{}  {}", report.task_id, sub.id, sub.text),
        None => format!("{verb} {}  {}", report.task_id, report.title),
    }
}

/// Sub-task listing: `gh7f  [ ]  Sub-task one`
pub fn subtasks_text(subs: &[SubTask]) -> String {
    subs.iter()
        .map(|s| format!("{}  [{}]  {}", s.id, if s.done { "x" } else { " " }, s.text))
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
        json!(record.task.tasks.iter().map(|s| json!({
            "id": s.id, "text": s.text, "done": s.done,
        })).collect::<Vec<_>>()),
    );
    obj.insert("file_path".into(), json!(record.path.display().to_string()));
    value.to_string()
}
