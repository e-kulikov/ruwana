# PR #1 Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the correctness and release-automation defects found in PR #1 while preserving the approved direct-read TOML architecture and its explicit trade-offs.

**Architecture:** Keep `ruwana-core` as the filesystem and domain layer and `ruwana-cli` as a thin adapter. Harden the storage boundary so malformed files are classified consistently, resolution detects impossible global-ID ambiguity, and calendar-dependent filtering receives the querying timezone explicitly. Configure release-plz so `ruwana-cli` alone owns the shared tag and root changelog.

**Tech Stack:** Rust 1.85+, edition 2024, chrono, serde/toml, walkdir, rayon, release-plz, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-07-03-ruwana-design.md` and `docs/superpowers/specs/2026-07-06-release-cicd-design.md`.

## Global Constraints

- Work only in `/home/ekulikov/code/github/e-kulikov/ruwana/v1` on branch `v1`; do not push, merge, publish, or post to GitHub.
- Preserve direct TOML reads; do not add SQLite, an index, or a cache.
- Follow test-driven development for every Rust behavior change: add a focused regression test, run it and record the expected failure, then implement the smallest fix and rerun it.
- Keep `ruwana-core` free of clap, argv, stdout, stderr, and process exits.
- All task data filesystem access remains behind `Store`.
- Preserve approved last-writer-wins semantics unless the advisor rules that a concrete implementation defect falls outside that decision. Do not add lock files or compare-and-swap merely to change the accepted concurrency policy.
- Preserve the approved project path policy: explicit paths are checked lexically and may traverse trusted symlinks; global discovery skips hidden directories and does not follow symlinks. Treat these as deliberate scope differences unless the advisor finds an actual contradiction in observable requirements.
- Maintain Rust 1.85 compatibility and the existing public CLI output unless this plan names a new error case.
- Before committing Rust changes run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`.
- Use conventional commit messages. Local commits are allowed; do not push them.
- When a design choice is unclear, ask `ruwana-advisor` through Herdr, record the question and ruling in the SDD ledger, then continue.

## Pre-flight rulings required from the advisor

Before production edits, obtain and record rulings for these points:

1. Choose the smallest Rust-1.85-compatible method for replacing an existing file atomically on Windows while keeping the temp file on the target filesystem. Prefer a maintained crate over hand-written unsafe platform bindings.
2. Confirm the timezone-shaped query API. `--urgent` must use the querying machine's local calendar, while `--due <date>` continues to use the due timestamp's stored offset.
3. Confirm that last-writer-wins, hidden project visibility, and symlink discovery are approved behavior rather than bugs. If so, change only misleading comments/docs and add tests that pin the intended behavior.
4. Decide whether a missing/unreadable `WIKI_ROOT` should be a hard I/O error for global commands and how per-entry walk errors should propagate without turning malformed TOML into a fatal error.
5. Confirm release-plz's package-level single-tag configuration and whether the shared workspace version guarantees that a core-only change releases `ruwana-cli`; add `version_group` only if required by release-plz semantics.

---

### Task 1: Make `ruwana-cli` the sole release-plz tag and changelog owner

**Files:**
- Modify: `release-plz.toml`
- Test/validate: release-plz configuration and the existing release workflow

**Interfaces:**
- Consumes: workspace crates `ruwana-core` and `ruwana-cli`, root `CHANGELOG.md` path.
- Produces: one `v{{ version }}` tag and one root changelog containing changes from both crates; GitHub Releases remain disabled for release-plz because cargo-dist owns them.

- [ ] Add a configuration regression check that parses `release-plz.toml` and asserts workspace tag/changelog generation is disabled, exactly one package enables both, and that package includes `ruwana-core` in the root changelog. Run it against the current file and confirm it fails. A small repository test script is acceptable only if it remains useful in CI; otherwise record a deterministic `python3`/`cargo` validation command in the ledger.
- [ ] Replace the workspace settings with `git_tag_enable = false` and `changelog_update = false`, retaining `publish = false`, `git_release_enable = false`, `git_tag_name = "v{{ version }}"`, `semver_check = false`, and `pr_labels = ["release"]`; add `git_only = true` and `release_always = false`.
- [ ] Add `[[package]]` for `name = "ruwana-cli"` with `git_tag_enable = true`, `git_tag_name = "v{{ version }}"`, `changelog_update = true`, `changelog_path = "./CHANGELOG.md"`, and `changelog_include = ["ruwana-core"]`. Apply the advisor's `version_group` ruling if needed.
- [ ] Validate the final config with release-plz's official schema/config command when available, and rerun the regression check.
- [ ] Commit this task independently.

### Task 2: Make task-file replacement cross-platform and temp names truly unique

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `crates/core/Cargo.toml` only if the chosen replacement primitive needs a dependency
- Modify: `crates/core/src/store.rs`
- Test: `crates/core/src/store.rs`
- Modify: `.github/workflows/ci.yml` only if the advisor determines Windows execution is needed to exercise the regression

**Interfaces:**
- `Store::save(&self, project: &str, task: &Task) -> Result<PathBuf, Error>` keeps its observable contract.
- Each save uses a same-directory temp path unique across processes and threads.
- Replacing an existing task succeeds on Unix and Windows; any temp file is cleaned up after a failed write/replace when possible.

- [ ] Keep/add a regression test that repeatedly overwrites an existing task and verifies the final file parses and no temp files remain. Do not claim this fails on current Linux or Windows: advisor verified that current Rust `std::fs::rename` already replaces an existing destination on Windows. The cross-platform replacement premise is corrected; the actionable defects are PID-only temp collisions and missing cleanup after errors.
- [ ] Add a same-process concurrent-save stress test using several threads and a barrier. It must fail against the PID-only temp name because at least one writer errors or leaves an invalid/temporary file.
- [ ] Generate each temp name with a collision-resistant per-save suffix in addition to the task ID. Create/write the temp file without clobbering another writer's temp file.
- [ ] Use `tempfile::Builder::tempfile_in(&dir)` and `NamedTempFile::persist(&target)` as ruled by the advisor. Promote the locked `tempfile` dependency into core production dependencies. Map `PersistError` back to its `std::io::Error`. Sync or durability changes are out of scope.
- [ ] Ensure all success and expected error paths leave no `.tmp` files from the completed operation.
- [ ] Run the focused store tests, then the full gate, and commit independently.

### Task 3: Treat every malformed task file consistently

**Files:**
- Modify: `crates/core/src/store.rs`
- Modify: `crates/core/src/model.rs` or `crates/core/src/ids.rs` only if validation belongs there
- Modify: `crates/core/src/lib.rs` if a typed corruption error changes
- Test: core store/model tests and CLI e2e tests where user-facing behavior matters

**Interfaces:**
- A directly targeted corrupt file returns `Error::TaskFileParse { path, message }`.
- A corrupt file encountered by a multi-task read is skipped with `Warning("skipping unparseable task file: ...")`.
- Corruption includes invalid UTF-8, a filename/outer-ID mismatch, malformed outer or subtask ID shape, and duplicate subtask IDs.

- [ ] Add a test containing invalid UTF-8 bytes. Confirm `load` currently returns a generic I/O error and `load_project` aborts instead of warning and continuing.
- [ ] Add table-driven malformed-record tests for an outer ID not matching `[a-z0-9]{8}`, a malformed 4-character subtask ID, and duplicate subtask IDs. Confirm each currently loads successfully where applicable.
- [ ] Centralize loaded-task invariant validation and map UTF-8 decoding failures through the same task-file parse/corruption path as TOML failures.
- [ ] Preserve direct-target failure and multi-read skip-with-warning behavior for every corruption class.
- [ ] Add an e2e test proving `list` emits one stderr warning, keeps stdout parseable, and returns the valid neighboring task when a `.toml` file contains invalid UTF-8.
- [ ] Run focused tests, the full gate, and commit independently.

### Task 4: Reject globally ambiguous task IDs

**Files:**
- Modify: `crates/core/src/resolve.rs`
- Modify: `crates/core/src/lib.rs`
- Test: `crates/core/src/resolve.rs` and `crates/cli/tests/e2e.rs`

**Interfaces:**
- With `--project`, ID resolution retains the existing one-project behavior.
- Without `--project`, one filename match resolves and more than one matching `<id>.toml` returns a typed ambiguity error listing candidate projects in deterministic order. An explicit `--id` with zero matches returns `IdNotFound`; a positional ID-shaped selector with zero matches preserves its existing exact-title fallback.
- Positional ID-shaped selectors and explicit `--id`, including compound subtask selectors, share this rule.

- [ ] Add core tests that create the same valid task ID in two projects and exercise explicit ID, positional ID-shaped lookup, and compound lookup. Confirm the current code silently returns the first project. Also pin the positional zero-hit title fallback.
- [ ] Add a specific `Error` variant and stable single-line display text such as `ambiguous task id; use --project to narrow: <id> (<project-a>, <project-b>)`.
- [ ] Change ID resolution to collect all filename hits before loading. Load only the sole match; keep project names sorted.
- [ ] Add an e2e assertion for exit code 1 and candidate projects on stderr.
- [ ] Run focused tests, the full gate, and commit independently.

### Task 5: Evaluate `--urgent` in the querying timezone

**Files:**
- Modify: `crates/core/src/query.rs`
- Modify: `crates/core/src/ops.rs`
- Modify: `crates/cli/src/main.rs`
- Test: core query tests and CLI e2e tests
- Modify: `docs/superpowers/specs/2026-07-03-ruwana-design.md` only to resolve its internal wording conflict

**Interfaces:**
- Query evaluation receives both an absolute `now` instant and an explicit calendar timezone/context.
- `Overdue` continues comparing instants.
- `Urgent` projects `now` and each due instant into the querying timezone and matches today/tomorrow there.
- `due_on` continues reading the calendar date from the due value's baked-in offset.

- [ ] Add a fixed-offset cross-timezone regression: choose a due instant whose stored-offset date differs from its date in the querying offset, and prove current `Urgent` classification is wrong relative to the querying calendar.
- [ ] Add a DST-sensitive regression using an injected timezone/context without relying on the host timezone. If chrono's `Local` cannot be injected hermetically, keep core generic over `TimeZone` and use `FixedOffset` tests that cover both date-boundary directions.
- [ ] Extend `query::apply` and callers with the minimal explicit timezone parameter. The CLI supplies `chrono::Local`; library tests supply deterministic offsets/timezones.
- [ ] Leave `due_on` and display semantics unchanged.
- [ ] Amend the approved spec sentence that currently says urgent compares only instants so it agrees with the later explicit `querying machine's local calendar` requirement.
- [ ] Run focused tests, the full gate, and commit independently.

### Task 6: Surface global discovery failures without changing deliberate visibility rules

**Files:**
- Modify: `crates/core/src/discover.rs`
- Modify: `crates/core/src/resolve.rs`
- Modify: `crates/core/src/ops.rs`
- Modify: `crates/core/src/lib.rs` if needed
- Test: discovery, ops/resolve, and CLI e2e tests

**Interfaces:**
- `discover_projects` returns a `Result` (or an equally explicit report) so a missing root and walk failures are not silently presented as an empty task set.
- Malformed task files remain nonfatal warnings; discovery filesystem failures are surfaced as I/O errors.
- Hidden-directory skipping, no-follow discovery, and trusted explicit symlink paths remain as approved unless the advisor rules otherwise.

- [ ] Add a missing-root test and confirm current discovery returns an empty vector.
- [ ] Add the strongest portable unreadable/walk-error test available. On platforms/users where permissions cannot reproduce it, unit-test the error-mapping helper and keep the missing-root integration test.
- [ ] Change discovery to propagate root and traversal errors with their path/context, and thread the result through ID collision checks, global resolution, and global list. Expose discovery through `Store` while leaving `discover.rs` as its implementation helper so filesystem access remains behind the storage boundary.
- [ ] Replace `read_dir(...).filter_map(|entry| entry.ok())` in `Store::load_project` with iteration that preserves directory-enumeration errors; do not misclassify filesystem errors as task corruption.
- [ ] Add regression tests pinning the deliberate behavior: hidden projects and symlinked projects are omitted globally, while a valid explicit path may still address them.
- [ ] Ensure task creation cannot loop on a discovery failure: `unique_task_id` must return `Result<String, Error>` and `add` must propagate it.
- [ ] Run focused tests, the full gate, and commit independently.

### Task 7: Tighten the public storage boundary and correct unsupported claims

**Files:**
- Modify: `crates/core/src/store.rs`, `crates/core/src/lib.rs`, and callers as needed
- Modify: `docs/superpowers/specs/2026-07-03-ruwana-design.md`
- Modify: `README.md` only if it repeats an affected claim
- Test: core store tests

**Interfaces:**
- Public methods that form paths from project/task IDs either validate their inputs internally or become `pub(crate)` with validated public operations remaining available.
- Legitimate explicit symlink projects remain supported.
- Documentation describes random IDs as overwhelmingly unlikely to be reused after deletion; it does not promise tombstone-backed non-reuse.
- Precise performance numbers remain only if backed by a committed reproducible benchmark; otherwise replace them with a qualitative scale statement.

- [ ] Add tests showing malformed IDs and escaping/absolute project inputs cannot reach a public filesystem mutation method.
- [ ] Restrict path constructors and raw save/delete methods or add validation at their common boundary, choosing the smallest API that prevents callers—including public selectors carrying strings—from bypassing project/ID invariants. Making methods `pub(crate)` alone is insufficient. Do not duplicate validation inconsistently across operations.
- [ ] Update the ID non-reuse wording to match the implementation's random-generation guarantee; do not add tombstones.
- [ ] Remove precise benchmark numbers unless a stable benchmark harness already exists and can reproduce them; retain the architectural rationale without unsupported measurements.
- [ ] Run focused tests, the full gate, and commit independently.

### Task 8: Final integration verification and self-review

**Files:**
- Review every file changed since base `f17dcbaeb80e095dcbbddd9560318b22ba71383d`
- Update this plan/ledger only with completed evidence and rulings

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `cargo test --workspace` and record the exact test count.
- [ ] Run the Rust 1.85 compatibility check using the repository's pinned/available 1.85 toolchain.
- [ ] Validate `release-plz.toml` and inspect `.github/workflows/*.yml` for syntax.
- [ ] Run `git diff --check` and inspect `git status --short`.
- [ ] Self-review the complete diff for accidental public API changes, host-timezone-dependent tests, temp artifacts, and edits that contradict the approved spec.
- [ ] Commit remaining plan/doc-only changes if any. Do not push.
