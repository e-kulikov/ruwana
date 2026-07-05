use std::path::Path;
use walkdir::WalkDir;

/// Walk WIKI_ROOT for `.ruwana` directories (spec: Project discovery).
/// Rules: skip hidden dirs during descent except `.ruwana` itself; never
/// descend into a `.ruwana`; don't follow symlinks. Unreadable entries
/// are skipped silently — discovery is best-effort by design.
pub fn discover_projects(wiki_root: &Path) -> Vec<String> {
    let mut projects = Vec::new();
    let mut walker = WalkDir::new(wiki_root).follow_links(false).into_iter();
    loop {
        let entry = match walker.next() {
            None => break,
            Some(Err(_)) => continue,
            Some(Ok(entry)) => entry,
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
    projects
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
        let projects = discover_projects(dir.path());
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
        assert_eq!(discover_projects(dir.path()), vec!["real".to_string()]);
    }

    #[test]
    fn does_not_descend_into_ruwana_itself() {
        let dir = tempfile::tempdir().unwrap();
        mk(dir.path(), "proj/.ruwana/nested/.ruwana");
        assert_eq!(discover_projects(dir.path()), vec!["proj".to_string()]);
    }

    #[test]
    fn a_plain_file_named_ruwana_is_not_a_project() {
        let dir = tempfile::tempdir().unwrap();
        mk(dir.path(), "proj");
        std::fs::write(dir.path().join("proj/.ruwana"), "").unwrap();
        assert!(discover_projects(dir.path()).is_empty());
    }

    #[test]
    fn empty_wiki_yields_no_projects() {
        let dir = tempfile::tempdir().unwrap();
        assert!(discover_projects(dir.path()).is_empty());
    }
}
