# ruwana

Agent-facing task tracker for a personal wiki. Tasks are TOML files under
`$WIKI_ROOT/<project>/.ruwana/` — no database, no daemon; the files are
the single source of truth. Full specification:
`docs/superpowers/specs/2026-07-03-ruwana-design.md`.

## Install

    cargo install --path crates/cli

## Quickstart

    export WIKI_ROOT=~/wiki          # default: ~/wiki
    ruwana add --project godel/ai-practice "Review Q3 OKRs" \
      --due "next friday" --tag okr --source meeting-2024-01-15
    ruwana list                      # open tasks across all projects
    ruwana done <id>                 # or: ruwana done "Review Q3 OKRs"
    ruwana show <id> --format json

## Notes for agents

- `rm` prompts a human; **always pass `--force`** (alias `--yes`) — in a
  non-interactive session `rm` without it exits 1 instead of hanging.
- `--format json` on `list` and `show` is the stable machine interface;
  everything informational is on stderr, stdout is data only.
- All errors: single-line stderr message, exit code 1.

## Development

    cargo test --workspace
    cargo clippy --workspace -- -D warnings

Workspace layout: `crates/core` (library `ruwana-core`, all task logic),
`crates/cli` (binary `ruwana`, argument parsing and output only).
