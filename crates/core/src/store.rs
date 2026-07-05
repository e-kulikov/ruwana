use crate::Error;
use crate::model::{self, Task};
use rayon::prelude::*;
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
        if !dir.is_dir() {
            return Err(Error::ProjectNotFound(project.to_string()));
        }
        Ok(dir)
    }

    fn ruwana_dir(&self, project: &str) -> PathBuf {
        self.wiki_root.join(project).join(".ruwana")
    }

    pub fn task_path(&self, project: &str, id: &str) -> PathBuf {
        self.ruwana_dir(project).join(format!("{id}.toml"))
    }

    /// ID lookup fast path: the ID is the filename, so existence is a stat.
    pub fn task_exists(&self, project: &str, id: &str) -> bool {
        self.task_path(project, id).is_file()
    }

    /// Atomic write: serialize to a uniquely named temp file, rename over
    /// the target. The pid in the temp name means two racing processes
    /// never share a temp path, so a race can't mix content — the loser
    /// simply replaces the winner wholesale (last-writer-wins, accepted
    /// by the spec's Concurrency section). Creates `.ruwana/` on demand.
    pub fn save(&self, project: &str, task: &Task) -> Result<PathBuf, Error> {
        let dir = self.ruwana_dir(project);
        std::fs::create_dir_all(&dir)?;
        let target = self.task_path(project, &task.id);
        let tmp = dir.join(format!("{}.toml.{}.tmp", task.id, std::process::id()));
        std::fs::write(&tmp, model::to_toml(task)?)?;
        std::fs::rename(&tmp, &target)?;
        Ok(target)
    }

    fn load_path(&self, project: &str, path: &Path) -> Result<TaskRecord, Error> {
        let text = std::fs::read_to_string(path)?;
        let task = model::from_toml(&text).map_err(|e| Error::TaskFileParse {
            path: path.display().to_string(),
            message: e.message().to_string(),
        })?;
        Ok(TaskRecord {
            task,
            project: project.to_string(),
            path: path.to_path_buf(),
        })
    }

    pub fn load(&self, project: &str, id: &str) -> Result<TaskRecord, Error> {
        let path = self.task_path(project, id);
        if !path.is_file() {
            return Err(Error::IdNotFound(id.to_string()));
        }
        self.load_path(project, &path)
    }

    pub fn delete(&self, project: &str, id: &str) -> Result<(), Error> {
        let path = self.task_path(project, id);
        if !path.is_file() {
            return Err(Error::IdNotFound(id.to_string()));
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    /// Read every task in one project. Parses in parallel (rayon).
    /// Unparseable files become Warnings, not errors (spec: Query Engine).
    pub fn load_project(&self, project: &str) -> Result<(Vec<TaskRecord>, Vec<Warning>), Error> {
        let dir = self.ruwana_dir(project);
        if !dir.is_dir() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
            .collect();
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
    pub fn read_raw(&self, path: &Path) -> Result<String, Error> {
        Ok(std::fs::read_to_string(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Status, Task};
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
    fn task_exists_is_a_stat_and_load_missing_is_id_not_found() {
        let (_dir, store) = wiki();
        assert!(!store.task_exists("proj/sub", "abc1de2f"));
        store.save("proj/sub", &task("abc1de2f", "Hello")).unwrap();
        assert!(store.task_exists("proj/sub", "abc1de2f"));
        assert!(
            matches!(store.load("proj/sub", "zzzzzzzz"), Err(Error::IdNotFound(id)) if id == "zzzzzzzz")
        );
    }

    #[test]
    fn delete_removes_the_file() {
        let (_dir, store) = wiki();
        store.save("proj/sub", &task("abc1de2f", "Hello")).unwrap();
        store.delete("proj/sub", "abc1de2f").unwrap();
        assert!(!store.task_exists("proj/sub", "abc1de2f"));
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
