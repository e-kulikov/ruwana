use crate::Error;
use crate::ids::{is_subtask_id_shaped, is_task_id_shaped};
use crate::store::{Store, TaskRecord, Warning};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdSelector {
    Task(String),
    Compound { task: String, subtask: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// The positional `<id-or-title>` argument.
    IdOrTitle(String),
    /// The `--id` flag (already syntax-validated).
    Id(IdSelector),
}

#[derive(Debug)]
pub struct Resolved {
    pub record: TaskRecord,
    /// Set iff the selector addressed a sub-task (compound `--id`).
    pub subtask_id: Option<String>,
    /// Skip-warnings collected while scanning projects for a title.
    pub warnings: Vec<Warning>,
}

impl Resolved {
    /// Preserve scan warnings when a caller rejects an already-resolved
    /// target before performing its mutation.
    pub fn into_error(self, error: Error) -> Error {
        Error::with_warnings(error, self.warnings)
    }
}

/// Validate `--id` syntax: `<task-id>` or `<task-id>:<subtask-id>`.
pub fn parse_id_selector(input: &str) -> Result<IdSelector, Error> {
    match input.split_once(':') {
        None if is_task_id_shaped(input) => Ok(IdSelector::Task(input.to_string())),
        Some((task, subtask)) if is_task_id_shaped(task) && is_subtask_id_shaped(subtask) => {
            Ok(IdSelector::Compound {
                task: task.to_string(),
                subtask: subtask.to_string(),
            })
        }
        _ => Err(Error::MalformedIdSelector),
    }
}

/// Projects to search: the validated `--project`, or all discovered ones.
fn candidate_projects(store: &Store, project: Option<&str>) -> Result<Vec<String>, Error> {
    match project {
        Some(p) => {
            store.validate_project(p)?;
            Ok(vec![p.to_string()])
        }
        None => store.discover_projects(),
    }
}

/// ID lookup: the ID is the filename, so this is a stat per project
/// (spec: Resolution Rules step 1). Loads only on hit.
fn find_by_id(store: &Store, projects: &[String], id: &str) -> Result<Option<TaskRecord>, Error> {
    let hits: Vec<_> = projects
        .iter()
        .filter_map(|project| match store.task_exists(project, id) {
            Ok(true) => Some(Ok(project)),
            Ok(false) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<_, _>>()?;
    match hits.as_slice() {
        [] => Ok(None),
        [project] => store.load(project, id).map(Some),
        _ => Err(Error::AmbiguousId {
            id: id.to_string(),
            projects: hits
                .iter()
                .map(|project| project.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

/// Title lookup: exact, case-sensitive (spec: Resolution Rules step 2).
fn find_by_title(
    store: &Store,
    projects: &[String],
    title: &str,
) -> Result<(Vec<TaskRecord>, Vec<Warning>), Error> {
    let mut hits = Vec::new();
    let mut warnings = Vec::new();
    for project in projects {
        let (records, mut project_warnings) = match store.load_project(project) {
            Ok(loaded) => loaded,
            Err(error) => return Err(Error::with_warnings(error, warnings)),
        };
        warnings.append(&mut project_warnings);
        hits.extend(records.into_iter().filter(|r| r.task.title == title));
    }
    Ok((hits, warnings))
}

fn require_subtask(record: &TaskRecord, subtask: &str) -> Result<(), Error> {
    if record.task.tasks.iter().any(|s| s.id == subtask) {
        return Ok(());
    }
    Err(Error::SubtaskNotFound {
        subtask: subtask.to_string(),
        task: record.task.id.clone(),
    })
}

/// Resolve any command target to a concrete task (and optional sub-task).
pub fn resolve(
    store: &Store,
    selector: &Selector,
    project: Option<&str>,
) -> Result<Resolved, Error> {
    let projects = candidate_projects(store, project)?;
    match selector {
        Selector::Id(IdSelector::Task(id)) => {
            let record =
                find_by_id(store, &projects, id)?.ok_or_else(|| Error::IdNotFound(id.clone()))?;
            Ok(Resolved {
                record,
                subtask_id: None,
                warnings: Vec::new(),
            })
        }
        Selector::Id(IdSelector::Compound { task, subtask }) => {
            let record = find_by_id(store, &projects, task)?
                .ok_or_else(|| Error::IdNotFound(task.clone()))?;
            require_subtask(&record, subtask)?;
            Ok(Resolved {
                record,
                subtask_id: Some(subtask.clone()),
                warnings: Vec::new(),
            })
        }
        Selector::IdOrTitle(arg) => {
            #[allow(clippy::collapsible_if)]
            if is_task_id_shaped(arg) {
                if let Some(record) = find_by_id(store, &projects, arg)? {
                    return Ok(Resolved {
                        record,
                        subtask_id: None,
                        warnings: Vec::new(),
                    });
                }
                // ID-shaped but no such file: fall through to title match.
            }
            let (mut hits, warnings) = find_by_title(store, &projects, arg)?;
            match hits.len() {
                0 => Err(Error::with_warnings(
                    Error::TitleNotFound(arg.clone()),
                    warnings,
                )),
                1 => Ok(Resolved {
                    record: hits.remove(0),
                    subtask_id: None,
                    warnings,
                }),
                _ => {
                    let candidates = hits
                        .iter()
                        .map(|r| format!("  {}  {}  {}", r.task.id, r.project, r.task.title))
                        .collect::<Vec<_>>()
                        .join("\n");
                    Err(Error::with_warnings(
                        Error::AmbiguousTitle { candidates },
                        warnings,
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Status, SubTask, Task};
    use crate::store::Store;
    use chrono::DateTime;

    fn task(id: &str, title: &str, subtasks: Vec<SubTask>) -> Task {
        let t = DateTime::parse_from_rfc3339("2024-01-15T09:32:11+01:00").unwrap();
        Task {
            id: id.into(),
            title: title.into(),
            status: Status::Open,
            due: None,
            tags: vec![],
            source: vec![],
            created: t,
            modified: t,
            related: vec![],
            description: None,
            tasks: subtasks,
        }
    }

    fn sub(id: &str, text: &str) -> SubTask {
        SubTask {
            id: id.into(),
            text: text.into(),
            done: false,
        }
    }

    /// Two projects: alpha has "abc1de2f" (title "Unique title", one
    /// sub-task gh7f) and "dupdupd1" (title "Same title"); beta has
    /// "dupdupd2" (also "Same title").
    fn wiki() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("alpha")).unwrap();
        std::fs::create_dir_all(dir.path().join("beta")).unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store
            .save(
                "alpha",
                &task("abc1de2f", "Unique title", vec![sub("gh7f", "Sub one")]),
            )
            .unwrap();
        store
            .save("alpha", &task("dupdupd1", "Same title", vec![]))
            .unwrap();
        store
            .save("beta", &task("dupdupd2", "Same title", vec![]))
            .unwrap();
        store
            .save(
                "beta",
                &task("abc1de2f", "Duplicate ID", vec![sub("gh7f", "Sub one")]),
            )
            .unwrap();
        (dir, store)
    }

    #[test]
    fn parse_id_selector_accepts_plain_and_compound() {
        assert_eq!(
            parse_id_selector("abc1de2f").unwrap(),
            IdSelector::Task("abc1de2f".into())
        );
        assert_eq!(
            parse_id_selector("abc1de2f:gh7f").unwrap(),
            IdSelector::Compound {
                task: "abc1de2f".into(),
                subtask: "gh7f".into()
            }
        );
    }

    #[test]
    fn parse_id_selector_rejects_malformed() {
        for bad in [
            "short",
            "abc1de2f:",
            ":gh7f",
            "abc1de2f:toolong7",
            "abc1de2f:GH7F",
            "a:b:c",
            "ABC1DE2F",
        ] {
            assert!(
                matches!(parse_id_selector(bad), Err(Error::MalformedIdSelector)),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn resolves_id_without_project_across_projects() {
        let (_dir, store) = wiki();
        let hit = resolve(&store, &Selector::IdOrTitle("dupdupd2".into()), None).unwrap();
        assert_eq!(hit.record.project, "beta");
        assert!(hit.subtask_id.is_none());
    }

    #[test]
    fn global_duplicate_ids_are_ambiguous_for_every_id_selector() {
        let (_dir, store) = wiki();
        for selector in [
            Selector::Id(IdSelector::Task("abc1de2f".into())),
            Selector::IdOrTitle("abc1de2f".into()),
            Selector::Id(IdSelector::Compound {
                task: "abc1de2f".into(),
                subtask: "gh7f".into(),
            }),
        ] {
            let err = resolve(&store, &selector, None).unwrap_err();
            assert_eq!(
                err.to_string(),
                "ambiguous task id; use --project to narrow: abc1de2f (alpha, beta)"
            );
        }
        let scoped = resolve(
            &store,
            &Selector::Id(IdSelector::Task("abc1de2f".into())),
            Some("alpha"),
        )
        .unwrap();
        assert_eq!(scoped.record.project, "alpha");
    }

    #[test]
    fn resolves_id_scoped_to_project() {
        let (_dir, store) = wiki();
        let hit = resolve(
            &store,
            &Selector::IdOrTitle("abc1de2f".into()),
            Some("alpha"),
        )
        .unwrap();
        assert_eq!(hit.record.task.title, "Unique title");
    }

    #[test]
    fn id_shaped_but_missing_falls_through_to_title_then_errors() {
        let (_dir, store) = wiki();
        // 8 chars of [a-z0-9], no task file, no task titled "zzzzzzzz"
        let err = resolve(&store, &Selector::IdOrTitle("zzzzzzzz".into()), None).unwrap_err();
        assert!(matches!(err, Error::TitleNotFound(t) if t == "zzzzzzzz"));
    }

    #[test]
    fn unique_title_resolves_without_project() {
        let (_dir, store) = wiki();
        let hit = resolve(&store, &Selector::IdOrTitle("Unique title".into()), None).unwrap();
        assert_eq!(hit.record.task.id, "abc1de2f");
    }

    #[test]
    fn title_match_is_exact_and_case_sensitive() {
        let (_dir, store) = wiki();
        assert!(resolve(&store, &Selector::IdOrTitle("unique title".into()), None).is_err());
        assert!(resolve(&store, &Selector::IdOrTitle("Unique".into()), None).is_err());
    }

    #[test]
    fn ambiguous_title_errors_and_project_narrows_it() {
        let (_dir, store) = wiki();
        let err = resolve(&store, &Selector::IdOrTitle("Same title".into()), None).unwrap_err();
        match err {
            Error::AmbiguousTitle { candidates } => {
                assert!(candidates.contains("dupdupd1"));
                assert!(candidates.contains("dupdupd2"));
            }
            other => panic!("expected AmbiguousTitle, got {other:?}"),
        }
        let hit = resolve(
            &store,
            &Selector::IdOrTitle("Same title".into()),
            Some("beta"),
        )
        .unwrap();
        assert_eq!(hit.record.task.id, "dupdupd2");
    }

    #[test]
    fn title_resolution_keeps_earlier_project_warnings_on_later_fatal_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let alpha = dir.path().join("alpha/.ruwana");
        let beta = dir.path().join("beta/.ruwana");
        std::fs::create_dir_all(&alpha).unwrap();
        std::fs::create_dir_all(&beta).unwrap();
        std::fs::write(alpha.join("aaaaaaa1.toml"), "x = [").unwrap();
        std::fs::write(beta.join("aaaaaaa2.toml"), "x = [").unwrap();
        std::fs::create_dir(beta.join("zzzzzzz2.toml")).unwrap();

        let err = resolve(&store, &Selector::IdOrTitle("Missing title".into()), None).unwrap_err();

        assert_eq!(err.warnings().len(), 2);
        assert!(
            err.warnings()[0]
                .to_string()
                .contains(&alpha.join("aaaaaaa1.toml").display().to_string())
        );
        assert!(
            err.warnings()[1]
                .to_string()
                .contains(&beta.join("aaaaaaa2.toml").display().to_string())
        );
        assert!(matches!(err, Error::Resolution { error, .. } if matches!(*error, Error::Io(_))));
    }

    #[test]
    fn compound_id_resolves_subtask() {
        let (_dir, store) = wiki();
        let sel = Selector::Id(IdSelector::Compound {
            task: "abc1de2f".into(),
            subtask: "gh7f".into(),
        });
        let hit = resolve(&store, &sel, Some("alpha")).unwrap();
        assert_eq!(hit.subtask_id.as_deref(), Some("gh7f"));
    }

    #[test]
    fn compound_id_with_unknown_subtask_errors() {
        let (_dir, store) = wiki();
        let sel = Selector::Id(IdSelector::Compound {
            task: "abc1de2f".into(),
            subtask: "zzzz".into(),
        });
        let err = resolve(&store, &sel, Some("alpha")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "no sub-task found with id: zzzz in task abc1de2f"
        );
    }

    #[test]
    fn explicit_id_selector_never_falls_back_to_title() {
        let (_dir, store) = wiki();
        let sel = Selector::Id(IdSelector::Task("zzzzzzzz".into()));
        assert!(
            matches!(resolve(&store, &sel, None), Err(Error::IdNotFound(id)) if id == "zzzzzzzz")
        );
    }
}
