# `ruwana` — CLI Specification

> Agent-facing task tracker for a personal wiki system.
> Implementation language: Rust.

---

## Naming

The tool is called **`ruwana`** — Quechua for "the thing that must be done."
It's *ruway* ("to do/make") plus the nominalizing suffix *-na*, the same
pattern that turns *mikhuy* ("to eat") into *mikhuna* ("food," literally
"the thing to be eaten"). It's the closest single-word Quechua counterpart
to Latin's *agendum*.

**Pronunciation:** /ruˈwana/ — stress on the middle syllable (Quechua stress
defaults to the penultimate syllable). Roughly **"roo-WAH-nah"** in English
spelling; the *r* is a light tap, closer to the *r* in Spanish "pero" than
to an English *r*.

---

## Problem Statement

The personal wiki system (LLM-maintained markdown + zk) needs task management
that is: agent-reliable (consistent format every session), human-readable
(TOML files on disk), project-aware (tasks scoped to wiki subprojects),
and self-contained (no external service dependencies). Existing tools (Taskwarrior,
GitHub Issues) don't integrate cleanly with the git-submodule project isolation
model where work tasks must die with their submodule.

---

## Design Principles

- **Agent-first interface**: every command must be unambiguous and scriptable.
  No interactive prompts. Output must be parseable.
- **File-system native**: TOML files are the source of truth. The SQLite
  index is derived, not primary — it can always be rebuilt from files.
- **Project isolation**: tasks are scoped to projects via `--project`, which
  maps to a directory path within the wiki. Work tasks die with their submodule.
- **Human-readable at rest**: a task file opened in any editor is legible
  without tooling.
- **Rebuild-safe**: `ruwana index --rebuild` should fully reconstruct the SQLite
  database from the TOML files at any time.
- **Library-first**: all task logic (CRUD, ID/sub-task resolution, date
  parsing, index sync) lives in a library crate, separate from the CLI
  argument-parsing layer. The `ruwana` binary is a thin wrapper over that
  library. This is a v1 requirement, not a later refactor — it's what lets a
  future `ruwana tui` (see "Future" below) call the same functions directly
  instead of shelling out to the CLI or duplicating logic.

---

## Storage Layout

```
$WIKI_ROOT/<project>/.ruwana/
├── index.db           ← SQLite index (gitignored, rebuilt automatically)
├── abc1de2f.toml       ← one file per task, named by ID
├── gh3ij4kl.toml
└── ...
```

`.ruwana/` lives inside each project directory. The `--project` flag specifies
the project path relative to the wiki root (e.g. `godel/ai-practice`), which
resolves to `$WIKI_ROOT/godel/ai-practice/.ruwana/`.

`WIKI_ROOT` is read from the `WIKI_ROOT` environment variable, defaulting to
`~/wiki`.

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

[[tasks]]
id   = "t7mz"
text = "Sub-task three"
done = true
```

There is no `project` field in the file. The project is not data belonging to
the task — it's the `<project>` path segment the file already lives under
(`$WIKI_ROOT/<project>/.ruwana/<id>.toml`), so storing it again inside the file
would just be a duplicate that could drift out of sync. Any command that needs
a task's project reads it off the file's path (or, when going through the
index, off the `project` column — see SQLite Schema below).

`source` is a separate, structured field from `related`: it's specifically
what `--source` on `add`/`edit` writes to, and it's what `--source` on `list`
filters against (see below). `related` stays free text for anything else
worth linking — a Linear issue, a wiki note — that isn't necessarily "the
thing that produced this task." A meeting transcript that spawned a task
would typically show up in both: `source` so it's filterable, `related` if
you also want a direct link to it.

When a task is marked done, `status` changes to `"done"` and `modified` is
updated. No other field changes shape.

**Sub-task IDs.** Each `[[tasks]]` entry has its own `id` — 4 characters
instead of 8 (see ID Generation), since a sub-task ID only needs to be unique
within its parent's `tasks` array, not globally; it's always addressed
together with the parent ID (see "Addressing a Sub-task" below).

---

## SQLite Schema

```sql
CREATE TABLE todos (
  id          TEXT PRIMARY KEY,
  project     TEXT NOT NULL,                  -- derived from the file's
                                                -- directory path, not stored
                                                -- in the TOML file itself
  title       TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'open',  -- 'open' | 'done'
  due         TEXT,                           -- ISO 8601 datetime with UTC
                                               -- offset, or NULL (see Date
                                               -- Parsing → Timezone Handling)
  tags        TEXT,                           -- comma-separated
  source      TEXT,                           -- comma-separated
  created_at  TEXT NOT NULL,                  -- ISO 8601 with timezone
  modified_at TEXT NOT NULL,
  file_path   TEXT NOT NULL                   -- absolute path to .toml file
);

CREATE INDEX idx_project   ON todos(project);
CREATE INDEX idx_status    ON todos(status);
CREATE INDEX idx_due       ON todos(due);
CREATE INDEX idx_tags      ON todos(tags);
CREATE INDEX idx_source    ON todos(source);
```

No separate table for sub-tasks. Sub-tasks aren't indexed individually — a
`<task-id>:<subtask-id>` lookup resolves `task-id` against `todos.file_path`
exactly as a normal ID lookup does, then reads/mutates the matching
`[[tasks]]` entry directly in the TOML file. This keeps the file as the
single source of truth for sub-task state and needs no schema change for
`index --rebuild` to keep working as-is.

---

## Index Sync

`index.db` is **gitignored** — never committed. It's a derived cache, not
data: a fresh clone or a machine that's missing it just triggers a rebuild
(already specified in Error Handling). The wiki's `.gitignore` should have
an entry for `**/.ruwana/index.db`.

Two things keep it from lagging behind:

1. **Writes made through `ruwana` update the index inline**, in the same
   process, right after the TOML file is written. There's no delay for
   anything done via the CLI — updating the index is just the last step of
   `add`/`done`/`undone`/`edit`/`rm`, not a separate background job.

2. **Changes made outside `ruwana` need a trigger**, since nothing runs the
   update step for them. The main case is files changing because of
   `git pull` / `submodule update` / `checkout` bringing in edits from
   elsewhere (another machine, a collaborator, CI) — hand-editing a `.toml`
   file directly is unlikely enough in practice not to warrant its own
   mechanism. Two layers:
   - **Git hooks** (`post-merge`, `post-checkout`) call
     `ruwana index --rebuild --project <path>` scoped to what changed, so a
     `git pull` resyncs the index as part of the pull itself rather than
     waiting for the next unrelated `ruwana` command. This is just the
     existing `--rebuild` machinery wired into git — cheap to add, P0.
   - **Stale-index fallback** (already in Error Handling) is the safety net
     underneath: if anything still slips through — a hand-edit, a hook that
     didn't run — the next `ruwana` command notices the index doesn't match
     the files it's about to touch and silently rebuilds before proceeding.
     This alone is enough to guarantee correctness even with no hooks
     installed at all; the hooks just mean you don't wait for it.

---

## ID Generation

- 8-character alphanumeric ID (lowercase letters + digits, base-36 style)
- Generated with sufficient randomness to be globally unique across all
  projects and notebooks
- Example: `a3bc9f2e`
- IDs are never reused, even after a task is deleted
- Sub-task IDs use the same charset but are only 4 characters (36^4 ≈ 1.68
  million combinations) — uniqueness there is only required within one
  task's own `tasks` array, not globally, so 8 characters would be overkill.
  Example: `gh7f`

---

## Addressing a Sub-task

Commands where operating on a single sub-task makes sense — `done`, `undone`,
`rm`, `edit` — accept an explicit `--id` flag as an alternative to the
positional `<id-or-title>` argument:

```
--id <task-id>              # same as the positional task ID
--id <task-id>:<subtask-id> # targets one sub-task within that task
```

```sh
ruwana done --id abd4hg2f:gh7f
```

Rules:

- `--id` and the positional `<id-or-title>` are mutually exclusive.
- The compound form always resolves the `<task-id>` part first (via the
  SQLite index, same as any other ID lookup), then looks up `<subtask-id>`
  among that task's `[[tasks]]` entries.
- `--project` stays optional with `--id`, same as with a plain ID — the ID
  half is already globally unique. Title-based resolution does not apply to
  sub-tasks (they have no title of their own), so `--id` is the only way to
  target one.
- `done` / `undone` on a sub-task flips that entry's `done` field only; the
  parent task's own `status` is untouched, and `modified` is bumped on the
  parent.
- `rm` on a sub-task removes just that `[[tasks]]` entry from the parent file
  (and bumps `modified`) — it does not touch the parent task or its file.
  `rm` on a plain task ID or title still deletes the whole file, as before.
- `edit` on a sub-task only supports `--title` (renames the sub-task's
  `text`). `--description`, `--due`, `--tag`, and `--remove-tag` apply to
  whole tasks only and are rejected when combined with a compound `--id`.
- `add`, `list`, `show`, `tasks`, and `index` don't take `--id` — they operate
  on whole tasks (or, for `tasks`, list all sub-tasks of one task at once), so
  a compound ID doesn't apply.

---

## Date Parsing

All date inputs (for `--due`, `--created-before`, `--created-after`,
`--modified-before`, `--modified-after`, `--due`) support two formats:

**Human-friendly (English only):**
- Relative: `yesterday`, `today`, `tomorrow`
- Relative shifted: `in 5 days`, `3 weeks ago`, `2 months ago`, `last tuesday`,
  `next friday`, `last week`
- Month-day: `Feb 3`, `March 15`

**Explicit formats:**
- ISO: `2024-03-05`
- European: `05.12.2024`
- US: `11/27/2024`

All dates are parsed into a concrete instant and stored as ISO 8601 in the
file and database — see Timezone Handling below for exactly how.

Reference implementation: zk uses
[`github.com/tj/go-naturaldate`](https://github.com/tj/go-naturaldate) for
this; a Rust equivalent is `chrono-english` or similar.

### Timezone Handling

None of the accepted `--due` formats carry a time-of-day, just a date. At
save time, that date is resolved to **end of day (23:59:59) in the local
timezone of the machine running the command**, and stored as a full ISO 8601
datetime carrying that offset — e.g. `2024-03-15T23:59:59+01:00` — the same
convention already used for `created` and `modified`, rather than converting
to UTC. Keeping the local offset (instead of UTC) matches the
"human-readable at rest" principle: opening the file shows a sensible local
due date/time, not one shifted to a different day depending on where UTC
midnight falls. It also matches the recommendation that "due today" reflects
the timezone of whoever set the due date, at the moment they set it.

Because the offset is baked in at save time, later reads don't need to
re-interpret anything — `--overdue` and `--urgent` just compare the stored
instant to "now" directly, regardless of what timezone the machine running
`list` happens to be in. The one place timezone still matters at *read* time
is `--due <date>` exact-match filtering on `list`: since the stored value is
a full instant, matching by date compares the calendar-date portion
converted into the *querying* machine's local timezone — so `--due today`
means "today where I'm asking from" even if the due date was originally set
from a different timezone (e.g. while traveling).

`created` and `modified` already carry an explicit offset for the same
reason and need no special handling beyond ordinary instant comparison.

---

## Commands

### `ruwana add` / `ruwana a` / `ruwana +`

Create a new task.

```
ruwana add --project <path> "<title>" [OPTIONS]
```

**Required:**
- `--project <path>` — project path relative to wiki root (e.g. `godel/ai-practice`).
  May contain `/`. Required for all write commands.
- `<title>` — task title as positional string argument

**Optional flags:**
- `--description <text>` — free-text description (stored in the `description` field)
- `--task <text>` — add a sub-task (repeatable; each `--task` adds one `[[tasks]]` entry)
- `--tag <name>` — add a tag (repeatable)
- `--due <date>` — due date (human-friendly or explicit)
- `--source <reference>` — add a source (repeatable; stored in the `source`
  array — see Task File Format)

**Output:** prints the created task ID on success.

```sh
ruwana add --project godel/ai-practice "Review Q3 OKRs" \
  --due "next friday" \
  --tag okr --tag review \
  --source meeting-2024-01-15

# output: a3bc9f2e
```

---

### `ruwana done` / `ruwana do`

Mark a task (or a single sub-task) as complete.

```
ruwana done <id-or-title> [--project <path>]
ruwana done --id <task-id>[:<subtask-id>] [--project <path>]
```

- Accepts either the task ID or the exact task title, or the `--id` flag (see
  "Addressing a Sub-task")
- `--project` is optional when using ID (ID is globally unique); required when
  using title (to disambiguate identical titles across projects)
- On a whole task: updates `status` → `"done"`, `modified`
- On a sub-task (`--id <task-id>:<subtask-id>`): updates that entry's `done`
  → `true`, and bumps the parent's `modified`; the parent's own `status` is
  unaffected

**Output:** prints confirmation with task ID and title (and sub-task ID/text,
if addressed).

---

### `ruwana undone` / `ruwana undo`

Remove the done mark from a task or a single sub-task (reopen it).

```
ruwana undone <id-or-title> [--project <path>]
ruwana undone --id <task-id>[:<subtask-id>] [--project <path>]
```

- Inverse of `done`
- On a whole task: updates `status` → `"open"`, `modified`
- On a sub-task: updates that entry's `done` → `false`, bumps the parent's
  `modified`

---

### `ruwana list` / `ruwana ls`

List tasks with optional filtering. Defaults to open tasks across all projects.

```
ruwana list [OPTIONS]
```

**Project filter:**
- `--project <path>` — limit to one project

**Status filters (mutually exclusive):**
- `--open` / `--incomplete` — open tasks only (default)
- `--done` / `--closed` / `--completed` — done tasks only
- `--all` — all tasks regardless of status
- `--overdue` — open tasks whose due date is in the past (today is not overdue)
- `--urgent` — open tasks due today or tomorrow

**Date filters (all accept human-friendly format):**
- `--due <date>` — exact due date match
- `--created-before <date>`
- `--created-after <date>`
- `--modified-before <date>`
- `--modified-after <date>`

**Tag filter:**
- `--tag <name>` — filter by tag (repeatable; AND semantics — matches tasks
  that have all given tags)

**Source filter:**
- `--source <name>` — filter by source (repeatable; OR semantics — matches
  tasks that have any of the given sources). This is deliberately the
  opposite of `--tag`'s AND: the use case is "show me everything that came
  out of meeting A or meeting B," not narrowing down by combination.

**Output format:**
```
a3bc9f2e  godel/ai-practice  Review Q3 OKRs          due: 2024-03-01
b7de1c4a  books              Read Thinking Fast…      overdue
```

---

### `ruwana show`

Display full details of a single task.

```
ruwana show <id-or-title> [--project <path>]
```

Prints the full TOML content of the task file to stdout.

---

### `ruwana tasks`

List the sub-tasks of a given task.

```
ruwana tasks <id-or-title> [--project <path>]
```

Prints each sub-task's `id`, `done` state, and `text` — this is how you find
the `<subtask-id>` to use with `--id <task-id>:<subtask-id>` on other
commands.

---

### `ruwana edit` / `ruwana e`

Update task fields, or rename a single sub-task.

```
ruwana edit <id-or-title> [--project <path>] [OPTIONS]
ruwana edit --id <task-id>[:<subtask-id>] [--project <path>] [OPTIONS]
```

**Editable fields (whole task):**
- `--title <new-title>` — rename the task
- `--description <text>` — replace description
- `--due <date>` — update or set due date
- `--tag <name>` — add a tag (repeatable); use `--remove-tag <name>` to remove
- `--remove-tag <name>` — remove a tag (repeatable)
- `--source <name>` — add a source (repeatable); use `--remove-source <name>`
  to remove
- `--remove-source <name>` — remove a source (repeatable)

**Editable fields (sub-task, `--id <task-id>:<subtask-id>`):**
- `--title <new-text>` — rename the sub-task's `text`
- `--description`, `--due`, `--tag`, `--remove-tag`, `--source`,
  `--remove-source` are invalid in combination with a compound `--id` and
  are rejected

Always updates `modified` timestamp.

---

### `ruwana rm` / `ruwana remove` / `ruwana delete` / `ruwana -`

Delete a task, or a single sub-task, permanently.

```
ruwana rm <id-or-title> [--project <path>]
ruwana rm --id <task-id>[:<subtask-id>] [--project <path>]
```

On a whole task: removes the TOML file and the SQLite entry. On a sub-task
(`--id <task-id>:<subtask-id>`): removes just that `[[tasks]]` entry from the
parent's TOML file and bumps `modified`; the parent task itself is untouched.

Prompts for confirmation unless `--force` (alias: `--yes`) is passed. (Note:
`--force`/`--yes` is required for agent use to avoid hanging on a
confirmation prompt — document this in CLAUDE.md.)

---

### `ruwana index`

Manage the SQLite index.

```
ruwana index --rebuild [--project <path>]
```

- `--rebuild` — drop and recreate the index by scanning all `.ruwana/*.toml` files
- Without `--project`, rebuilds index for all projects under `WIKI_ROOT`

Useful after: manual edits, git clone, submodule init, or index corruption.

---

## Alias Table

| Canonical       | Aliases                                |
| --------------- | --------------------------------------- |
| `ruwana add`    | `ruwana a`, `ruwana +`                   |
| `ruwana done`   | `ruwana do`                              |
| `ruwana undone` | `ruwana undo`                            |
| `ruwana list`   | `ruwana ls`                              |
| `ruwana rm`     | `ruwana remove`, `ruwana delete`, `ruwana -` |
| `ruwana edit`   | `ruwana e`                               |

---

## Error Handling

All errors print to stderr and exit non-zero.

| Situation                          | Exit code | Message                                        |
| ---------------------------------- | --------- | ---------------------------------------------- |
| `--project` not found on disk      | 1         | `project path not found: godel/ai-practice`    |
| ID not found                       | 1         | `no task found with id: xyz`                   |
| Title matches 0 tasks              | 1         | `no task found with title: "..."`              |
| Title matches >1 tasks             | 1         | `ambiguous title; use --project to narrow or use ID` |
| `--id` malformed                   | 1         | `invalid --id format, expected <task-id> or <task-id>:<subtask-id>` |
| Sub-task ID not found in task      | 1         | `no sub-task found with id: gh7f in task abd4hg2f` |
| `--id` used together with `<id-or-title>` | 1  | `--id and a positional id/title are mutually exclusive` |
| Whole-task-only flag used with a compound `--id` | 1 | `--due is not valid when editing a sub-task` |
| Due date cannot be parsed          | 1         | `cannot parse date: "..."`                     |
| Index missing (auto-rebuild)       | 0         | `index not found, rebuilding...` (stderr only) |
| Index out of sync (auto-rebuild)   | 0         | `index stale, rebuilding...` (stderr only)     |

The index is auto-rebuilt silently on mismatch; operations do not fail because
of a stale index.

---

## Must-Have (P0 — v1 ships with these)

- [ ] `add`, `done`, `undone`, `list`, `rm`, `show`, `tasks`, `edit` commands
- [ ] All aliases
- [ ] `--project` flag on all write commands
- [ ] ID + title resolution for `done`, `undone`, `rm`, `show`, `tasks`, `edit`
- [ ] SQLite index with auto-rebuild on missing/stale
- [ ] `ruwana index --rebuild`
- [ ] Human-friendly and explicit date parsing
- [ ] Due dates resolved to end-of-day in the saving machine's local
      timezone and stored with an explicit offset (see Timezone Handling)
- [ ] `--overdue` and `--urgent` filters
- [ ] `--force` flag (alias `--yes`) on `rm` for agent use
- [ ] All date range filters on `list`
- [ ] Tag filtering on `list`
- [ ] `source` field on tasks (`--source` on `add`/`edit`,
      `--remove-source` on `edit`, `--source` filter on `list` — see Task
      File Format and Index Sync)
- [ ] Sub-tasks via `--task` flag (stored as `[[tasks]]` entries), each with
      its own generated 4-character `id`
- [ ] `--id <task-id>[:<subtask-id>]` addressing on `done`, `undone`, `rm`,
      `edit`, including rejection of whole-task-only flags on a sub-task edit
- [ ] `WIKI_ROOT` env var
- [ ] `index.db` gitignored; git hook templates (`post-merge`,
      `post-checkout`) that trigger a scoped `index --rebuild` (see Index
      Sync)
- [ ] Core logic split into a library crate, independent of CLI arg-parsing
      (see "Library-first" in Design Principles) — no TUI code yet, just the
      separation that makes one possible later without a rewrite
- [ ] `--format json` output on `list` and `show` for agent parsing — the
      tool is built agent-first, so this isn't a fast-follow, it's part of
      the baseline agent-facing surface
- [ ] `--sort` flag on `list` (by due, created, modified, title)

## Nice-to-Have (P1 — fast follow)

- [ ] `ruwana add --interactive` for human use (opens $EDITOR)
- [ ] `--limit` flag on `list`
- [ ] `ruwana edit --add-task` for adding a sub-task to an existing task
      (removal is already covered by `ruwana rm --id <task-id>:<subtask-id>`)
- [ ] Completion scripts (bash, zsh, fish)
- [ ] `--remove-due` flag on `edit` to clear a due date
- [ ] Recurring tasks

---

## Future (P2 — planned, after P0 and P1)

- [ ] `ruwana tui` — an interactive terminal UI (likely `ratatui`) for browsing,
      filtering, and editing tasks without leaving the terminal: list
      navigation, toggle done/undone, inline edit of title/due/tags,
      sub-task view and toggling, delete with confirmation.
- [ ] `--priority` field (e.g. low/medium/high), as a genuinely separate
      axis from urgency. `--urgent`/`--overdue` are about *when* something's
      due; priority is about how much it matters regardless of date. Not
      derived from or replacing `--due` in any way.

Not scheduled yet, and explicitly not part of v1 or the P1 fast-follow — this
is here so the requirement isn't lost, not to pull it forward. The one thing
that has to happen *now*, in v1, is the "Library-first" architecture in
Design Principles: the TUI must be able to call the same core crate functions
the CLI calls (add/done/undone/list/edit/rm/index, ID and sub-task
resolution) directly, rather than shelling out to `ruwana` as a subprocess or
re-implementing task logic against the TOML files and SQLite index a second
time.

---

## Someday / Maybe

Not planned, not designed, not ruled out — worth revisiting only if an
actual need for one of these shows up:

- Notifications or reminders
- Sync to external services (Linear, GitHub Issues, etc.)
- GUI interface (web or native)

---

## Out of Scope

- Multi-user collaboration
