# ruwana

Agent-facing task tracker for a personal wiki. Tasks are TOML files under
`$WIKI_ROOT/<project>/.ruwana/` — no database, no daemon; the files are
the single source of truth. Full specification:
`docs/superpowers/specs/2026-07-03-ruwana-design.md`.

## Install

Prebuilt binaries for Linux (glibc + musl), macOS (Apple Silicon +
Intel), and Windows are attached to every
[GitHub Release](https://github.com/e-kulikov/ruwana/releases).

Linux / macOS:

    curl --proto '=https' --tlsv1.2 -LsSf \
      https://github.com/e-kulikov/ruwana/releases/latest/download/ruwana-cli-installer.sh | sh

Windows (PowerShell):

    irm https://github.com/e-kulikov/ruwana/releases/latest/download/ruwana-cli-installer.ps1 | iex

From source:

    cargo install --git https://github.com/e-kulikov/ruwana ruwana-cli

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
- All errors go to stderr and exit 1. Most are single-line; clap parse
  errors include Usage text and ambiguity diagnostics may include candidates.

## Development

    cargo test --workspace
    cargo clippy --workspace -- -D warnings

Workspace layout: `crates/core` (library `ruwana-core`, all task logic),
`crates/cli` (binary `ruwana`, argument parsing and output only).

## Releasing

Releases are fully automated ([spec](docs/superpowers/specs/2026-07-06-release-cicd-design.md)):

1. Merge conventional commits to `main` (`feat:` → minor, `fix:` → patch;
   `feat!:`/`BREAKING CHANGE:` → major — under 0.x, breaking bumps minor).
2. release-plz maintains a rolling **Release PR** with the version bump
   and changelog. **Merging that PR is the release.**
3. The resulting `vX.Y.Z` tag triggers cargo-dist, which builds binaries
   for all five targets and publishes the GitHub Release.

**The agent contract is the public API**: changes to exact stderr
strings, the `--format json` schema, exit-code semantics, the on-disk
TOML format, or command/flag names MUST be committed as `feat!:`/`fix!:`
(or with a `BREAKING CHANGE:` footer).

### One-time repository setup

- Create a fine-grained PAT (this repo only; permissions: Contents RW +
  Pull requests RW) and store it as the `RELEASE_PLZ_TOKEN` repo secret —
  the default `GITHUB_TOKEN` cannot trigger CI on the Release PR.
- Branch protection on `main`: require the `check` job (CI) to pass;
  require branches up to date; no force pushes.
