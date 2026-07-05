use chrono::{DateTime, FixedOffset};
use ruwana_core::model::{Status, SubTask};
use ruwana_core::ops::ActionReport;
use ruwana_core::store::TaskRecord;

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

pub fn list_json(_records: &[TaskRecord]) -> String {
    "[]".to_string() // Task 13 implements this
}

pub fn show_json(_record: &TaskRecord) -> String {
    "{}".to_string() // Task 13 implements this
}
