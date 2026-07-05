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

#[test]
fn list_json_has_the_spec_schema() {
    let wiki = wiki_with(&["proj"]);
    let id = add_task(
        wiki.path(),
        "proj",
        "Json me",
        &["--tag", "okr", "--source", "meet-a", "--due", "2030-06-01"],
    );
    let plain = add_task(wiki.path(), "proj", "No extras", &[]);

    let output = ruwana(wiki.path()).args(["list", "--format", "json"]).output().unwrap();
    assert!(output.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks = parsed.as_array().unwrap();
    assert_eq!(tasks.len(), 2);

    let rich = tasks.iter().find(|t| t["id"] == id.as_str()).unwrap();
    assert_eq!(rich["project"], "proj");
    assert_eq!(rich["title"], "Json me");
    assert_eq!(rich["status"], "open");
    assert_eq!(rich["tags"], serde_json::json!(["okr"]));
    assert_eq!(rich["source"], serde_json::json!(["meet-a"]));
    let due = rich["due"].as_str().unwrap();
    assert!(due.starts_with("2030-06-01T23:59:59"), "due was {due}");
    assert!(rich["created"].as_str().unwrap().contains('T'));

    let bare = tasks.iter().find(|t| t["id"] == plain.as_str()).unwrap();
    assert!(bare["due"].is_null());
    assert_eq!(bare["tags"], serde_json::json!([]));
    // list JSON excludes the heavy fields
    assert!(bare.get("description").is_none());
    assert!(bare.get("tasks").is_none());
}

#[test]
fn empty_list_json_is_an_empty_array() {
    let wiki = wiki_with(&["proj"]);
    ruwana(wiki.path())
        .args(["list", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::diff("[]\n"));
}

#[test]
fn show_json_includes_full_record() {
    let wiki = wiki_with(&["proj"]);
    let id = add_task(
        wiki.path(),
        "proj",
        "Full record",
        &["--description", "The details", "--task", "Sub one"],
    );
    let output = ruwana(wiki.path()).args(["show", &id, "--format", "json"]).output().unwrap();
    assert!(output.status.success());
    let t: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(t["id"], id.as_str());
    assert_eq!(t["description"], "The details");
    assert_eq!(t["related"], serde_json::json!([]));
    assert_eq!(t["tasks"][0]["text"], "Sub one");
    assert_eq!(t["tasks"][0]["done"], false);
    assert_eq!(t["tasks"][0]["id"].as_str().unwrap().len(), 4);
    assert!(t["file_path"].as_str().unwrap().ends_with(&format!("{id}.toml")));
}
