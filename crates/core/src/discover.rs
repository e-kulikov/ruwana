use std::path::Path;
use walkdir::WalkDir;

/// Walk WIKI_ROOT for `.ruwana` directories (spec: Project discovery).
/// Rules: skip hidden dirs during descent except `.ruwana` itself; never
/// descend into a `.ruwana`; don't follow symlinks. Filesystem failures
/// propagate so global commands cannot present a broken root as empty.
pub fn discover_projects(wiki_root: &Path) -> std::io::Result<Vec<String>> {
    let root_metadata = std::fs::metadata(wiki_root)?;
    if !root_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!("WIKI_ROOT is not a directory: {}", wiki_root.display()),
        ));
    }
    let mut projects = Vec::new();
    let mut walker = WalkDir::new(wiki_root).follow_links(false).into_iter();
    loop {
        let entry = match walker.next() {
            None => break,
            Some(entry) => entry.map_err(std::io::Error::other)?,
        };
        if entry.depth() == 0 || !entry.file_type().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name == ".ruwana" {
            if let Some(rel) = entry
                .path()
                .parent()
                .and_then(|parent| parent.strip_prefix(wiki_root).ok())
            {
                let project = rel.to_string_lossy().replace('\\', "/");
                if !project.is_empty() {
                    projects.push(project);
                }
            }
            walker.skip_current_dir();
        } else if name.starts_with('.') {
            walker.skip_current_dir();
        }
    }
    projects.sort();
    Ok(projects)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(root: &std::path::Path, rel: &str) {
        std::fs::create_dir_all(root.join(rel)).unwrap();
    }

    #[test]
    fn finds_ruwana_dirs_at_any_depth_sorted() {
        let dir = tempfile::tempdir().unwrap();
        mk(dir.path(), "books/.ruwana");
        mk(dir.path(), "godel/ai-practice/.ruwana");
        mk(dir.path(), "godel/other"); // no .ruwana → not a project
        let projects = discover_projects(dir.path()).unwrap();
        assert_eq!(
            projects,
            vec!["books".to_string(), "godel/ai-practice".to_string()]
        );
    }

    #[test]
    fn skips_hidden_directories_during_descent() {
        let dir = tempfile::tempdir().unwrap();
        mk(dir.path(), ".git/objects/fake/.ruwana"); // inside hidden dir → invisible
        mk(dir.path(), "real/.ruwana");
        assert_eq!(
            discover_projects(dir.path()).unwrap(),
            vec!["real".to_string()]
        );
    }

    #[test]
    fn does_not_descend_into_ruwana_itself() {
        let dir = tempfile::tempdir().unwrap();
        mk(dir.path(), "proj/.ruwana/nested/.ruwana");
        assert_eq!(
            discover_projects(dir.path()).unwrap(),
            vec!["proj".to_string()]
        );
    }

    #[test]
    fn a_plain_file_named_ruwana_is_not_a_project() {
        let dir = tempfile::tempdir().unwrap();
        mk(dir.path(), "proj");
        std::fs::write(dir.path().join("proj/.ruwana"), "").unwrap();
        assert!(discover_projects(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn empty_wiki_yields_no_projects() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover_projects(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn missing_wiki_root_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover_projects(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn file_wiki_root_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("not-a-directory");
        std::fs::write(&root, "not a wiki").unwrap();
        assert!(discover_projects(&root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn global_discovery_skips_symlinked_projects_but_explicit_paths_can_use_them() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        mk(dir.path(), "real/.ruwana");
        symlink(dir.path().join("real"), dir.path().join("linked")).unwrap();
        assert_eq!(discover_projects(dir.path()).unwrap(), vec!["real"]);
        let store = crate::store::Store::new(dir.path().to_path_buf());
        assert!(store.validate_project("linked").is_ok());
    }
}
