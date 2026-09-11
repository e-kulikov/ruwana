use crate::Error;
use crate::ids;
use crate::model::{Status, SubTask, Task};
use crate::query::{self, Filter, SortKey};
use crate::resolve::{self, Resolved, Selector};
use crate::store::{Store, TaskRecord, Warning};
use chrono::{DateTime, FixedOffset};

/// Input for `add`. Dates arrive already parsed: callers (CLI, future
/// TUI) convert raw strings with `dates::parse_to_instant` first.
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub title: String,
    pub description: Option<String>,
    pub due: Option<DateTime<FixedOffset>>,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    /// Sub-task texts; each gets a generated 4-char id, done = false.
    pub subtasks: Vec<String>,
}

/// What a mutation touched — the CLI formats this for confirmation lines.
#[derive(Debug)]
pub struct ActionReport {
    pub task_id: String,
    pub title: String,
    /// Set iff the action addressed a single sub-task (post-change state;
    /// for sub-task removal, the removed entry).
    pub subtask: Option<SubTask>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, Default)]
pub struct EditFields {
    pub title: Option<String>,
    pub description: Option<String>,
    pub due: Option<DateTime<FixedOffset>>,
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
    pub add_sources: Vec<String>,
    pub remove_sources: Vec<String>,
}

impl EditFields {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.due.is_none()
            && self.add_tags.is_empty()
            && self.remove_tags.is_empty()
            && self.add_sources.is_empty()
            && self.remove_sources.is_empty()
    }

    /// Return the first whole-task-only flag set for a sub-task edit.
    /// `due_set` lets adapters reject a raw `--due` before parsing it.
    fn subtask_flag_error(&self, due_set: bool) -> Option<Error> {
        let offending = [
            (self.description.is_some(), "description"),
            (due_set, "due"),
            (!self.add_tags.is_empty(), "tag"),
            (!self.remove_tags.is_empty(), "remove-tag"),
            (!self.add_sources.is_empty(), "source"),
            (!self.remove_sources.is_empty(), "remove-source"),
        ];
        offending
            .iter()
            .find(|(set, _)| *set)
            .map(|(_, flag)| Error::FlagInvalidForSubtask {
                flag: (*flag).to_string(),
            })
    }
}

/// Whether raw edit arguments need sub-task-only preflight before parsing
/// whole-task values such as `--due`.
pub fn edit_needs_subtask_preflight(fields: &EditFields, due_set: bool) -> bool {
    fields.subtask_flag_error(due_set).is_some()
}

/// Check target-specific edit rules before an adapter parses whole-task-only
/// values such as `--due`.
pub fn preflight_edit(resolved: &Resolved, fields: &EditFields, due_set: bool) -> Option<Error> {
    resolved
        .subtask_id
        .as_ref()
        .and_then(|_| fields.subtask_flag_error(due_set))
}

fn validate_title(raw: &str) -> Result<String, Error> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidField {
            field: "title".into(),
            reason: "must not be empty".into(),
        });
    }
    Ok(trimmed.to_string())
}

/// Trim, reject empties, dedup preserving first-seen order (spec: Validation).
fn validate_values(field: &str, raw: Vec<String>) -> Result<Vec<String>, Error> {
    let mut out: Vec<String> = Vec::new();
    for value in raw {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(Error::InvalidField {
                field: field.into(),
                reason: "must not be empty".into(),
            });
        }
        if !out.iter().any(|v| v == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

/// Generate a task ID against the discovered projects and explicit destination —
/// existence is a filename stat per project (spec: ID Generation).
fn unique_task_id(store: &Store, project: &str) -> Result<String, Error> {
    unique_task_id_with(store, project, ids::generate_task_id)
}

fn unique_task_id_with<F>(store: &Store, project: &str, mut generate: F) -> Result<String, Error>
where
    F: FnMut() -> String,
{
    let mut projects = store.discover_projects()?;
    if !projects.iter().any(|candidate| candidate == project) {
        projects.push(project.to_string());
    }
    loop {
        let id = generate();
        if !projects
            .iter()
            .map(|p| store.task_exists(p, &id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|exists| exists)
        {
            return Ok(id);
        }
    }
}

fn new_subtask(text: &str, existing: &[SubTask]) -> SubTask {
    let id = loop {
        let candidate = ids::generate_subtask_id();
        if !existing.iter().any(|s| s.id == candidate) {
            break candidate;
        }
    };
    SubTask {
        id,
        text: text.to_string(),
        done: false,
    }
}

pub fn add(
    store: &Store,
    project: &str,
    new: NewTask,
    now: DateTime<FixedOffset>,
) -> Result<Task, Error> {
    store.validate_project(project)?;
    let title = validate_title(&new.title)?;
    let tags = validate_values("tag", new.tags)?;
    let sources = validate_values("source", new.sources)?;
    let mut subtasks: Vec<SubTask> = Vec::new();
    for text in &new.subtasks {
        let text = validate_title(text).map_err(|_| Error::InvalidField {
            field: "task".into(),
            reason: "must not be empty".into(),
        })?;
        subtasks.push(new_subtask(&text, &subtasks));
    }
    let task = Task {
        id: unique_task_id(store, project)?,
        title,
        status: Status::Open,
        due: new.due,
        tags,
        source: sources,
        created: now,
        modified: now,
        related: vec![],
        description: new.description,
        tasks: subtasks,
    };
    store.save(project, &task)?;
    Ok(task)
}

/// Resolution without mutation — for `show`, `tasks`, and the CLI's `rm`
/// confirmation prompt.
pub fn find(store: &Store, selector: &Selector, project: Option<&str>) -> Result<Resolved, Error> {
    resolve::resolve(store, selector, project)
}

fn report(record: &TaskRecord, subtask: Option<SubTask>, warnings: Vec<Warning>) -> ActionReport {
    ActionReport {
        task_id: record.task.id.clone(),
        title: record.task.title.clone(),
        subtask,
        warnings,
    }
}

pub fn set_done(
    store: &Store,
    selector: &Selector,
    project: Option<&str>,
    done: bool,
    now: DateTime<FixedOffset>,
) -> Result<ActionReport, Error> {
    let resolved = resolve::resolve(store, selector, project)?;
    let mut record = resolved.record;
    // True no-op when already in the requested state: exit success
    // without bumping `modified` or rewriting the file (spec:
    // idempotency — agents retry, retries must not churn the file).
    let already_there = match &resolved.subtask_id {
        Some(sub_id) => {
            record
                .task
                .tasks
                .iter()
                .find(|s| &s.id == sub_id)
                .expect("resolve verified the sub-task exists")
                .done
                == done
        }
        None => (record.task.status == Status::Done) == done,
    };
    if already_there {
        let untouched = resolved
            .subtask_id
            .as_ref()
            .and_then(|sub_id| record.task.tasks.iter().find(|s| &s.id == sub_id).cloned());
        return Ok(report(&record, untouched, resolved.warnings));
    }
    let touched_subtask = match &resolved.subtask_id {
        Some(sub_id) => {
            // Sub-task: flip the entry only; parent status untouched
            // (spec: Addressing a Sub-task).
            let entry = record
                .task
                .tasks
                .iter_mut()
                .find(|s| &s.id == sub_id)
                .expect("resolve verified the sub-task exists");
            entry.done = done;
            Some(entry.clone())
        }
        None => {
            record.task.status = if done { Status::Done } else { Status::Open };
            None
        }
    };
    record.task.modified = now;
    store.save(&record.project, &record.task)?;
    Ok(report(&record, touched_subtask, resolved.warnings))
}

pub fn edit(
    store: &Store,
    selector: &Selector,
    project: Option<&str>,
    fields: EditFields,
    now: DateTime<FixedOffset>,
) -> Result<ActionReport, Error> {
    if fields.is_empty() {
        return Err(Error::EmptyEdit);
    }
    let resolved = resolve::resolve(store, selector, project)?;
    edit_resolved(store, resolved, fields, now)
}

/// Edit an already-resolved target so adapters can resolve before parsing
/// whole-task-only values while retaining any resolution warnings.
pub fn edit_resolved(
    store: &Store,
    resolved: Resolved,
    fields: EditFields,
    now: DateTime<FixedOffset>,
) -> Result<ActionReport, Error> {
    if fields.is_empty() {
        return Err(resolved.into_error(Error::EmptyEdit));
    }
    if let Some(error) = preflight_edit(&resolved, &fields, fields.due.is_some()) {
        return Err(resolved.into_error(error));
    }
    let Resolved {
        mut record,
        subtask_id,
        warnings,
    } = resolved;
    let mutation = (|| -> Result<Option<SubTask>, Error> {
        let touched_subtask = match &subtask_id {
            Some(sub_id) => {
                // Sub-task edit supports --title only (spec: Addressing a
                // Sub-task). Report the first offending flag by CLI name.
                let text = validate_title(fields.title.as_deref().unwrap_or(""))?;
                let entry = record
                    .task
                    .tasks
                    .iter_mut()
                    .find(|s| &s.id == sub_id)
                    .expect("resolve verified the sub-task exists");
                entry.text = text;
                Some(entry.clone())
            }
            None => {
                let remove_tags = validate_values("tag", fields.remove_tags)?;
                let remove_sources = validate_values("source", fields.remove_sources)?;
                if let Some(title) = &fields.title {
                    record.task.title = validate_title(title)?;
                }
                if let Some(description) = fields.description {
                    record.task.description = Some(description);
                }
                if let Some(due) = fields.due {
                    record.task.due = Some(due);
                }
                for tag in validate_values("tag", fields.add_tags)? {
                    if !record.task.tags.contains(&tag) {
                        record.task.tags.push(tag);
                    }
                }
                record
                    .task
                    .tags
                    .retain(|t| !remove_tags.iter().any(|r| r == t));
                for source in validate_values("source", fields.add_sources)? {
                    if !record.task.source.contains(&source) {
                        record.task.source.push(source);
                    }
                }
                record
                    .task
                    .source
                    .retain(|s| !remove_sources.iter().any(|r| r == s));
                None
            }
        };
        record.task.modified = now;
        store.save(&record.project, &record.task)?;
        Ok(touched_subtask)
    })();
    match mutation {
        Ok(touched_subtask) => Ok(report(&record, touched_subtask, warnings)),
        Err(error) => Err(Error::with_warnings(error, warnings)),
    }
}

/// Delete a whole task (file) or one sub-task (entry + modified bump).
/// Takes an already-resolved target so the CLI can prompt in between.
pub fn remove(
    store: &Store,
    resolved: &Resolved,
    now: DateTime<FixedOffset>,
) -> Result<ActionReport, Error> {
    let mut record = resolved.record.clone();
    match &resolved.subtask_id {
        Some(sub_id) => {
            let removed = record
                .task
                .tasks
                .iter()
                .find(|s| &s.id == sub_id)
                .cloned()
                .expect("resolve verified the sub-task exists");
            record.task.tasks.retain(|s| &s.id != sub_id);
            record.task.modified = now;
            store.save(&record.project, &record.task)?;
            Ok(report(&record, Some(removed), Vec::new()))
        }
        None => {
            store.delete(&record.project, &record.task.id)?;
            Ok(report(&record, None, Vec::new()))
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    pub project: Option<String>,
    pub filter: Filter,
    pub sort: SortKey,
}

/// Gather records (one project, or every discovered project), then
/// filter + sort in memory (spec: Query Engine).
pub fn list<Tz: chrono::TimeZone>(
    store: &Store,
    q: &ListQuery,
    now: DateTime<FixedOffset>,
    timezone: &Tz,
) -> Result<(Vec<TaskRecord>, Vec<Warning>), Error> {
    let projects = match &q.project {
        Some(p) => {
            store.validate_project(p)?;
            vec![p.clone()]
        }
        None => store.discover_projects()?,
    };
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    for project in &projects {
        let (mut project_records, mut project_warnings) = store.load_project(project)?;
        records.append(&mut project_records);
        warnings.append(&mut project_warnings);
    }
    Ok((
        query::apply(records, &q.filter, q.sort, now, timezone),
        warnings,
    ))
}

/// Resolve one task and return it with the verbatim TOML file contents.
pub fn show(
    store: &Store,
    selector: &Selector,
    project: Option<&str>,
) -> Result<(Resolved, String), Error> {
    let resolved = resolve::resolve(store, selector, project)?;
    let raw = store.read_raw(&resolved.record.path)?;
    Ok((resolved, raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Status;
    use crate::resolve::{IdSelector, Selector};
    use crate::store::Store;
    use chrono::DateTime;

    fn dt(s: &str) -> chrono::DateTime<chrono::FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    fn now() -> chrono::DateTime<chrono::FixedOffset> {
        dt("2024-03-13T10:00:00+01:00")
    }

    fn later() -> chrono::DateTime<chrono::FixedOffset> {
        dt("2024-03-14T11:00:00+01:00")
    }

    fn wiki() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("proj")).unwrap();
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }

    fn new_task(title: &str) -> NewTask {
        NewTask {
            title: title.into(),
            description: None,
            due: None,
            tags: vec![],
            sources: vec![],
            subtasks: vec![],
        }
    }

    fn by_id(id: &str) -> Selector {
        Selector::Id(IdSelector::Task(id.into()))
    }

    fn by_sub(task: &str, sub: &str) -> Selector {
        Selector::Id(IdSelector::Compound {
            task: task.into(),
            subtask: sub.into(),
        })
    }

    #[test]
    fn unique_id_checks_an_explicit_hidden_project_omitted_by_discovery() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".hidden/proj")).unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store
            .save(
                ".hidden/proj",
                &Task {
                    id: "collid01".into(),
                    title: "Existing".into(),
                    status: Status::Open,
                    due: None,
                    tags: vec![],
                    source: vec![],
                    created: now(),
                    modified: now(),
                    related: vec![],
                    description: None,
                    tasks: vec![],
                },
            )
            .unwrap();
        assert!(store.discover_projects().unwrap().is_empty());
        let mut candidates = ["collid01", "freeid02"].into_iter();
        assert_eq!(
            unique_task_id_with(&store, ".hidden/proj", || candidates.next().unwrap().into())
                .unwrap(),
            "freeid02"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unique_id_checks_an_explicit_symlink_project_omitted_by_discovery() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        symlink(destination.path(), dir.path().join("linked")).unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store
            .save(
                "linked",
                &Task {
                    id: "collid01".into(),
                    title: "Existing".into(),
                    status: Status::Open,
                    due: None,
                    tags: vec![],
                    source: vec![],
                    created: now(),
                    modified: now(),
                    related: vec![],
                    description: None,
                    tasks: vec![],
                },
            )
            .unwrap();
        assert!(store.discover_projects().unwrap().is_empty());
        let mut candidates = ["collid01", "freeid02"].into_iter();
        assert_eq!(
            unique_task_id_with(&store, "linked", || candidates.next().unwrap().into()).unwrap(),
            "freeid02"
        );
    }

    #[test]
    fn add_creates_task_with_generated_id_and_timestamps() {
        let (_dir, store) = wiki();
        let created = add(&store, "proj", new_task("Review OKRs"), now()).unwrap();
        assert!(crate::ids::is_task_id_shaped(&created.id));
        assert_eq!(created.created, now());
        assert_eq!(created.modified, now());
        assert_eq!(created.status, Status::Open);
        let loaded = store.load("proj", &created.id).unwrap();
        assert_eq!(loaded.task, created);
    }

    #[test]
    fn add_generates_subtask_ids_and_dedups_tags() {
        let (_dir, store) = wiki();
        let created = add(
            &store,
            "proj",
            NewTask {
                tags: vec!["okr".into(), " okr ".into(), "review".into()],
                sources: vec!["meet-a".into(), "meet-a".into()],
                subtasks: vec!["First".into(), "Second".into()],
                ..new_task("With subs")
            },
            now(),
        )
        .unwrap();
        assert_eq!(created.tags, vec!["okr", "review"]);
        assert_eq!(created.source, vec!["meet-a"]);
        assert_eq!(created.tasks.len(), 2);
        assert_eq!(created.tasks[0].text, "First");
        assert!(!created.tasks[0].done);
        assert_eq!(created.tasks[0].id.len(), 4);
        assert_ne!(created.tasks[0].id, created.tasks[1].id);
    }

    #[test]
    fn add_rejects_empty_title_and_empty_tag() {
        let (_dir, store) = wiki();
        assert!(matches!(
            add(&store, "proj", new_task("   "), now()),
            Err(Error::InvalidField { field, .. }) if field == "title"
        ));
        assert!(matches!(
            add(&store, "proj", NewTask { tags: vec!["  ".into()], ..new_task("Ok") }, now()),
            Err(Error::InvalidField { field, .. }) if field == "tag"
        ));
    }

    #[test]
    fn add_rejects_unknown_project() {
        let (_dir, store) = wiki();
        assert!(matches!(
            add(&store, "ghost", new_task("X"), now()),
            Err(Error::ProjectNotFound(p)) if p == "ghost"
        ));
    }

    fn even_later() -> chrono::DateTime<chrono::FixedOffset> {
        dt("2024-03-15T12:00:00+01:00")
    }

    #[test]
    fn done_and_undone_are_idempotent_true_noops() {
        let (_dir, store) = wiki();
        let t = add(&store, "proj", new_task("Flip me"), now()).unwrap();
        set_done(&store, &by_id(&t.id), None, true, later()).unwrap();
        let after = store.load("proj", &t.id).unwrap().task;
        assert_eq!(after.status, Status::Done);
        assert_eq!(after.modified, later());
        // Retry with a newer clock: succeeds but rewrites NOTHING —
        // `modified` stays at the first transition (spec: idempotency).
        set_done(&store, &by_id(&t.id), None, true, even_later()).unwrap();
        assert_eq!(store.load("proj", &t.id).unwrap().task.modified, later());
        // undone flips it back and bumps modified again
        set_done(&store, &by_id(&t.id), None, false, even_later()).unwrap();
        let reopened = store.load("proj", &t.id).unwrap().task;
        assert_eq!(reopened.status, Status::Open);
        assert_eq!(reopened.modified, even_later());
    }

    #[test]
    fn subtask_done_flips_entry_only_parent_status_untouched() {
        let (_dir, store) = wiki();
        let t = add(
            &store,
            "proj",
            NewTask {
                subtasks: vec!["Sub".into()],
                ..new_task("Parent")
            },
            now(),
        )
        .unwrap();
        let sub_id = t.tasks[0].id.clone();
        let report = set_done(&store, &by_sub(&t.id, &sub_id), None, true, later()).unwrap();
        assert_eq!(report.subtask.as_ref().unwrap().id, sub_id);
        let after = store.load("proj", &t.id).unwrap().task;
        assert_eq!(after.status, Status::Open); // parent untouched
        assert!(after.tasks[0].done);
        assert_eq!(after.modified, later()); // parent modified bumped
        // retry is a true no-op for sub-tasks too
        set_done(&store, &by_sub(&t.id, &sub_id), None, true, even_later()).unwrap();
        assert_eq!(store.load("proj", &t.id).unwrap().task.modified, later());
    }

    #[test]
    fn edit_updates_fields_and_manages_tags_sources() {
        let (_dir, store) = wiki();
        let t = add(
            &store,
            "proj",
            NewTask {
                tags: vec!["old".into()],
                ..new_task("Before")
            },
            now(),
        )
        .unwrap();
        edit(
            &store,
            &by_id(&t.id),
            None,
            EditFields {
                title: Some("After".into()),
                description: Some("New desc".into()),
                due: Some(dt("2024-04-01T23:59:59+02:00")),
                add_tags: vec!["new".into(), "old".into()], // adding existing = no-op
                remove_tags: vec!["old".into(), "ghost".into()], // removing missing = no-op
                add_sources: vec!["src-1".into()],
                remove_sources: vec![],
            },
            later(),
        )
        .unwrap();
        let after = store.load("proj", &t.id).unwrap().task;
        assert_eq!(after.title, "After");
        assert_eq!(after.description.as_deref(), Some("New desc"));
        assert_eq!(after.due, Some(dt("2024-04-01T23:59:59+02:00")));
        assert_eq!(after.tags, vec!["new"]);
        assert_eq!(after.source, vec!["src-1"]);
        assert_eq!(after.modified, later());
    }

    #[test]
    fn edit_requires_at_least_one_field() {
        let (_dir, store) = wiki();
        let t = add(&store, "proj", new_task("X"), now()).unwrap();
        assert!(matches!(
            edit(&store, &by_id(&t.id), None, EditFields::default(), later()),
            Err(Error::EmptyEdit)
        ));
    }

    #[test]
    fn edit_on_subtask_renames_text_and_rejects_whole_task_flags() {
        let (_dir, store) = wiki();
        let t = add(
            &store,
            "proj",
            NewTask {
                subtasks: vec!["Old text".into()],
                ..new_task("P")
            },
            now(),
        )
        .unwrap();
        let sub_id = t.tasks[0].id.clone();
        edit(
            &store,
            &by_sub(&t.id, &sub_id),
            None,
            EditFields {
                title: Some("New text".into()),
                ..EditFields::default()
            },
            later(),
        )
        .unwrap();
        assert_eq!(
            store.load("proj", &t.id).unwrap().task.tasks[0].text,
            "New text"
        );

        let err = edit(
            &store,
            &by_sub(&t.id, &sub_id),
            None,
            EditFields {
                due: Some(dt("2024-04-01T23:59:59+02:00")),
                ..EditFields::default()
            },
            later(),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "--due is not valid when editing a sub-task"
        );
    }

    #[test]
    fn edit_subtask_rejects_remove_values_before_validating_them() {
        let (_dir, store) = wiki();
        let task = add(
            &store,
            "proj",
            NewTask {
                subtasks: vec!["Subtask".into()],
                ..new_task("Parent")
            },
            now(),
        )
        .unwrap();
        let subtask = task.tasks[0].id.clone();

        for (fields, expected) in [
            (
                EditFields {
                    remove_tags: vec![" ".into()],
                    ..EditFields::default()
                },
                "--remove-tag is not valid when editing a sub-task",
            ),
            (
                EditFields {
                    remove_sources: vec![" ".into()],
                    ..EditFields::default()
                },
                "--remove-source is not valid when editing a sub-task",
            ),
        ] {
            let err = edit(&store, &by_sub(&task.id, &subtask), None, fields, later()).unwrap_err();
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn edit_resolved_keeps_warnings_on_subtask_preflight_error() {
        let (_dir, store) = wiki();
        let task = add(
            &store,
            "proj",
            NewTask {
                subtasks: vec!["Subtask".into()],
                ..new_task("Parent")
            },
            now(),
        )
        .unwrap();
        let mut resolved = find(&store, &by_sub(&task.id, &task.tasks[0].id), None).unwrap();
        resolved.warnings.push(Warning("prior warning".into()));

        let err = edit_resolved(
            &store,
            resolved,
            EditFields {
                remove_tags: vec![" ".into()],
                ..EditFields::default()
            },
            later(),
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "--remove-tag is not valid when editing a sub-task"
        );
        assert_eq!(err.warnings()[0].to_string(), "prior warning");
    }

    #[test]
    fn edit_resolved_keeps_warnings_on_whole_task_validation_error() {
        let (_dir, store) = wiki();
        let task = add(&store, "proj", new_task("Parent"), now()).unwrap();
        let mut resolved = find(&store, &by_id(&task.id), None).unwrap();
        resolved.warnings.push(Warning("prior warning".into()));

        let err = edit_resolved(
            &store,
            resolved,
            EditFields {
                add_tags: vec![" ".into()],
                ..EditFields::default()
            },
            later(),
        )
        .unwrap_err();

        assert_eq!(err.to_string(), "invalid tag: must not be empty");
        assert_eq!(err.warnings()[0].to_string(), "prior warning");
    }

    #[test]
    fn remove_whole_task_deletes_file() {
        let (_dir, store) = wiki();
        let t = add(&store, "proj", new_task("Doomed"), now()).unwrap();
        let resolved = find(&store, &by_id(&t.id), None).unwrap();
        remove(&store, &resolved, later()).unwrap();
        assert!(!store.task_exists("proj", &t.id).unwrap());
    }

    #[test]
    fn remove_subtask_keeps_parent_and_bumps_modified() {
        let (_dir, store) = wiki();
        let t = add(
            &store,
            "proj",
            NewTask {
                subtasks: vec!["Sub".into()],
                ..new_task("P")
            },
            now(),
        )
        .unwrap();
        let sub_id = t.tasks[0].id.clone();
        let resolved = find(&store, &by_sub(&t.id, &sub_id), None).unwrap();
        remove(&store, &resolved, later()).unwrap();
        let after = store.load("proj", &t.id).unwrap().task;
        assert!(after.tasks.is_empty());
        assert_eq!(after.modified, later());
    }

    use crate::query::{Filter, StatusFilter};

    #[test]
    fn list_spans_all_projects_and_respects_project_scope() {
        let (dir, store) = wiki();
        std::fs::create_dir_all(dir.path().join("other")).unwrap();
        add(&store, "proj", new_task("In proj"), now()).unwrap();
        add(&store, "other", new_task("In other"), now()).unwrap();

        let (all, warnings) = list(&store, &ListQuery::default(), now(), &chrono::Local).unwrap();
        assert_eq!(all.len(), 2);
        assert!(warnings.is_empty());

        let scoped_query = ListQuery {
            project: Some("other".into()),
            ..ListQuery::default()
        };
        let (scoped, _) = list(&store, &scoped_query, now(), &chrono::Local).unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].task.title, "In other");
    }

    #[test]
    fn list_applies_filter_and_propagates_warnings() {
        let (dir, store) = wiki();
        let t = add(&store, "proj", new_task("Open task"), now()).unwrap();
        add(&store, "proj", new_task("Done task"), now())
            .and_then(|d| set_done(&store, &by_id(&d.id), None, true, now()))
            .unwrap();
        std::fs::write(dir.path().join("proj/.ruwana/broken12.toml"), "x = [").unwrap();

        let (open_only, warnings) =
            list(&store, &ListQuery::default(), now(), &chrono::Local).unwrap();
        assert_eq!(open_only.len(), 1);
        assert_eq!(open_only[0].task.id, t.id);
        assert_eq!(warnings.len(), 1);

        let done_query = ListQuery {
            filter: Filter {
                status: StatusFilter::Done,
                ..Filter::default()
            },
            ..ListQuery::default()
        };
        let (done_only, _) = list(&store, &done_query, now(), &chrono::Local).unwrap();
        assert_eq!(done_only.len(), 1);
        assert_eq!(done_only[0].task.title, "Done task");
    }

    #[test]
    fn list_on_invalid_project_errors() {
        let (_dir, store) = wiki();
        let q = ListQuery {
            project: Some("ghost".into()),
            ..ListQuery::default()
        };
        assert!(matches!(
            list(&store, &q, now(), &chrono::Local),
            Err(Error::ProjectNotFound(_))
        ));
    }

    #[test]
    fn show_returns_resolved_record_and_verbatim_toml() {
        let (_dir, store) = wiki();
        let t = add(&store, "proj", new_task("Show me"), now()).unwrap();
        let (resolved, raw) = show(&store, &by_id(&t.id), None).unwrap();
        assert_eq!(resolved.record.task.id, t.id);
        assert_eq!(raw, std::fs::read_to_string(&resolved.record.path).unwrap());
        assert!(raw.contains("title = \"Show me\""));
    }
}
