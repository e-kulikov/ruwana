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
- TOML files as the single source of truth; SQLite as a rebuildable cache.
- Per-project isolation: everything about a project's tasks lives under that
  project's directory and travels (or dies) with it.
- Machine-parseable output (`--format json`) as a first-class surface.
- Core logic in a library crate so a future TUI reuses it directly.

**Non-Goals (v1):**

- Multi-user collaboration, sync to external services, notifications, GUI.
- Recurring tasks, priorities, interactive editing (deferred — see Roadmap).
- Preserving comments/formatting in hand-edited TOML files (see Task File
  Format → Canonical serialization).

---

## Design Principles

- **Agent-first interface**: every command must be unambiguous and
  scriptable. No interactive prompts (the sole exception, `rm`, has a
  documented `--force` bypass). Output must be parseable.
- **File-system native**: TOML files are the source of truth. The SQLite
  index is derived, not primary — it can always be rebuilt from files.
- **Project isolation**: tasks are scoped to projects via `--project`, which
  maps to a directory path within the wiki. Work tasks die with their
  submodule. Each project carries its own index; there is no global state
  outside `$WIKI_ROOT`.
- **Human-readable at rest**: a task file opened in any editor is legible
  without tooling.
- **Rebuild-safe**: `ruwana index --rebuild` fully reconstructs the SQLite
  database from the TOML files at any time; nothing lives only in SQLite.
- **Library-first**: all task logic (CRUD, ID/sub-task resolution, date
  parsing, index sync) lives in the library layer, separate from the CLI
  argument-parsing layer. The `ruwana` binary is a thin wrapper over that
  library. This is a v1 requirement, not a later refactor — it's what lets a
  future `ruwana tui` call the same functions directly instead of shelling
  out to the CLI or duplicating logic.

---

## Architecture

### Crate layout

A single Cargo package with two targets:

```
ruwana/
├── Cargo.toml
├── src/
│   ├── lib.rs          ← library crate: all logic lives here
│   ├── main.rs         ← binary crate: clap parsing + calls into lib + printing
│   ├── model.rs        ← Task, SubTask, Status types (serde)
│   ├── store.rs        ← TOML file read/write, .ruwana dir handling
│   ├── index.rs        ← SQLite index: open, sync, rebuild, staleness check
│   ├── discover.rs     ← WIKI_ROOT walk, project discovery
│   ├── resolve.rs      ← ID / title / compound-ID resolution
│   ├── dates.rs        ← natural + explicit date parsing, EOD resolution
│   ├── ids.rs          ← ID generation (8-char task, 4-char sub-task)
│   └── ops.rs          ← command-level API: add, done, undone, list, edit, rm…
├── hooks/              ← git hook templates (post-merge, post-checkout)
└── tests/              ← integration tests (assert_cmd against a temp WIKI_ROOT)
```

The binary (`main.rs`) does exactly three things: parse args with clap, call
one `ops::*` function, format the result (text or JSON). No task logic in
`main.rs`. The `ops` module's function signatures take plain Rust types (no
clap types) and return `Result<T, Error>` with typed errors — this is the
API a future TUI consumes.

*Why one package, not a workspace:* the lib/bin split inside one package
already enforces the boundary (the binary can only use the library's public
API). A workspace split (`ruwana-core` + `ruwana-cli`) is a mechanical
refactor if the TUI ever wants a separate crate; doing it now adds
ceremony with no v1 benefit.

### Dependencies

| Concern | Crate | Notes |
| --- | --- | --- |
| CLI parsing | `clap` v4 (derive) | subcommands, aliases, arg groups for mutual exclusion |
| TOML | `toml` (serde) | canonical serialization; see Task File Format |
| SQLite | `rusqlite` with `bundled` feature | no system sqlite dependency |
| Dates/times | `chrono` | local-offset ISO 8601 timestamps |
| Natural dates | `interim` | maintained fork of `chrono-english` (zk uses the Go analogue `tj/go-naturaldate`) |
| Randomness | `rand` | ID generation |
| JSON output | `serde_json` | `--format json` |
| Errors | `thiserror` (lib) + `anyhow` (bin) | typed errors in the library API |
| Walking | `walkdir` | project discovery |
| Tests | `assert_cmd`, `predicates`, `tempfile` | integration tests |

Explicit numeric-date parsing (`2024-03-05`, `05.12.2024`, `11/27/2024`) is
hand-rolled (~30 lines) rather than delegated, because the separator itself
disambiguates day/month order — see Date Parsing.

---

## Storage Layout

```
$WIKI_ROOT/<project>/.ruwana/
├── index.db           ← SQLite index (gitignored, rebuilt automatically)
├── abc1de2f.toml      ← one file per task, named by ID
├── gh3ij4kl.toml
└── ...
```

`.ruwana/` lives inside each project directory. The `--project` flag
specifies the project path relative to the wiki root (e.g.
`godel/ai-practice`), which resolves to
`$WIKI_ROOT/godel/ai-practice/.ruwana/`.

`WIKI_ROOT` is read from the `WIKI_ROOT` environment variable, defaulting to
`~/wiki`.

Rules:

- The **project directory** must already exist — `ruwana` never creates
  project directories (`project path not found` otherwise). The `.ruwana/`
  directory inside it, however, is created on demand by the first `add`.
- The index is **per project**: each `.ruwana/` has its own `index.db`.
  There is no global database. This is what makes "tasks die with their
  submodule" literally true — deleting or de-initializing a submodule
  removes every trace of its tasks, index included.
- `--project` must be a relative path with no `..` components; it is
  resolved strictly under `$WIKI_ROOT` (reject anything that escapes it).

### Project discovery

Commands that operate across all projects (`list` without `--project`, ID
lookup without `--project`, `index --rebuild` without `--project`) discover
projects by walking `$WIKI_ROOT` recursively looking for `.ruwana`
directories, with these rules:

- Skip hidden directories (dot-prefixed) during descent — except `.ruwana`
  itself, which is the match target. `.git` is therefore skipped for free.
- Do not descend *into* a `.ruwana` directory looking for nested projects.
- Follow the directory tree only (no symlink following), to keep the walk
  cheap and cycle-free.

A personal wiki is small (hundreds of directories); a full walk per
invocation is well under perceptible latency and avoids any registry file
that could go stale.

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
  path (or the index's `project` column).

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
supported (the file parses fine and the stale-index fallback picks it up)
but hand-*decorating* is not. Anything worth keeping belongs in
`description` or `related`.

**Writes are atomic**: write to `<id>.toml.tmp` in the same directory, then
rename over the target. A crash mid-write never leaves a corrupt task file.

**Sub-task IDs.** Each `[[tasks]]` entry has its own `id` — 4 characters
instead of 8 (see ID Generation), since a sub-task ID only needs to be
unique within its parent's `tasks` array; it's always addressed together
with the parent ID (see "Addressing a Sub-task").

---

## SQLite Schema

Each project's `index.db`:

```sql
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- rows: ('schema_version', '1')

CREATE TABLE todos (
  id          TEXT PRIMARY KEY,
  project     TEXT NOT NULL,   -- derived from the directory path, not stored in TOML
  title       TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'open',  -- 'open' | 'done'
  due         TEXT,            -- RFC 3339 with offset, or NULL
  tags        TEXT,            -- ',tag1,tag2,' — comma-wrapped, see below
  source      TEXT,            -- same encoding as tags
  created_at  TEXT NOT NULL,   -- RFC 3339 with offset
  modified_at TEXT NOT NULL,
  file_path   TEXT NOT NULL,   -- absolute path to the .toml file
  file_mtime  INTEGER NOT NULL,-- mtime (ns since epoch) at index time
  file_size   INTEGER NOT NULL -- size in bytes at index time
);

CREATE INDEX idx_status ON todos(status);
CREATE INDEX idx_due    ON todos(due);
```

- `tags`/`source` are stored **comma-wrapped** (`,a,b,` — leading and
  trailing delimiter included) so exact-element matching is a plain
  `tags LIKE '%,' || ? || ',%'` with no substring false positives (`ok`
  must not match `okr`). NULL when empty. Commas inside tag/source values
  are rejected at input time (see Validation).
- `file_mtime`/`file_size` power staleness detection (see Index Sync).
- No `project` index — within one per-project database the column is
  constant; it exists so rows from multiple databases can be merged in
  memory without re-deriving the project from the path.
- `schema_version` mismatch (older/newer binary) triggers a silent full
  rebuild of that database, same path as a missing index.

No separate table for sub-tasks. Sub-tasks aren't indexed individually — a
`<task-id>:<subtask-id>` lookup resolves `task-id` against `todos.file_path`
exactly as a normal ID lookup does, then reads/mutates the matching
`[[tasks]]` entry directly in the TOML file. The file stays the single
source of truth for sub-task state, and `index --rebuild` needs no schema
change.

---

## Index Sync

`index.db` is **gitignored** — never committed. It's a derived cache: a
fresh clone or a machine missing it just triggers a rebuild. The wiki's
`.gitignore` should contain `**/.ruwana/index.db`.

Three layers keep it correct:

1. **Inline writes.** Every mutation made through `ruwana` updates the index
   in the same process, right after the TOML file is written (including
   refreshing `file_mtime`/`file_size`). Updating the index is the last
   step of `add`/`done`/`undone`/`edit`/`rm`, not a background job.

2. **Git hooks** (`post-merge`, `post-checkout`) call
   `ruwana index --rebuild --project <path>`, so a `git pull` /
   `submodule update` / branch switch resyncs the index as part of the git
   operation. Templates ship in the repo's `hooks/` directory; installation
   is manual (copy or symlink into `.git/hooks/`), documented in the README
   and CLAUDE.md. This is convenience, not correctness — layer 3 guarantees
   correctness with no hooks installed.

3. **Stale-index fallback.** Before serving any command from a project's
   index, `ruwana` compares the directory's actual state against the index:

   - List `.ruwana/*.toml` and compare the set of `(path, mtime_ns, size)`
     triples against the indexed `(file_path, file_mtime, file_size)` rows.
   - Any difference — new file, missing file, changed mtime or size —
     triggers a silent rebuild of **that project's** index before the
     command proceeds (`index stale, rebuilding...` on stderr).
   - A missing or wrong-schema-version `index.db` triggers the same rebuild
     (`index not found, rebuilding...`).

   The check is one `readdir` + `stat` per file per invocation — negligible
   for personal-wiki scale — and makes correctness independent of hooks and
   robust against hand edits.

**Concurrency.** `ruwana` assumes a single user but not a single process
(an agent and a human can race). SQLite is opened with a 5-second
`busy_timeout`; the atomic TOML rename makes file writes safe. There is no
cross-file transactionality — the worst case for a true race is a stale
index row, which layer 3 self-heals on the next command. No lock files.

---

## ID Generation

- Task IDs: 8 characters from `[a-z0-9]` (36 symbols; 36⁸ ≈ 2.8 × 10¹²),
  generated from a CSPRNG. Example: `a3bc9f2e`.
- On generation, the ID is checked against every discovered project's index
  (collision → regenerate; at this keyspace a collision is effectively
  never, but the check is one query per project and keeps "globally unique"
  honest).
- IDs are never reused: deletion removes the file, and new IDs are random,
  not sequential.
- Sub-task IDs: same charset, 4 characters (36⁴ ≈ 1.68 million), unique
  only within the parent's `tasks` array (collision within the array →
  regenerate). Example: `gh7f`. Global uniqueness would be overkill: a
  sub-task is always addressed together with its parent ID.

---

## Resolution Rules

Every command that takes `<id-or-title>` resolves it in this order:

1. If the argument is exactly 8 chars of `[a-z0-9]`, try it as an ID first
   (against the given project's index, or every discovered project's index
   when `--project` is absent). Exactly one match → resolved.
2. Otherwise (or if no ID matched), treat it as a title: **exact,
   case-sensitive** string match on `title`, scoped by `--project` if given.
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
- The compound form resolves `<task-id>` first (via the index, like any ID
  lookup), then finds `<subtask-id>` among that task's `[[tasks]]` entries.
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
- `add`, `list`, `show`, `tasks`, and `index` don't take `--id` — they
  operate on whole tasks (or, for `tasks`, list all sub-tasks at once).

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

Because the offset is baked in at save time, reads don't re-interpret
anything — `--overdue` and `--urgent` compare the stored instant to "now"
directly, regardless of the querying machine's timezone. The one read-time
timezone rule is `--due <date>` exact matching on `list`: the stored
instant's calendar date is computed **in the querying machine's local
timezone** and compared to the requested date — so `--due today` means
"today where I'm asking from," even if the due date was set from another
timezone.

Date **filters** (`--created-before` etc.) resolve their argument the same
way (end of day, local timezone) and compare instants:
`--created-before today` means "created before tonight's end of day."

---

## Validation

Applied on `add` and `edit`, all producing exit-1 errors:

- `title` / sub-task `text`: non-empty after trimming.
- `tags` / `source` values: non-empty, no commas (reserved as the index
  delimiter), no leading/trailing whitespace (trimmed). Duplicate values
  are deduplicated silently (adding an existing tag is a no-op, not an
  error; removing a missing tag likewise).
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

- Whole task: `status` → `"done"`, `modified` bumped. Already-done → no-op,
  still exit 0 (idempotent — agents retry).
- Sub-task: entry's `done` → `true`, parent's `modified` bumped, parent's
  `status` untouched.

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

**Date filters:** `--due <date>` (exact calendar-date match, querying
timezone), `--created-before/-after <date>`, `--modified-before/-after
<date>`.

**Tag filter:** `--tag <name>` — repeatable, **AND** semantics (task must
have all given tags).

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

The due column shows `due: YYYY-MM-DD` (calendar date, querying timezone),
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
serves from the index and does not include `description`, `related`, or
sub-tasks — use `show` for the full record.

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

Whole task: removes the TOML file and its index row. Sub-task: removes just
that `[[tasks]]` entry and bumps `modified`.

Prompts for confirmation (`delete task a3bc9f2e "…"? [y/N]` on stderr,
reads stdin) unless `--force` (alias `--yes`) is passed. If stdin is not a
TTY and `--force` is absent, `rm` **fails** with exit 1
(`refusing to delete without --force in non-interactive mode`) rather than
hanging — agents must pass `--force`, and this is documented in CLAUDE.md.

### `ruwana index`

Manage the SQLite index.

```
ruwana index --rebuild [--project <path>]
```

- `--rebuild` — drop and recreate the index by scanning `.ruwana/*.toml`
  files. Without `--project`, rebuilds every discovered project's index.
- A TOML file that fails to parse during rebuild is **skipped with a
  warning on stderr** (`skipping unparseable task file: <path>: <error>`);
  the rebuild continues and exits 0. A broken file must not brick the whole
  project's index.

Useful after: manual edits, git clone, submodule init, index corruption.

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

All errors print a single-line message to stderr and exit non-zero. Exit
code is uniformly **1** for all user-facing errors (agents branch on
zero/non-zero plus the message; finer-grained codes are not needed and
would be one more thing to keep stable). Internal failures (I/O, SQLite)
also exit 1 with the underlying error in the message.

| Situation                                         | Exit | Message |
| ------------------------------------------------- | ---- | ------- |
| `--project` not found on disk                     | 1    | `project path not found: godel/ai-practice` |
| `--project` escapes `WIKI_ROOT` (absolute / `..`) | 1    | `invalid project path: must be relative and inside WIKI_ROOT` |
| ID not found                                      | 1    | `no task found with id: xyz` |
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
| Index missing (auto-rebuild)                      | 0    | `index not found, rebuilding...` (stderr only) |
| Index stale (auto-rebuild)                        | 0    | `index stale, rebuilding...` (stderr only) |

The index is auto-rebuilt silently on mismatch; operations never fail
because of a stale index. Informational notices always go to stderr so
stdout stays clean for parsing.

---

## Testing

- **Unit tests (library):** date parsing (table-driven: every documented
  format, both timezone edge directions, impossible dates), ID generation
  (charset, length, collision-retry), resolution rules (ID vs title,
  ambiguity, compound `--id`), tag/source validation, comma-wrapped
  matching.
- **Round-trip property:** serialize → parse → serialize is a fixpoint for
  any valid task; `index --rebuild` over a directory of files produces an
  index equivalent to the one built by inline writes.
- **Integration tests (`tests/`, via `assert_cmd` + `tempfile`):** each
  command end-to-end against a temp `WIKI_ROOT` — including the stale-index
  fallback (touch a file behind the index's back, assert the next `list`
  still returns correct data and prints the rebuild notice), the
  non-interactive `rm` refusal, `--format json` schema shape, and exit
  codes/messages from the table above.
- Timezone-sensitive tests pin `TZ` explicitly rather than inheriting the
  host's.

---

## Roadmap

### Must-Have (P0 — v1 ships with these)

- [ ] `add`, `done`, `undone`, `list`, `rm`, `show`, `tasks`, `edit` + all aliases
- [ ] `--project` flag on all write commands; project-path validation
- [ ] ID + title resolution for `done`, `undone`, `rm`, `show`, `tasks`, `edit`
- [ ] Per-project SQLite index with mtime/size staleness detection and
      auto-rebuild on missing/stale/schema-mismatch
- [ ] `ruwana index --rebuild` (scoped and global), unparseable files
      skipped with warning
- [ ] Human-friendly (`interim`) and explicit (separator-disambiguated)
      date parsing
- [ ] Due dates resolved to end-of-day in the saving machine's local
      timezone, stored with explicit offset
- [ ] `--overdue`, `--urgent`, all date-range filters, `--due` exact match
- [ ] Tag filtering (AND) and source filtering (OR) on `list`
- [ ] `source` field (`--source` on `add`/`edit`, `--remove-source`,
      `--source` filter)
- [ ] Sub-tasks via `--task`, each with a generated 4-char `id`
- [ ] `--id <task-id>[:<subtask-id>]` on `done`, `undone`, `rm`, `edit`,
      with whole-task-flag rejection on sub-task edits
- [ ] `WIKI_ROOT` env var (default `~/wiki`)
- [ ] `index.db` gitignored; git hook templates (`post-merge`,
      `post-checkout`) in `hooks/`
- [ ] Library/binary separation per Architecture (no TUI code, just the boundary)
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

- [ ] `ruwana tui` — interactive terminal UI (likely `ratatui`): list
      navigation, toggle done/undone, inline edit, sub-task view, delete
      with confirmation. Must call the library crate's `ops` API directly —
      never shell out to the CLI or reimplement task logic.
- [ ] `--priority` field (low/medium/high) as a separate axis from urgency:
      `--urgent`/`--overdue` are about *when*; priority is about how much
      it matters regardless of date.

### Someday / Maybe

Not planned, not ruled out: notifications/reminders, sync to external
services (Linear, GitHub Issues), GUI (web or native).

### Out of Scope

- Multi-user collaboration
