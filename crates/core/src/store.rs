use crate::Error;
use crate::model::{self, Task};
use rayon::prelude::*;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// A task together with where it lives. `project` is the path segment
/// under WIKI_ROOT — derived from the file path, never stored in TOML.
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub task: Task,
    pub project: String,
    pub path: PathBuf,
}

/// A non-fatal notice for the CLI to print on stderr. Core never prints.
#[derive(Debug)]
pub struct Warning(pub String);

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// All filesystem access for task data goes through this type — it is the
/// seam behind which a read-through cache could later be added (spec:
/// Query Engine → Escape hatch).
pub struct Store {
    wiki_root: PathBuf,
}

impl Store {
    pub fn new(wiki_root: PathBuf) -> Self {
        Self { wiki_root }
    }

    pub fn wiki_root(&self) -> &Path {
        &self.wiki_root
    }

    pub fn discover_projects(&self) -> Result<Vec<String>, Error> {
        Ok(crate::discover::discover_projects(&self.wiki_root)?)
    }

    /// Enforce the spec's project-path rules: relative, no `..`, and the
    /// directory must exist under WIKI_ROOT. Returns the project dir.
    /// Checks are lexical only — symlinks inside the wiki are trusted and
    /// followed (deliberate spec decision; do not canonicalize).
    pub fn validate_project(&self, project: &str) -> Result<PathBuf, Error> {
        let rel = Path::new(project);
        let clean = !project.is_empty()
            && rel.is_relative()
            && rel.components().all(|c| matches!(c, Component::Normal(_)));
        if !clean {
            return Err(Error::InvalidProjectPath);
        }
        let dir = self.wiki_root.join(rel);
        match std::fs::metadata(&dir) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(Error::ProjectNotFound(project.to_string()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::ProjectNotFound(project.to_string()));
            }
            Err(error) => return Err(Error::Io(error)),
        }
        Ok(dir)
    }

    fn ruwana_dir(&self, project: &str) -> PathBuf {
        self.wiki_root.join(project).join(".ruwana")
    }

    fn task_path(&self, project: &str, id: &str) -> PathBuf {
        self.ruwana_dir(project).join(format!("{id}.toml"))
    }

    fn validate_task_id(&self, id: &str) -> Result<(), Error> {
        if crate::ids::is_task_id_shaped(id) {
            Ok(())
        } else {
            Err(Error::InvalidField {
                field: "task id".into(),
                reason: "must be 8 lowercase letters or digits".into(),
            })
        }
    }

    fn checked_task_path(&self, project: &str, id: &str) -> Result<PathBuf, Error> {
        self.validate_project(project)?;
        self.validate_task_id(id)?;
        Ok(self.task_path(project, id))
    }

    fn task_file_exists(&self, path: &Path) -> Result<bool, Error> {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(Error::Io(error)),
        }
    }

    /// ID lookup fast path: the ID is the filename, so existence is a stat.
    pub fn task_exists(&self, project: &str, id: &str) -> Result<bool, Error> {
        self.task_file_exists(&self.checked_task_path(project, id)?)
    }

    /// Atomic write: serialize to an exclusively-created same-directory
    /// temporary file, then replace the target. Racing saves retain the
    /// spec's accepted last-writer-wins behavior without sharing a temp
    /// path. Creates `.ruwana/` on demand.
    pub fn save(&self, project: &str, task: &Task) -> Result<PathBuf, Error> {
        self.validate_project(project)?;
        model::validate(task).map_err(|reason| Error::InvalidField {
            field: "task".into(),
            reason,
        })?;
        let dir = self.ruwana_dir(project);
        std::fs::create_dir_all(&dir)?;
        let target = self.task_path(project, &task.id);
        let mut tmp = tempfile::Builder::new()
            .prefix(&format!("{}.toml.", task.id))
            .suffix(".tmp")
            .tempfile_in(&dir)?;
        tmp.write_all(model::to_toml(task)?.as_bytes())?;
        tmp.persist(&target).map_err(|err| Error::Io(err.error))?;
        Ok(target)
    }

    fn load_path(&self, project: &str, path: &Path) -> Result<TaskRecord, Error> {
        let text = std::fs::read_to_string(path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::InvalidData {
                Error::TaskFileParse {
                    path: path.display().to_string(),
                    message: err.to_string(),
                }
            } else {
                Error::Io(err)
            }
        })?;
        let task = model::from_toml(&text).map_err(|e| Error::TaskFileParse {
            path: path.display().to_string(),
            message: e.message().to_string(),
        })?;
        model::validate(&task).map_err(|message| Error::TaskFileParse {
            path: path.display().to_string(),
            message,
        })?;
        // Filename-is-ID is a storage invariant (spec: Storage Layout); a
        // mismatch is corruption, reported exactly like unparseable TOML —
        // multi-task reads skip it with a warning, direct targets fail loudly.
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem != task.id {
            return Err(Error::TaskFileParse {
                path: path.display().to_string(),
                message: format!("task id {:?} does not match filename", task.id),
            });
        }
        Ok(TaskRecord {
            task,
            project: project.to_string(),
            path: path.to_path_buf(),
        })
    }

    pub fn load(&self, project: &str, id: &str) -> Result<TaskRecord, Error> {
        let path = self.checked_task_path(project, id)?;
        if !self.task_file_exists(&path)? {
            return Err(Error::IdNotFound(id.to_string()));
        }
        self.load_path(project, &path)
    }

    pub fn delete(&self, project: &str, id: &str) -> Result<(), Error> {
        let path = self.checked_task_path(project, id)?;
        if !self.task_file_exists(&path)? {
            return Err(Error::IdNotFound(id.to_string()));
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    /// Read every task in one project. Parses in parallel (rayon).
    /// Unparseable files become Warnings, not errors (spec: Query Engine).
    pub fn load_project(&self, project: &str) -> Result<(Vec<TaskRecord>, Vec<Warning>), Error> {
        self.validate_project(project)?;
        let dir = self.ruwana_dir(project);
        match std::fs::metadata(&dir) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Ok((Vec::new(), Vec::new())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), Vec::new()));
            }
            Err(error) => return Err(Error::Io(error)),
        }
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                paths.push(path);
            }
        }
        paths.sort();
        let results: Vec<Result<TaskRecord, Error>> = paths
            .par_iter()
            .map(|p| self.load_path(project, p))
            .collect();
        let mut records = Vec::new();
        let mut warnings = Vec::new();
        for result in results {
            match result {
                Ok(rec) => records.push(rec),
                Err(Error::TaskFileParse { path, message }) => warnings.push(Warning(format!(
                    "skipping unparseable task file: {path}: {message}"
                ))),
                Err(other) => return Err(other),
            }
        }
        Ok((records, warnings))
    }

    /// Verbatim file contents (for `show` text output). Kept on Store so
    /// no other module touches the filesystem.
    pub(crate) fn read_raw(&self, path: &Path) -> Result<String, Error> {
        Ok(std::fs::read_to_string(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Status, SubTask, Task};
    use chrono::DateTime;

    fn task(id: &str, title: &str) -> Task {
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
            tasks: vec![],
        }
    }

    /// Temp wiki with one project dir `proj/sub` (no .ruwana yet).
    fn wiki() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("proj/sub")).unwrap();
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }

    #[test]
    fn validate_project_accepts_existing_relative_path() {
        let (_dir, store) = wiki();
        assert!(store.validate_project("proj/sub").is_ok());
    }

    #[test]
    fn validate_project_rejects_missing_absolute_and_dotdot() {
        let (_dir, store) = wiki();
        assert!(
            matches!(store.validate_project("nope"), Err(Error::ProjectNotFound(p)) if p == "nope")
        );
        assert!(matches!(
            store.validate_project("/etc"),
            Err(Error::InvalidProjectPath)
        ));
        assert!(matches!(
            store.validate_project("proj/../proj/sub"),
            Err(Error::InvalidProjectPath)
        ));
    }

    #[test]
    fn public_mutations_reject_escaping_projects_and_malformed_ids() {
        let (_dir, store) = wiki();
        assert!(matches!(
            store.save("../proj/sub", &task("abc1de2f", "Escape")),
            Err(Error::InvalidProjectPath)
        ));
        assert!(matches!(
            store.save("proj/sub", &task("../escape", "Bad ID")),
            Err(Error::InvalidField { .. })
        ));
        assert!(matches!(
            store.delete("/tmp", "abc1de2f"),
            Err(Error::InvalidProjectPath)
        ));
    }

    #[test]
    fn save_load_round_trip_creates_ruwana_dir() {
        let (dir, store) = wiki();
        let path = store.save("proj/sub", &task("abc1de2f", "Hello")).unwrap();
        assert_eq!(path, dir.path().join("proj/sub/.ruwana/abc1de2f.toml"));
        let rec = store.load("proj/sub", "abc1de2f").unwrap();
        assert_eq!(rec.task.title, "Hello");
        assert_eq!(rec.project, "proj/sub");
        assert_eq!(rec.path, path);
    }

    #[test]
    fn save_leaves_no_tmp_file_behind() {
        let (dir, store) = wiki();
        store.save("proj/sub", &task("abc1de2f", "Hello")).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("proj/sub/.ruwana"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "tmp files left: {leftovers:?}");
    }

    #[test]
    fn concurrent_saves_to_one_task_are_atomic_and_leave_no_temp_files() {
        let (dir, store) = wiki();
        let barrier = std::sync::Barrier::new(16);
        let results = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            let store = &store;
            for writer in 0..16 {
                let task = task("abc1de2f", &format!("Writer {writer}"));
                let barrier = &barrier;
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    store.save("proj/sub", &task)
                }));
            }
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });

        assert!(
            results.iter().all(Result::is_ok),
            "concurrent saves failed: {results:?}"
        );
        assert!(store.load("proj/sub", "abc1de2f").is_ok());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("proj/sub/.ruwana"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "tmp files left: {leftovers:?}");
    }

    #[test]
    fn task_exists_is_a_stat_and_load_missing_is_id_not_found() {
        let (_dir, store) = wiki();
        assert!(!store.task_exists("proj/sub", "abc1de2f").unwrap());
        store.save("proj/sub", &task("abc1de2f", "Hello")).unwrap();
        assert!(store.task_exists("proj/sub", "abc1de2f").unwrap());
        assert!(
            matches!(store.load("proj/sub", "zzzzzzzz"), Err(Error::IdNotFound(id)) if id == "zzzzzzzz")
        );
    }

    #[cfg(unix)]
    #[test]
    fn task_exists_propagates_unreadable_storage_metadata_errors() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, store) = wiki();
        store.save("proj/sub", &task("abc1de2f", "Hello")).unwrap();
        let task_dir = dir.path().join("proj/sub/.ruwana");
        std::fs::set_permissions(&task_dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = store.task_exists("proj/sub", "abc1de2f");
        std::fs::set_permissions(&task_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
    }

    #[test]
    fn delete_removes_the_file() {
        let (_dir, store) = wiki();
        store.save("proj/sub", &task("abc1de2f", "Hello")).unwrap();
        store.delete("proj/sub", "abc1de2f").unwrap();
        assert!(!store.task_exists("proj/sub", "abc1de2f").unwrap());
    }

    #[test]
    fn load_project_skips_unparseable_files_with_warning() {
        let (dir, store) = wiki();
        store
            .save("proj/sub", &task("abc1de2f", "Good one"))
            .unwrap();
        store
            .save("proj/sub", &task("bcd2ef3a", "Also good"))
            .unwrap();
        std::fs::write(
            dir.path().join("proj/sub/.ruwana/broken12.toml"),
            "not = valid = toml",
        )
        .unwrap();
        let (records, warnings) = store.load_project("proj/sub").unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(warnings.len(), 1);
        let w = warnings[0].to_string();
        assert!(w.starts_with("skipping unparseable task file: "), "{w}");
        assert!(w.contains("broken12.toml"), "{w}");
    }

    #[test]
    fn invalid_utf8_is_corruption_for_direct_and_project_reads() {
        let (dir, store) = wiki();
        store
            .save("proj/sub", &task("abc1de2f", "Good one"))
            .unwrap();
        std::fs::write(
            dir.path().join("proj/sub/.ruwana/badutf88.toml"),
            [0xff, 0xfe],
        )
        .unwrap();

        assert!(matches!(
            store.load("proj/sub", "badutf88"),
            Err(Error::TaskFileParse { .. })
        ));
        let (records, warnings) = store.load_project("proj/sub").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].to_string().contains("badutf88.toml"));
    }

    #[test]
    fn malformed_task_ids_and_duplicate_subtask_ids_are_corruption() {
        let (dir, store) = wiki();
        let mut invalid_outer = task("BAD1DE2F", "Bad outer ID");
        let mut invalid_subtask = task("abc1de2f", "Bad subtask ID");
        invalid_subtask.tasks = vec![SubTask {
            id: "abc".into(),
            text: "Too short".into(),
            done: false,
        }];
        let mut duplicate_subtasks = task("bcd2ef3a", "Duplicate subtasks");
        duplicate_subtasks.tasks = vec![
            SubTask {
                id: "abcd".into(),
                text: "First".into(),
                done: false,
            },
            SubTask {
                id: "abcd".into(),
                text: "Second".into(),
                done: false,
            },
        ];

        let malformed = [
            ("outerbad", &mut invalid_outer),
            ("abc1de2f", &mut invalid_subtask),
            ("bcd2ef3a", &mut duplicate_subtasks),
        ];
        let task_dir = dir.path().join("proj/sub/.ruwana");
        std::fs::create_dir_all(&task_dir).unwrap();
        for (id, task) in malformed {
            std::fs::write(
                task_dir.join(format!("{id}.toml")),
                model::to_toml(task).unwrap(),
            )
            .unwrap();
            assert!(matches!(
                store.load("proj/sub", id),
                Err(Error::TaskFileParse { .. })
            ));
        }

        let (records, warnings) = store.load_project("proj/sub").unwrap();
        assert!(records.is_empty());
        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn load_rejects_file_whose_id_does_not_match_filename() {
        let (dir, store) = wiki();
        // A perfectly valid task TOML — but saved under the wrong filename
        // (e.g. a hand-rename or bad merge). That's corruption.
        let mismatched = task("bbbbbbbb", "Wrong home");
        std::fs::create_dir_all(dir.path().join("proj/sub/.ruwana")).unwrap();
        std::fs::write(
            dir.path().join("proj/sub/.ruwana/aaaaaaaa.toml"),
            crate::model::to_toml(&mismatched).unwrap(),
        )
        .unwrap();

        // Direct target: loud failure with the invariant in the message.
        let err = store.load("proj/sub", "aaaaaaaa").unwrap_err();
        assert!(err.to_string().contains("does not match filename"), "{err}");

        // Multi-task read: skipped with a warning, like unparseable TOML.
        let (records, warnings) = store.load_project("proj/sub").unwrap();
        assert_eq!(records.len(), 0);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn load_project_on_project_without_ruwana_dir_is_empty() {
        let (_dir, store) = wiki();
        let (records, warnings) = store.load_project("proj/sub").unwrap();
        assert!(records.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_project_ignores_non_toml_files() {
        let (dir, store) = wiki();
        store
            .save("proj/sub", &task("abc1de2f", "Good one"))
            .unwrap();
        std::fs::write(dir.path().join("proj/sub/.ruwana/notes.txt"), "hi").unwrap();
        let (records, warnings) = store.load_project("proj/sub").unwrap();
        assert_eq!(records.len(), 1);
        assert!(warnings.is_empty());
    }
}
