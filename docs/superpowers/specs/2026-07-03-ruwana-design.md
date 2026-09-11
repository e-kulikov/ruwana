# `ruwana` — CLI Specification (v1)

> Agent-facing task tracker for a personal wiki system.
> Implementation language: Rust.
> Status: **approved spec** — supersedes `spec.draft.md`.

---

## Naming

The tool is called **`ruwana`** — Quechua for "the thing that must be done."
It's *ruway* ("to do/make") plus the nominalizing suffix *-na*, the same
pattern that turns *mikhuy* ("to eat") into *mikhuna* ("food," literally
"the thing to be eaten"). It's the closest single-word Quechua counterpart
to Latin's *agendum*.

**Pronunciation:** /ruˈwana/ — stress on the middle syllable; roughly
**"roo-WAH-nah"**.

---

## Problem Statement

The personal wiki system (LLM-maintained markdown + zk) needs task management
that is: agent-reliable (consistent format every session), human-readable
(TOML files on disk), project-aware (tasks scoped to wiki subprojects), and
self-contained (no external service dependencies). Existing tools
(Taskwarrior, GitHub Issues) don't integrate cleanly with the git-submodule
project isolation model where work tasks must die with their submodule.

---

## Goals and Non-Goals

**Goals (v1):**

- Full task CRUD from the command line, scriptable and prompt-free (except
  `rm` confirmation, bypassed with `--force`).
- TOML files as the single — and only — source of truth. No derived state
  (no database, no cache files) to keep consistent.
- Per-project isolation: everything about a project's tasks lives under that
  project's directory and travels (or dies) with it.
- Machine-parseable output (`--format json`) as a first-class surface.
- Core logic in a library crate so a future TUI reuses it directly.

**Non-Goals (v1):**

- Multi-user collaboration, sync to external services, notifications, GUI.
- Recurring tasks, priorities, interactive editing (deferred — see Roadmap).
- Preserving comments/formatting in hand-edited TOML files (see Task File
  Format → Canonical serialization).
- A query index/cache. Measured at target scale, it costs more than it
  saves — see Query Engine. The storage API is designed so one can be
  added later without touching any command.

---

## Design Principles

- **Agent-first interface**: every command must be unambiguous and
  scriptable. No interactive prompts (the sole exception, `rm`, has a
  documented `--force` bypass). Output must be parseable.
- **File-system native**: TOML files are not just the source of truth —
  they are the *only* state. Every command reads them directly; nothing can
  ever be stale, and `git pull`, submodule updates, and hand edits are
  visible immediately with no sync step.
- **Project isolation**: tasks are scoped to projects via `--project`, which
  maps to a directory path within the wiki. Work tasks die with their
  submodule — trivially, since a project's `.ruwana/` directory is the
  entirety of its task state.
- **Human-readable at rest**: a task file opened in any editor is legible
  without tooling.
- **Library-first**: all task logic (CRUD, ID/sub-task resolution, date
  parsing, querying) lives in the library layer, separate from the CLI
  argument-parsing layer. The `ruwana` binary is a thin wrapper over that
  library. This is a v1 requirement, not a later refactor — it's what lets a
  future `ruwana tui` call the same functions directly instead of shelling
  out to the CLI or duplicating logic.

---

## Architecture

### Monorepo layout

The repository is a monorepo of two packages — `ruwana-core` (library) and
`ruwana-cli` (binary) — managed as a **Cargo workspace**. Cargo itself is
the monorepo manager: for a pure-Rust repo of this size it already provides
everything heavier managers (Bazel, Buck2, moon, Nx, cargo-make) exist to
add for polyglot or very large repos — one shared `Cargo.lock` and unified
dependency resolution, one `target/` cache with cross-crate incremental
builds, `cargo build/test/clippy --workspace` as the single CI entry point,
and `[workspace.dependencies]` / `[workspace.package]` / `[workspace.lints]`
so versions, metadata, and lint policy are declared once and inherited. A
build system layered on top would add configuration surface without adding
capability; if the repo ever gains non-Rust components, a task runner can
be added later without restructuring.

```
ruwana/                          ← workspace root
├── Cargo.toml                   ← [workspace]: members = ["crates/*"],
│                                   workspace.package, workspace.dependencies,
│                                   workspace.lints
├── Cargo.lock                   ← single lockfile for the whole workspace
├── crates/
│   ├── core/                    ← library crate: ALL task logic
│   │   ├── Cargo.toml           ← name = "ruwana-core" (see naming note)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── model.rs         ← Task, SubTask, Status types (serde)
│   │       ├── store.rs         ← storage API: load/save/delete tasks,
│   │       │                       .ruwana dir handling (see Query Engine)
│   │       ├── discover.rs      ← WIKI_ROOT walk, project discovery
│   │       ├── query.rs         ← in-memory filtering and sorting
│   │       ├── resolve.rs       ← ID / title / compound-ID resolution
│   │       ├── dates.rs         ← natural + explicit date parsing, EOD resolution
│   │       ├── ids.rs           ← ID generation (8-char task, 4-char sub-task)
│   │       └── ops.rs           ← command-level API: add, done, undone, list…
│   └── cli/                     ← binary crate; produces the `ruwana` binary
│       ├── Cargo.toml           ← name = "ruwana-cli", [[bin]] name = "ruwana";
│       │                           depends on ruwana-core
│       ├── src/
│       │   ├── main.rs
│       │   ├── args.rs          ← clap definitions (subcommands, aliases, groups)
│       │   └── output.rs        ← text/JSON formatting of ops results
│       └── tests/               ← integration tests (assert_cmd, temp WIKI_ROOT)
└── docs/
```

**Naming.** Directory names are unprefixed (`crates/core`, `crates/cli`) —
the `crates/` path already namespaces them, so repeating `ruwana-` there
would be noise. Package names keep the prefix (`ruwana-core`,
`ruwana-cli`) because they live in Cargo's flat, global namespace: Cargo
rejects a package named `core` outright (it shadows Rust's built-in
`core` crate), and unprefixed names would collide on crates.io if ever
published. Directory and package name are independent in Cargo, so this
costs nothing — `members = ["crates/*"]` picks them up regardless, and
imports read `use ruwana_core::…`.

Boundary rules:

- `ruwana-core` has **no dependency on clap** and no knowledge of argv,
  stdout, or exit codes. Its `ops` module takes plain Rust types and
  returns `Result<T, ruwana_core::Error>` with typed errors — this is the
  exact API a future `ruwana-tui` consumes (it would join `crates/` as a
  third workspace member depending on `ruwana-core`).
- `ruwana-cli` contains no task logic: it parses args (`args.rs`), calls
  one `ops::*` function, and formats the result (`output.rs`). Mapping
  typed core errors to the exit-1 messages in Error Handling lives here.
- All file access goes through `store.rs`. The rest of the crate asks the
  store for tasks; it does not open files itself. This is the seam where a
  cache could be introduced later (see Query Engine) with zero changes to
  `ops`, `query`, `resolve`, or the CLI.
- Unit tests live beside the code in `ruwana-core`; end-to-end CLI tests
  live in `crates/cli/tests/` and exercise the real binary against a temp
  `WIKI_ROOT`.
- Shared dependency versions are declared once in
  `[workspace.dependencies]`; member crates reference them with
  `dep = { workspace = true }`.

*Why a workspace:* the two-crate split makes the library boundary a hard
compile-time contract (`ruwana-core` cannot even see clap), gives the
future TUI an obvious home as a third member crate, and keeps CLI-only
dependencies out of the library's dependency tree.

### Dependencies

| Concern | Crate | Notes |
| --- | --- | --- |
| CLI parsing | `clap` v4 (derive) | `ruwana-cli` only; subcommands, aliases, arg groups |
| TOML | `toml` (serde) | canonical serialization; see Task File Format |
| Dates/times | `chrono` | local-offset ISO 8601 timestamps |
| Natural dates | `interim` | maintained fork of `chrono-english` (zk uses the Go analogue `tj/go-naturaldate`) |
| Randomness | `rand` | ID generation |
| JSON output | `serde_json` | `--format json` |
| Errors | `thiserror` (core) + `anyhow` (cli) | typed errors in the library API |
| Walking | `walkdir` | project discovery |
| Parallel parse | `rayon` | parallelize TOML parsing across files (see Query Engine) |
| Tests | `assert_cmd`, `predicates`, `tempfile` | integration tests |

There is deliberately no SQLite dependency — see Query Engine.

Explicit numeric-date parsing (`2024-03-05`, `05.12.2024`, `11/27/2024`) is
hand-rolled (~30 lines) rather than delegated, because the separator itself
disambiguates day/month order — see Date Parsing.

---

## Storage Layout

```
$WIKI_ROOT/<project>/.ruwana/
├── abc1de2f.toml      ← one file per task, named by ID
├── gh3ij4kl.toml
└── ...
```

`.ruwana/` lives inside each project directory and contains nothing but
task files. The `--project` flag specifies the project path relative to the
wiki root (e.g. `godel/ai-practice`), which resolves to
`$WIKI_ROOT/godel/ai-practice/.ruwana/`.

`WIKI_ROOT` is read from the `WIKI_ROOT` environment variable, defaulting to
`~/wiki`.

Rules:

- The **project directory** must already exist — `ruwana` never creates
  project directories (`project path not found` otherwise). The `.ruwana/`
  directory inside it, however, is created on demand by the first `add`.
- Everything about a project's tasks is inside its `.ruwana/` directory and
  is plain committed data. Deleting or de-initializing a submodule removes
  every trace of its tasks; a fresh clone is immediately fully functional.
  Nothing needs gitignoring.
- `--project` must be a relative path with no `..` components; it is
  resolved strictly under `$WIKI_ROOT` (reject anything that escapes it).
- Path checks are **lexical only**: symlinks inside the wiki are trusted
  and followed. A symlinked project directory that points outside
  `$WIKI_ROOT` is a deliberate user setup, not an attack surface — this is
  a single-user tool operating on the user's own data, so no
  canonicalization or symlink-escape guard is performed (explicit decision
  from the 2026-07-03 adversarial review).

### Project discovery

Commands that operate across all projects (`list` without `--project`, ID
lookup without `--project`) discover projects by walking `$WIKI_ROOT`
recursively looking for `.ruwana` directories, with these rules:

- Skip hidden directories (dot-prefixed) during descent — except `.ruwana`
  itself, which is the match target. `.git` is therefore skipped for free.
- Do not descend *into* a `.ruwana` directory looking for nested projects.
- Follow the directory tree only (no symlink following), to keep the walk
  cheap and cycle-free.

Discovery remains a small direct filesystem walk; no benchmark claim is
made without a committed reproducible harness.

---

## Query Engine

**There is no database.** Every command reads the TOML files it needs,
directly, every time. Filtering, sorting, and title lookup happen in memory
over parsed tasks (`query.rs`); ID lookup doesn't even parse — a task's ID
*is* its filename, so resolving an ID is a `stat` of `<id>.toml` per
discovered project.

This is an architectural decision, not a simplification shortcut. An earlier
draft specified a per-project SQLite index with staleness detection,
auto-rebuild, and git-hook triggers. Direct reads avoid that additional
consistency subsystem. The structural rationale is:

- **Correctness forces the index to do the walk anyway.** To be safe
  against `git pull` and hand edits, every indexed command must
  readdir+stat all task files first — the same syscall work direct parsing
  starts with. The index only ever saves the parse step.
- **The index's failure-recovery path is slower than no index.** A rebuild
  is parse-everything *plus* SQLite writes — every `git pull` would put the
  indexed design on a path slower than this design's steady state.
- **ID lookup needs no index**, because the storage layout already encodes
  the primary key in the filename.
- **SQL can't accelerate the hard filters anyway.** Tag matching against a
  delimited text column is a table scan inside SQLite too; the in-memory
  `Vec` filter does identical work without a schema.

What the tool drops in exchange: the SQLite schema
and dependency, staleness detection, auto-rebuild, an `index` subcommand,
git hook templates, and a gitignore requirement — an entire consistency
subsystem whose only job would have been defending a 5 ms saving.

**Escape hatch.** All file access goes through `store.rs` (see Boundary
rules). If a wiki ever grows past tens of thousands of tasks, a global
read-through cache (SQLite in the XDG cache directory, keyed by
canonicalized `WIKI_ROOT` — never inside the wiki) can be added behind
that interface without changing any command, output, or file format. That
is a P2+ possibility, deliberately unscheduled.

**Unparseable files** encountered during a multi-task read (`list`, title
resolution, ID collision checks) are skipped with a warning on stderr
(`skipping unparseable task file: <path>: <error>`); one broken file must
not brick the whole wiki. A command that *directly targets* a broken file
(`show abc1de2f`) fails loudly instead — see Error Handling.

**Concurrency.** `ruwana` assumes a single user but not a single process
(an agent and a human can race). Task-file writes are atomic (pid-unique
temp file + rename, see Task File Format), and with no derived state there
is nothing to get out of sync: the worst case for a racing read is seeing
the file as it was a moment ago, and the worst case for two racing
*mutations* of the same task is **last-writer-wins** — the slower write
replaces the faster one wholesale, never a corrupted mix. That trade-off
is accepted deliberately for a single-user tool (confirmed in the
2026-07-03 adversarial review): no lock files, no busy-timeouts, no
compare-and-swap.

---

## Task File Format

Each task is stored as a single TOML file named `<id>.toml`:

```toml
id       = "abc1de2f"
title    = "Write the onboarding doc"
status   = "open"
due      = "2024-03-15T23:59:59+01:00"
tags     = ["review", "okr", "q3"]
source   = ["meeting-2024-01-15"]
created  = "2024-01-15T09:32:11+01:00"
modified = "2024-01-20T14:05:47+01:00"

related = [
  "[Meeting 2024-01-15](../wiki/2024-01-15-q1-planning.md)",
  "https://linear.app/team/issue/ABC-123",
]

description = """
Optional free-text description of the task. Markdown supported.
"""

[[tasks]]
id   = "gh7f"
text = "Sub-task one"
done = false

[[tasks]]
id   = "j2kx"
text = "Sub-task two"
done = false
```

Field semantics:

- `id`, `title`, `status`, `created`, `modified` — required. `status` is
  `"open"` or `"done"`; unknown values are a parse error.
- `due`, `description` — optional; omitted when unset (never written as an
  empty string).
- `tags`, `source`, `related`, `tasks` — optional arrays; omitted when
  empty.
- All timestamps are RFC 3339 with an explicit UTC offset (see Timezone
  Handling).
- There is **no `project` field** in the file. The project is not data
  belonging to the task — it's the `<project>` path segment the file already
  lives under (`$WIKI_ROOT/<project>/.ruwana/<id>.toml`); storing it again
  would be a duplicate that could drift. Commands read it off the file's
  path.

`source` is a separate, structured field from `related`: it's specifically
what `--source` on `add`/`edit` writes to, and what `--source` on `list`
filters against. `related` stays free text for anything else worth linking —
a Linear issue, a wiki note — that isn't necessarily "the thing that
produced this task." A meeting transcript that spawned a task would
typically show up in both.

When a task is marked done, `status` changes to `"done"` and `modified` is
updated. No other field changes shape.

**Canonical serialization.** `ruwana` writes task files by serializing the
in-memory task via serde — fields in the order shown above, sub-tasks as
trailing `[[tasks]]` blocks. Any write rewrites the whole file in canonical
form. Consequence: TOML comments and bespoke formatting added by hand do
**not** survive the next `ruwana` mutation of that file. Hand-editing is
supported (the file parses fine and is picked up immediately, since there
is no cache to refresh) but hand-*decorating* is not. Anything worth
keeping belongs in `description` or `related`.

**Writes are atomic**: write to a uniquely named temp file
(`<id>.toml.<pid>.tmp`) in the same directory, then rename over the target.
A crash mid-write never leaves a corrupt task file, and because the temp
name includes the writing process's pid, two racing writers can never
share a temp path — a race ends with one complete file cleanly replacing
the other, never a mix (see Query Engine → Concurrency).

**Sub-task IDs.** Each `[[tasks]]` entry has its own `id` — 4 characters
instead of 8 (see ID Generation), since a sub-task ID only needs to be
unique within its parent's `tasks` array; it's always addressed together
with the parent ID (see "Addressing a Sub-task").

---

## ID Generation

- Task IDs: 8 characters from `[a-z0-9]` (36 symbols; 36⁸ ≈ 2.8 × 10¹²),
  generated from a CSPRNG. Example: `a3bc9f2e`.
- On generation, collision checks cover every globally discovered project
  plus the explicitly supplied destination project (one `stat` per project;
  collision → regenerate). Discovery visibility/no-follow rules are
  unchanged. This is a snapshot check and does not guarantee uniqueness
  under races. At this keyspace a collision is effectively never, but the
  check is nearly free and keeps "globally unique" honest.
- Deletion removes the file. New random IDs are overwhelmingly unlikely to
  reuse a deleted value; no tombstone-backed non-reuse guarantee exists.
- Sub-task IDs: same charset, 4 characters (36⁴ ≈ 1.68 million), unique
  only within the parent's `tasks` array (collision within the array →
  regenerate). Example: `gh7f`. Global uniqueness would be overkill: a
  sub-task is always addressed together with its parent ID.

---

## Resolution Rules

Every command that takes `<id-or-title>` resolves it in this order:

1. If the argument is exactly 8 chars of `[a-z0-9]`, try it as an ID first:
   check for `<id>.toml` in the given project's `.ruwana/` (or every
   discovered project's, when `--project` is absent). The ID is the
   filename, so this is a `stat`, not a parse. Exactly one match →
   resolved; more than one match → `ambiguous task id; use --project to
   narrow: <id> (<project-a>, <project-b>)`.
2. Otherwise (or if no ID matched), treat it as a title: **exact,
   case-sensitive** string match on `title` across parsed tasks, scoped by
   `--project` if given.
   - 0 matches → error `no task found with title: "..."`.
   - > 1 matches → error `ambiguous title; use --project to narrow or use ID`
     (the error lists the matching IDs and projects on stderr).
   - `--project` is **required** for title resolution only in the sense that
     ambiguity across projects is an error; a title that is unique across
     the whole wiki resolves fine without it.

There is no prefix/fuzzy matching in v1 — agents copy exact IDs, and exact
matching keeps resolution deterministic.

### Addressing a Sub-task

Commands where operating on a single sub-task makes sense — `done`,
`undone`, `rm`, `edit` — accept an explicit `--id` flag as an alternative to
the positional `<id-or-title>`:

```
--id <task-id>              # same as the positional task ID
--id <task-id>:<subtask-id> # targets one sub-task within that task
```

```sh
ruwana done --id abd4hg2f:gh7f
```

Rules:

- `--id` and the positional `<id-or-title>` are mutually exclusive
  (enforced as a clap arg group).
- The compound form resolves `<task-id>` first (filename lookup, like any
  ID), then finds `<subtask-id>` among that task's `[[tasks]]` entries.
- `--project` stays optional with `--id`. Titles never resolve to sub-tasks
  (they have no title of their own), so `--id` is the only way to target
  one.
- `done`/`undone` on a sub-task flips that entry's `done` field only; the
  parent's own `status` is untouched, and the parent's `modified` is
  bumped.
- `rm` on a sub-task removes just that `[[tasks]]` entry (and bumps
  `modified`); the parent task and file survive. `rm` on a plain task ID or
  title deletes the whole file.
- `edit` on a sub-task supports only `--title` (renames the sub-task's
  `text`). `--description`, `--due`, `--tag`, `--remove-tag`, `--source`,
  `--remove-source` are whole-task flags and are rejected with a compound
  `--id`.
- `add`, `list`, `show`, and `tasks` don't take `--id` — they operate on
  whole tasks (or, for `tasks`, list all sub-tasks at once).

---

## Date Parsing

All date inputs (`--due`, `--created-before`, `--created-after`,
`--modified-before`, `--modified-after`) accept two families of formats:

**Human-friendly (English only, parsed via `interim`):**

- Relative: `yesterday`, `today`, `tomorrow`
- Relative shifted: `in 5 days`, `3 weeks ago`, `2 months ago`,
  `last tuesday`, `next friday`, `last week`
- Month-day: `Feb 3`, `March 15` (nearest such date; year defaults to the
  current year)

**Explicit numeric (hand-parsed; the separator disambiguates the order):**

- ISO, `-` separator: `2024-03-05` → year-month-day
- European, `.` separator: `05.12.2024` → day-month-year (5 Dec)
- US, `/` separator: `11/27/2024` → month-day-year (27 Nov)

A string that parses under none of these → error `cannot parse date: "..."`.
An impossible calendar date (`31.02.2024`) is the same error.

### Timezone Handling

None of the accepted formats carry a time-of-day. At save time, the date is
resolved to **end of day (23:59:59) in the local timezone of the machine
running the command** and stored as a full RFC 3339 datetime carrying that
offset — e.g. `2024-03-15T23:59:59+01:00` — the same convention used for
`created` and `modified`. Keeping the local offset (instead of UTC) matches
"human-readable at rest": opening the file shows a sensible local due time,
not one shifted across a day boundary by UTC conversion.

Because the offset is baked in at save time, `--overdue` compares stored
instants to "now" directly. `--urgent` instead projects both the due instant
and "now" into the querying machine's local calendar. `--due <date>` exact
matching on `list` reads the stored instant's calendar date in its own
baked-in offset — the offset that was local at save time — and compares it
to the requested date. Reading the date in the stored offset, never through
another instant's offset, avoids DST reprojection errors that silently shift
the calendar date. A due date set from a different timezone therefore keeps
the calendar date of the place where it was set.

Date **filters** (`--created-before` etc.) resolve their argument the same
way (end of day, local timezone) and compare instants:
`--created-before today` means "created before tonight's end of day."

---

## Validation

Applied on `add` and `edit`, all producing exit-1 errors:

- `title` / sub-task `text`: non-empty after trimming.
- `tags` / `source` values: non-empty, no leading/trailing whitespace
  (trimmed). Duplicate values are deduplicated silently (adding an
  existing tag is a no-op, not an error; removing a missing tag likewise).
- `--project`: relative, no `..`, resolves to an existing directory under
  `$WIKI_ROOT`.

---

## Commands

Global flags (every subcommand): `--project <path>` where documented, and
`--format <text|json>` on `list` and `show` (default `text`).

### `ruwana add` / `ruwana a` / `ruwana +`

Create a new task.

```
ruwana add --project <path> "<title>" [OPTIONS]
```

**Required:**

- `--project <path>` — project path relative to wiki root (e.g.
  `godel/ai-practice`). Required for all write commands that create data.
- `<title>` — task title, positional.

**Optional flags:**

- `--description <text>` — free-text description
- `--task <text>` — add a sub-task (repeatable; each adds one `[[tasks]]`
  entry with a generated 4-char `id`, `done = false`)
- `--tag <name>` — add a tag (repeatable)
- `--due <date>` — due date (human-friendly or explicit)
- `--source <reference>` — add a source (repeatable)

Creates `.ruwana/` in the project directory if missing. Sets
`created = modified = now` (local offset).

**Output:** the created task ID on stdout, nothing else.

```sh
ruwana add --project godel/ai-practice "Review Q3 OKRs" \
  --due "next friday" --tag okr --tag review --source meeting-2024-01-15
# → a3bc9f2e
```

### `ruwana done` / `ruwana do`

Mark a task (or a single sub-task) complete.

```
ruwana done <id-or-title> [--project <path>]
ruwana done --id <task-id>[:<subtask-id>] [--project <path>]
```

- Whole task: `status` → `"done"`, `modified` bumped. Already-done → a
  **true no-op**, still exit 0: the file is not rewritten and `modified`
  is not bumped, so retries never churn the file or git history
  (idempotent — agents retry).
- Sub-task: entry's `done` → `true`, parent's `modified` bumped, parent's
  `status` untouched. Already-done sub-task → the same true no-op.

**Output:** confirmation with task ID and title (and sub-task ID/text if
addressed), e.g. `done a3bc9f2e  Review Q3 OKRs`.

### `ruwana undone` / `ruwana undo`

Inverse of `done`; same shape, same idempotency. Whole task: `status` →
`"open"`. Sub-task: `done` → `false`.

### `ruwana list` / `ruwana ls`

List tasks with optional filtering. Defaults to open tasks across all
projects.

```
ruwana list [OPTIONS]
```

**Project filter:** `--project <path>` — limit to one project.

**Status filters (mutually exclusive):**

- `--open` / `--incomplete` — open tasks only (default)
- `--done` / `--closed` / `--completed` — done tasks only
- `--all` — all tasks
- `--overdue` — open tasks whose due date is in the past (due today is
  *not* overdue — the instant is end of day, so it isn't past until
  midnight)
- `--urgent` — open tasks due today or tomorrow (querying machine's local
  calendar)

**Date filters:** `--due <date>` (exact calendar-date match, in the
stored offset), `--created-before/-after <date>`, `--modified-before/-after
<date>`.

**Tag filter:** `--tag <name>` — repeatable, **AND** semantics (task must
have all given tags). Exact-element match — tag `ok` never matches `okr`.

**Source filter:** `--source <name>` — repeatable, **OR** semantics (task
has any given source). Deliberately opposite of `--tag`: the use case is
"everything that came out of meeting A or meeting B," not narrowing by
combination.

**Sorting:** `--sort <due|created|modified|title>` — default `due`,
ascending, tasks without a due date last (then by `created` ascending as
tiebreak).

**Text output** — one task per line, column-aligned:

```
a3bc9f2e  godel/ai-practice  Review Q3 OKRs        due: 2024-03-01
b7de1c4a  books              Read Thinking Fast…   overdue
```

The due column shows `due: YYYY-MM-DD` (calendar date in the stored offset),
`overdue` for past-due open tasks, empty when no due date. No output (and
exit 0) when nothing matches.

**JSON output** (`--format json`) — a JSON array on stdout; empty array
when nothing matches:

```json
[
  {
    "id": "a3bc9f2e",
    "project": "godel/ai-practice",
    "title": "Review Q3 OKRs",
    "status": "open",
    "due": "2024-03-01T23:59:59+01:00",
    "tags": ["okr", "review"],
    "source": ["meeting-2024-01-15"],
    "created": "2024-01-15T09:32:11+01:00",
    "modified": "2024-01-20T14:05:47+01:00"
  }
]
```

`due` is `null` when unset; `tags`/`source` are `[]` when empty. `list`
deliberately excludes `description`, `related`, and sub-tasks to keep the
listing compact — use `show` for the full record.

### `ruwana show`

Display one task in full.

```
ruwana show <id-or-title> [--project <path>] [--format <text|json>]
```

- `text` (default): the task file's TOML content, verbatim, on stdout.
- `json`: the full task as one JSON object — the `list` fields plus
  `description` (string or null), `related` (array of strings), and
  `tasks` (array of `{"id", "text", "done"}`), plus `"file_path"`.

### `ruwana tasks`

List the sub-tasks of one task.

```
ruwana tasks <id-or-title> [--project <path>]
```

Prints one line per sub-task: `id`, done state, text —

```
gh7f  [ ]  Sub-task one
j2kx  [x]  Sub-task two
```

This is how you find the `<subtask-id>` for `--id <task-id>:<subtask-id>`.
A task with no sub-tasks prints nothing, exit 0.

### `ruwana edit` / `ruwana e`

Update task fields, or rename a single sub-task.

```
ruwana edit <id-or-title> [--project <path>] [OPTIONS]
ruwana edit --id <task-id>[:<subtask-id>] [--project <path>] [OPTIONS]
```

**Whole task:**

- `--title <new-title>` — rename
- `--description <text>` — replace description
- `--due <date>` — set/update due date
- `--tag <name>` / `--remove-tag <name>` — add/remove tags (repeatable)
- `--source <name>` / `--remove-source <name>` — add/remove sources
  (repeatable)

**Sub-task (compound `--id`):** `--title <new-text>` only; every other
field flag is rejected (`--due is not valid when editing a sub-task`).

At least one field flag is required (bare `edit` is a usage error). Always
bumps `modified`.

### `ruwana rm` / `ruwana remove` / `ruwana delete` / `ruwana -`

Delete a task, or a single sub-task, permanently.

```
ruwana rm <id-or-title> [--project <path>] [--force|--yes]
ruwana rm --id <task-id>[:<subtask-id>] [--project <path>] [--force|--yes]
```

Whole task: removes the TOML file. Sub-task: removes just that `[[tasks]]`
entry and bumps `modified`.

Prompts for confirmation (`delete task a3bc9f2e "…"? [y/N]` on stderr,
reads stdin) unless `--force` (alias `--yes`) is passed. If stdin is not a
TTY and `--force` is absent, `rm` **fails** with exit 1
(`refusing to delete without --force in non-interactive mode`) rather than
hanging — agents must pass `--force`, and this is documented in CLAUDE.md.

---

## Alias Table

| Canonical       | Aliases                                      |
| --------------- | -------------------------------------------- |
| `ruwana add`    | `ruwana a`, `ruwana +`                       |
| `ruwana done`   | `ruwana do`                                  |
| `ruwana undone` | `ruwana undo`                                |
| `ruwana list`   | `ruwana ls`                                  |
| `ruwana rm`     | `ruwana remove`, `ruwana delete`, `ruwana -` |
| `ruwana edit`   | `ruwana e`                                   |

---

## Error Handling

All errors print to stderr and exit non-zero. Most errors are single-line,
but argument-parsing failures include clap's Usage text and ambiguity
diagnostics may include candidate lines. Exit code is uniformly **1** for
all user-facing errors (agents branch on zero/non-zero plus the message;
finer-grained codes are not needed and would be one more thing to keep
stable). Internal failures (I/O) also exit 1 with the underlying error in
the message. `--help`/`--version` exit 0.

| Situation                                         | Exit | Message |
| ------------------------------------------------- | ---- | ------- |
| `--project` not found on disk                     | 1    | `project path not found: godel/ai-practice` |
| `--project` escapes `WIKI_ROOT` (absolute / `..`) | 1    | `invalid project path: must be relative and inside WIKI_ROOT` |
| ID not found                                      | 1    | `no task found with id: xyz` |
| ID matches tasks in multiple projects             | 1    | `ambiguous task id; use --project to narrow: xyz (project-a, project-b)` |
| Title matches 0 tasks                             | 1    | `no task found with title: "..."` |
| Title matches >1 tasks                            | 1    | `ambiguous title; use --project to narrow or use ID` (candidates listed) |
| `--id` malformed                                  | 1    | `invalid --id format, expected <task-id> or <task-id>:<subtask-id>` |
| Sub-task ID not found in task                     | 1    | `no sub-task found with id: gh7f in task abd4hg2f` |
| `--id` together with positional `<id-or-title>`   | 1    | `--id and a positional id/title are mutually exclusive` |
| Whole-task-only flag with compound `--id`         | 1    | `--due is not valid when editing a sub-task` |
| Date cannot be parsed                             | 1    | `cannot parse date: "..."` |
| Empty title / invalid tag or source value         | 1    | `invalid <field>: ...` |
| `edit` with no field flags                        | 1    | `edit requires at least one field to change` |
| `rm` non-interactive without `--force`            | 1    | `refusing to delete without --force in non-interactive mode` |
| Task file unparseable when directly targeted      | 1    | `cannot parse task file: <path>: <error>` |
| Task file unparseable during a multi-task read    | 0    | `skipping unparseable task file: <path>: <error>` (stderr only; command proceeds) |

Informational notices always go to stderr so stdout stays clean for
parsing.

---

## Testing

- **Unit tests (`ruwana-core`):** date parsing (table-driven: every
  documented format, both timezone edge directions, impossible dates), ID
  generation (charset, length, collision-retry), resolution rules (ID vs
  title, ambiguity, compound `--id`), tag/source validation, exact-element
  tag matching (`ok` vs `okr`), query filters and sort order.
- **Round-trip property:** serialize → parse → serialize is a fixpoint for
  any valid task.
- **Integration tests (`crates/cli/tests/`, via `assert_cmd` +
  `tempfile`):** each command end-to-end against a temp `WIKI_ROOT` —
  including out-of-band changes (add/edit/delete a task file behind the
  tool's back, assert the next `list` reflects it immediately), the
  skip-with-warning path for a corrupt file, the non-interactive `rm`
  refusal, `--format json` schema shape, and exit codes/messages from the
  table above.
- Timezone-sensitive tests pin `TZ` explicitly rather than inheriting the
  host's.

---

## Roadmap

### Must-Have (P0 — v1 ships with these)

- [ ] `add`, `done`, `undone`, `list`, `rm`, `show`, `tasks`, `edit` + all aliases
- [ ] `--project` flag on all write commands; project-path validation
- [ ] ID + title resolution for `done`, `undone`, `rm`, `show`, `tasks`, `edit`
      (ID = filename `stat`; title = in-memory exact match)
- [ ] Direct-read query engine: project discovery walk, parallel TOML
      parse, in-memory filter/sort; corrupt files skipped with warning
- [ ] Human-friendly (`interim`) and explicit (separator-disambiguated)
      date parsing
- [ ] Due dates resolved to end-of-day in the saving machine's local
      timezone, stored with explicit offset
- [ ] `--overdue`, `--urgent`, all date-range filters, `--due` exact match
- [ ] Tag filtering (AND, exact-element) and source filtering (OR) on `list`
- [ ] `source` field (`--source` on `add`/`edit`, `--remove-source`,
      `--source` filter)
- [ ] Sub-tasks via `--task`, each with a generated 4-char `id`
- [ ] `--id <task-id>[:<subtask-id>]` on `done`, `undone`, `rm`, `edit`,
      with whole-task-flag rejection on sub-task edits
- [ ] `WIKI_ROOT` env var (default `~/wiki`)
- [ ] Cargo workspace with `ruwana-core` / `ruwana-cli` separation per
      Architecture; all file access behind `store.rs` (the future-cache
      seam)
- [ ] `--format json` on `list` and `show`
- [ ] `--sort` on `list` (due, created, modified, title)
- [ ] `--force`/`--yes` on `rm`; non-interactive refusal without it
- [ ] Atomic file writes; idempotent `done`/`undone`
- [ ] Test suite per Testing section

### Nice-to-Have (P1 — fast follow)

- [ ] `ruwana add --interactive` for human use (opens `$EDITOR`)
- [ ] `--limit` flag on `list`
- [ ] `ruwana edit --add-task` for adding a sub-task to an existing task
- [ ] Completion scripts (bash, zsh, fish)
- [ ] `--remove-due` flag on `edit`
- [ ] Recurring tasks

### Future (P2 — planned, after P0 and P1)

- [ ] `ruwana tui` — interactive terminal UI (likely `ratatui`), added as a
      third workspace crate: list navigation, toggle done/undone, inline
      edit, sub-task view, delete with confirmation. Must call
      `ruwana-core`'s `ops` API directly — never shell out to the CLI or
      reimplement task logic.
- [ ] `--priority` field (low/medium/high) as a separate axis from urgency:
      `--urgent`/`--overdue` are about *when*; priority is about how much
      it matters regardless of date.

### Someday / Maybe

Not planned, not ruled out: notifications/reminders, sync to external
services (Linear, GitHub Issues), GUI (web or native), and a global
read-through query cache (SQLite in the XDG cache dir, behind `store.rs`)
if a wiki ever outgrows the direct-read engine — see Query Engine.

### Out of Scope

- Multi-user collaboration
