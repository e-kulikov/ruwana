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
    assert!(
        output.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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

    ruwana(wiki.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
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
    ruwana(wiki.path())
        .args(["a", "--project", "proj", "Via alias"])
        .assert()
        .success();
    ruwana(wiki.path())
        .arg("ls")
        .assert()
        .success()
        .stdout(predicate::str::contains("Via alias"));
    ruwana(wiki.path())
        .args(["do", "Via alias"])
        .assert()
        .success();
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

    let output = ruwana(wiki.path())
        .args(["list", "--format", "json"])
        .output()
        .unwrap();
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
    let output = ruwana(wiki.path())
        .args(["show", &id, "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let t: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(t["id"], id.as_str());
    assert_eq!(t["description"], "The details");
    assert_eq!(t["related"], serde_json::json!([]));
    assert_eq!(t["tasks"][0]["text"], "Sub one");
    assert_eq!(t["tasks"][0]["done"], false);
    assert_eq!(t["tasks"][0]["id"].as_str().unwrap().len(), 4);
    assert!(
        t["file_path"]
            .as_str()
            .unwrap()
            .ends_with(&format!("{id}.toml"))
    );
}

#[test]
fn full_subtask_lifecycle() {
    let wiki = wiki_with(&["proj"]);
    let id = add_task(
        wiki.path(),
        "proj",
        "Parent",
        &["--task", "Sub one", "--task", "Sub two"],
    );

    // tasks: lists sub-task ids + state + text
    let output = ruwana(wiki.path()).args(["tasks", &id]).output().unwrap();
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout).unwrap();
    assert!(listing.contains("[ ]"), "{listing}");
    assert!(listing.contains("Sub one"));
    let sub_id = listing
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert_eq!(sub_id.len(), 4);

    // done on the sub-task: parent stays open
    let compound = format!("{id}:{sub_id}");
    ruwana(wiki.path())
        .args(["done", "--id", &compound])
        .assert()
        .success();
    ruwana(wiki.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Parent"));
    ruwana(wiki.path())
        .args(["tasks", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("[x]"));

    // edit renames the sub-task text; whole-task flag is rejected
    ruwana(wiki.path())
        .args(["edit", "--id", &compound, "--title", "Renamed sub"])
        .assert()
        .success();
    ruwana(wiki.path())
        .args(["edit", "--id", &compound, "--due", "tomorrow"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "--due is not valid when editing a sub-task",
        ));

    // rm the sub-task: parent file survives, the *other* sub-task remains
    // (each `--task` adds one independent `[[tasks]]`; removing one must
    // not affect its sibling).
    ruwana(wiki.path())
        .args(["rm", "--id", &compound, "--force"])
        .assert()
        .success();
    ruwana(wiki.path())
        .args(["tasks", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Sub two"))
        .stdout(predicate::str::contains("Renamed sub").not());
    ruwana(wiki.path()).args(["show", &id]).assert().success();
}

#[test]
fn show_prints_verbatim_toml() {
    let wiki = wiki_with(&["proj"]);
    let id = add_task(wiki.path(), "proj", "Verbatim", &[]);
    let file =
        std::fs::read_to_string(wiki.path().join(format!("proj/.ruwana/{id}.toml"))).unwrap();
    ruwana(wiki.path())
        .args(["show", &id])
        .assert()
        .success()
        .stdout(predicate::str::diff(file));
}

#[test]
fn undone_reopens_and_title_resolution_respects_project() {
    let wiki = wiki_with(&["alpha", "beta"]);
    add_task(wiki.path(), "alpha", "Shared name", &[]);
    add_task(wiki.path(), "beta", "Shared name", &[]);

    // ambiguous without --project, exact message
    ruwana(wiki.path())
        .args(["done", "Shared name"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "ambiguous title; use --project to narrow or use ID",
        ));

    // scoped works; undone reopens
    ruwana(wiki.path())
        .args(["done", "Shared name", "--project", "alpha"])
        .assert()
        .success();
    ruwana(wiki.path())
        .args(["undone", "Shared name", "--project", "alpha"])
        .assert()
        .success();
    ruwana(wiki.path())
        .args(["list", "--project", "alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Shared name"));
}

#[test]
fn error_table_messages_and_exit_codes() {
    let wiki = wiki_with(&["proj"]);
    let id = add_task(wiki.path(), "proj", "Exists", &[]);

    let cases: Vec<(Vec<&str>, &str)> = vec![
        (
            vec!["done", "--id", "zzzzzzzz"],
            "no task found with id: zzzzzzzz",
        ),
        (
            vec!["done", "No such title"],
            "no task found with title: \"No such title\"",
        ),
        (
            vec!["done", "--id", "bad-format!"],
            "invalid --id format, expected <task-id> or <task-id>:<subtask-id>",
        ),
        (
            vec!["done", &id, "--id", &id],
            "--id and a positional id/title are mutually exclusive",
        ),
        (
            vec!["list", "--project", "ghost"],
            "project path not found: ghost",
        ),
        (
            vec!["add", "--project", "proj", "X", "--due", "gibberish"],
            "cannot parse date: \"gibberish\"",
        ),
        (
            vec!["edit", &id],
            "edit requires at least one field to change",
        ),
        (vec!["add", "--project", "proj", "   "], "invalid title"),
    ];
    for (args, message) in cases {
        ruwana(wiki.path())
            .args(&args)
            .assert()
            .code(1)
            .stderr(predicate::str::contains(message));
    }

    let missing_sub = format!("{id}:zzzz");
    ruwana(wiki.path())
        .args(["done", "--id", &missing_sub])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(format!(
            "no sub-task found with id: zzzz in task {id}"
        )));
}

#[test]
fn rm_refuses_without_force_non_interactively_and_yes_alias_works() {
    let wiki = wiki_with(&["proj"]);
    let id = add_task(wiki.path(), "proj", "Doomed", &[]);

    // assert_cmd runs with stdin not a TTY → must refuse, file untouched
    ruwana(wiki.path())
        .args(["rm", &id])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "refusing to delete without --force in non-interactive mode",
        ));
    ruwana(wiki.path()).args(["show", &id]).assert().success();

    ruwana(wiki.path())
        .args(["rm", &id, "--yes"])
        .assert()
        .success();
    // Positional resolution falls through ID → title when the ID-shaped
    // file is missing (spec: Resolution Rules step 1 → step 2), so the
    // exact-id `id` no longer exists on disk and is retried, and fails,
    // as a title lookup.
    ruwana(wiki.path())
        .args(["show", &id])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(format!(
            "no task found with title: \"{id}\""
        )));
}

#[test]
fn corrupt_file_is_skipped_with_warning_but_direct_target_fails_loudly() {
    let wiki = wiki_with(&["proj"]);
    add_task(wiki.path(), "proj", "Healthy", &[]);
    std::fs::write(wiki.path().join("proj/.ruwana/broken12.toml"), "not toml [").unwrap();

    // list: exit 0, healthy task on stdout, warning on stderr
    ruwana(wiki.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Healthy"))
        .stderr(predicate::str::contains("skipping unparseable task file:"));

    // directly targeting the broken file: loud failure
    ruwana(wiki.path())
        .args(["show", "--id", "broken12"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("cannot parse task file:"));
}

#[test]
fn out_of_band_file_changes_are_visible_immediately() {
    let wiki = wiki_with(&["proj"]);
    let id = add_task(wiki.path(), "proj", "Original", &[]);
    // Simulate a git pull / hand edit: rewrite the file behind ruwana's back
    let path = wiki.path().join(format!("proj/.ruwana/{id}.toml"));
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("Original", "Edited elsewhere");
    std::fs::write(&path, edited).unwrap();

    ruwana(wiki.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("Edited elsewhere"));
}

#[test]
fn overdue_urgent_and_due_sorting() {
    let wiki = wiki_with(&["proj"]);
    add_task(wiki.path(), "proj", "Way past", &["--due", "2020-01-01"]);
    add_task(wiki.path(), "proj", "Today task", &["--due", "today"]);
    add_task(wiki.path(), "proj", "Far future", &["--due", "2099-01-01"]);
    add_task(wiki.path(), "proj", "No due", &[]);

    ruwana(wiki.path())
        .args(["list", "--overdue"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Way past"))
        .stdout(predicate::str::contains("Today task").not()); // due today ≠ overdue

    ruwana(wiki.path())
        .args(["list", "--urgent"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Today task"))
        .stdout(predicate::str::contains("Far future").not());

    // default sort: due ascending, no-due last; overdue rendered as marker
    let output = ruwana(wiki.path()).arg("list").output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines[0].contains("Way past") && lines[0].contains("overdue"),
        "{stdout}"
    );
    assert!(lines[1].contains("Today task"));
    assert!(lines[2].contains("Far future") && lines[2].contains("due: 2099-01-01"));
    assert!(lines[3].contains("No due"));
}

#[test]
fn tag_and_source_filters_end_to_end() {
    let wiki = wiki_with(&["proj"]);
    add_task(
        wiki.path(),
        "proj",
        "Both tags",
        &["--tag", "okr", "--tag", "review"],
    );
    add_task(
        wiki.path(),
        "proj",
        "One tag",
        &["--tag", "okr", "--source", "meet-a"],
    );
    add_task(wiki.path(), "proj", "From b", &["--source", "meet-b"]);

    // AND semantics
    let output = ruwana(wiki.path())
        .args(["list", "--tag", "okr", "--tag", "review"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Both tags") && !stdout.contains("One tag"),
        "{stdout}"
    );

    // OR semantics
    let output = ruwana(wiki.path())
        .args(["list", "--source", "meet-a", "--source", "meet-b"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("One tag") && stdout.contains("From b") && !stdout.contains("Both tags"),
        "{stdout}"
    );
}

#[test]
fn wiki_root_defaults_to_home_wiki() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("wiki/proj")).unwrap();
    let mut cmd = Command::cargo_bin("ruwana").unwrap();
    cmd.env_remove("WIKI_ROOT")
        .env("HOME", home.path())
        .env("TZ", "Europe/Berlin")
        .args(["add", "--project", "proj", "Default root"])
        .assert()
        .success();
    let entries = std::fs::read_dir(home.path().join("wiki/proj/.ruwana"))
        .unwrap()
        .count();
    assert_eq!(entries, 1);
}
