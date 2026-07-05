use assert_cmd::Command;
use predicates::prelude::*;

/// A ruwana command against a fresh temp wiki. TZ pinned so due-date
/// rendering never depends on the host timezone.
fn ruwana(wiki: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("ruwana").unwrap();
    cmd.env("WIKI_ROOT", wiki).env("TZ", "Europe/Berlin");
    cmd
}

fn wiki_with(projects: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for p in projects {
        std::fs::create_dir_all(dir.path().join(p)).unwrap();
    }
    dir
}

/// `add` prints only the new task ID; capture it.
fn add_task(wiki: &std::path::Path, project: &str, title: &str, extra: &[&str]) -> String {
    let output = ruwana(wiki)
        .args(["add", "--project", project, title])
        .args(extra)
        .output()
        .unwrap();
    assert!(output.status.success(), "add failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn add_prints_id_and_list_shows_the_task() {
    let wiki = wiki_with(&["godel/ai-practice"]);
    let id = add_task(wiki.path(), "godel/ai-practice", "Review Q3 OKRs", &[]);
    assert_eq!(id.len(), 8);

    ruwana(wiki.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains(&id))
        .stdout(predicate::str::contains("godel/ai-practice"))
        .stdout(predicate::str::contains("Review Q3 OKRs"));
}

#[test]
fn done_confirms_and_default_list_hides_done_tasks() {
    let wiki = wiki_with(&["proj"]);
    let id = add_task(wiki.path(), "proj", "Finish me", &[]);

    ruwana(wiki.path())
        .args(["done", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains(&id))
        .stdout(predicate::str::contains("Finish me"));

    ruwana(wiki.path()).arg("list").assert().success().stdout(predicate::str::is_empty());
    ruwana(wiki.path())
        .args(["list", "--done"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Finish me"));
}

#[test]
fn unknown_project_is_exit_1_with_spec_message() {
    let wiki = wiki_with(&[]);
    ruwana(wiki.path())
        .args(["add", "--project", "ghost", "X"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("project path not found: ghost"));
}

#[test]
fn aliases_work() {
    let wiki = wiki_with(&["proj"]);
    // add → a, list → ls, done → do
    ruwana(wiki.path()).args(["a", "--project", "proj", "Via alias"]).assert().success();
    ruwana(wiki.path()).arg("ls").assert().success().stdout(predicate::str::contains("Via alias"));
    ruwana(wiki.path()).args(["do", "Via alias"]).assert().success();
}
