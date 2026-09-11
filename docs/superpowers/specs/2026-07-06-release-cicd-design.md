# `ruwana` — Release & CI/CD Specification

> Automated release pipeline for the ruwana Cargo workspace on GitHub Actions.
> Status: **approved spec**.
> Companion to `2026-07-03-ruwana-design.md` (the v1 CLI spec).

---

## Problem Statement

ruwana lives at `github.com/e-kulikov/ruwana` (Cargo workspace: `ruwana-core`
lib + `ruwana-cli` bin, single maintainer, trunk-based, conventional
commits, largely agent-driven development). It needs releases that are
automated end to end — version bumps, changelog, tags, binaries — with
exactly one deliberate human action per release and no hand-maintained
version numbers or release notes anywhere.

---

## Goals and Non-Goals

**Goals:**

- One human action per release (merging a release PR); everything else —
  semver computation, changelog, `Cargo.toml` bumps, tag, binaries,
  GitHub Release — is automated.
- CI gate (test + clippy + fmt) on every PR and push to `main`, required
  for merge.
- Version numbers derived from conventional-commit history, never edited
  by hand.
- Prebuilt binaries for every platform the author uses — Windows 11
  x86_64, macOS (Apple Silicon and Intel), and Linux/WSL x86_64 —
  downloadable from GitHub Releases; installable via
  `cargo install --git` as the fallback.
- The released binary's `--version` provably matches the released tag.

**Non-Goals (this iteration):**

- Publishing to crates.io (both crates stay `publish = false`; see
  Decision Log — revisit when the tool has external users).
- Code signing and macOS notarization (unsigned binaries are fine for
  personal use; Gatekeeper/SmartScreen warnings are acceptable —
  revisit if the tool gains outside users).
- ARM Linux and ARM Windows targets (nothing the author runs; config
  lines away when needed).
- Dependency-audit automation (`cargo-deny`), artifact attestations —
  deferred to P1, not because they're hard but to keep the first
  iteration minimal.
- Docker images, package managers (homebrew/AUR/apt), release
  announcements.

---

## Strategy Overview

**Trunk-based development + Release-PR pattern + tag-driven binary builds.**

```
feature commits → main ──► ci.yml (gate: test/clippy/fmt)
        │
        └─► release-plz.yml ──► maintains rolling "Release PR"
                                  (version bump + CHANGELOG, recomputed
                                   on every push to main)
                    │
        human merges the Release PR   ◄── THE one manual step
                    │
        release-plz tags vX.Y.Z ──► release.yml (cargo-dist)
                                        ├─ builds target binaries
                                        ├─ archives + checksums
                                        └─ creates GitHub Release
```

Releases batch naturally: the Release PR accumulates unreleased changes
and its diff/changelog is the release review. Merging on every feature is
possible; releasing weekly-ish is the expected cadence. Agent-driven
commit bursts (a 17-commit overnight run) produce exactly one pending
release PR, not 17 releases.

---

## Components

### 1. `.github/workflows/ci.yml` — the merge gate

Triggers: `pull_request`, `push` to `main`.

Jobs (single job `check`, Ubuntu):

1. `actions/checkout`
2. `dtolnay/rust-toolchain@stable` with `components: clippy, rustfmt`
3. `Swatinem/rust-cache@v2`
4. `cargo fmt --check`
5. `cargo clippy --workspace --all-targets -- -D warnings`
6. `cargo test --workspace`

Details:

- `concurrency: group: ci-${{ github.ref }}, cancel-in-progress: true` —
  superseded pushes don't waste runners.
- `permissions: contents: read` (least privilege).
- Steps ordered cheapest-first (fmt fails in seconds; tests last).
- This mirrors the local gate the repo already uses (Task 14's
  `test && clippy && fmt --check`), so CI can't disagree with local
  verification.

### 2. `.github/workflows/release-plz.yml` — versioning & release PR

Trigger: `push` to `main`. Two jobs, both via
`release-plz/action@v0.5` (pin by major):

- **`release-plz-pr`**: creates/updates the rolling Release PR — computes
  the next version from conventional commits since the last tag
  (`fix:` → patch, `feat:` → minor, `feat!:`/`BREAKING CHANGE:` → major;
  under 0.x, breaking changes bump minor per Cargo semver), rewrites
  `[workspace.package] version`, updates `Cargo.lock`, and generates
  `CHANGELOG.md` (git-cliff engine, keep-a-changelog format).
- **`release-plz-release`**: after the Release PR merges, creates the git
  tag `vX.Y.Z` — and nothing else. It deliberately does NOT create a
  GitHub Release (`git_release_enable = false`): release-plz owns tags
  and the changelog, cargo-dist owns the GitHub Release object, so the
  two tools never fight over it.

Config `release-plz.toml` (repo root):

```toml
[workspace]
publish = false                       # no crates.io
git_only = true                       # repository releases, never crates.io
release_always = false
features_always_increment_minor = true # `feat:` bumps minor under 0.x
git_release_enable = false            # cargo-dist owns the GitHub Release
git_tag_enable = false
git_tag_name = "v{{ version }}"        # one shared workspace tag
semver_check = false                  # no public library API commitment yet
changelog_update = false
pr_labels = ["release"]

[[package]]
name = "ruwana-cli"
git_tag_enable = true
git_tag_name = "v{{ version }}"
changelog_update = true
changelog_path = "./CHANGELOG.md"
changelog_include = ["ruwana-core"]
```

**Auth gotcha (load-bearing):** PRs created with the default
`GITHUB_TOKEN` do **not** trigger other workflows — the Release PR would
show no CI checks and be unmergeable under branch protection. The
workflow therefore uses a fine-grained PAT stored as the
`RELEASE_PLZ_TOKEN` repo secret (scopes: contents RW + pull-requests RW
on this repo only). Creating that secret is a documented manual setup
step.

### 3. `.github/workflows/release.yml` — binaries (cargo-dist)

Generated and owned by **cargo-dist** (the maintained
`astral-sh/cargo-dist` fork — the original axodotdev project was
discontinued in 2025; the fork is the successor, used across the Rust
ecosystem). Never hand-edited: config lives in `dist-workspace.toml`,
and `dist init` regenerates the workflow after config or version
changes.

Trigger: push of tag `v**`.

Config (`dist-workspace.toml`):

- Targets — one per platform the author actually runs, plus the static
  Linux fallback:

  | Target | Covers |
  | ------ | ------ |
  | `x86_64-unknown-linux-gnu` | the author's WSL2 (primary) |
  | `x86_64-unknown-linux-musl` | static fallback: containers, minimal hosts |
  | `x86_64-pc-windows-msvc` | the author's Windows 11 host (NT 10.0.26200, AMD64) |
  | `aarch64-apple-darwin` | macOS on Apple Silicon |
  | `x86_64-apple-darwin` | macOS on Intel |

  cargo-dist builds each on its native GitHub runner (ubuntu / windows /
  macos images) — no cross-compilation fragility.
- `installers = ["shell", "powershell"]` — `curl | sh` for Linux/macOS,
  `irm | iex` for Windows, both published with each release.
- `ci = "github"`, `install-path = "CARGO_HOME"`.
- Artifacts: per-target archives (tarballs; `.zip` for Windows, dist's
  default) + `sha256` checksums + both installer scripts, attached to
  the GitHub Release for the tag; release body = the changelog section
  for that version.

Platform caveat: unit/e2e tests keep running on Ubuntu only (ci.yml).
The code is platform-portable by construction (no unix-only APIs;
`discover.rs` already normalizes `\` → `/`), and the release builds
themselves will surface compile-level platform breakage. A full 3-OS
test matrix is deliberately P1, not P0 — it triples CI minutes for a
risk the release build already half-covers.

Only `ruwana-cli` (binary `ruwana`) is distributed; `ruwana-core` ships
inside it.

### 4. Version-consistency test (in-repo, part of this iteration)

One new e2e test in `crates/cli/tests/e2e.rs`:

```rust
#[test]
fn version_flag_reports_crate_version() {
    let mut cmd = Command::cargo_bin("ruwana").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
```

This closes the loop: release-plz bumps the manifest, the binary is
built from that manifest, and CI proves `--version` reflects it — a
release can never ship a stale version string. (clap's
`#[command(version)]` already wires `CARGO_PKG_VERSION` in; the test
pins the contract.)

### 5. Repository settings (manual, documented in README)

- Branch protection / ruleset on `main`: require the `check` job (ci.yml)
  to pass; require branches up to date; no force pushes. (GitHub
  settings aren't in-repo; this is a one-time documented step.)
- Repo secret `RELEASE_PLZ_TOKEN` (fine-grained PAT, see §2).
- `rust-version = "1.85"` added to `[workspace.package]` (edition 2024
  floor) so toolchain expectations are explicit.

---

## Versioning Policy

- Start `0.1.0`; stay 0.x until the CLI surface feels frozen. Under
  Cargo semver, 0.x breaking changes bump the **minor**; release-plz
  follows this automatically.
- **The agent contract is the public API.** Changes to any of: exact
  stderr error strings, the `--format json` schema, exit-code semantics,
  the on-disk TOML task format, or command/flag names MUST be committed
  as `feat!:` / `fix!:` (or with a `BREAKING CHANGE:` footer) so the
  version number honestly signals contract changes to agent consumers.
  This rule is documented in the README's development section.
- Both workspace crates share one version via `version.workspace = true`
  (already the case); release-plz bumps them together.
- Commit-type conventions (already in use) are the *only* input to
  version computation — no manual version edits, ever. A commit that
  should not appear in the changelog uses `chore:`/`docs:`/`style:`.

---

## Release Flow (end to end)

1. Work merges to `main` as conventional commits (direct or via PR; CI
   gates either way).
2. release-plz updates the rolling Release PR within a minute of each
   push: bumped version, regenerated changelog. It sits until wanted.
3. **Release decision = merging that PR.** Review is reading its diff:
   the changelog IS the release notes.
4. release-plz tags `vX.Y.Z` on the merge commit.
5. cargo-dist's workflow builds both Linux targets, packages archives +
   checksums + installer, and publishes the GitHub Release with the
   changelog body.
6. Consumers: download from Releases, `curl | sh` the installer, or
   `cargo install --git https://github.com/e-kulikov/ruwana ruwana-cli`.

Rollback: a bad release is handled forward (`fix:` commit → merge → new
release). Deleting a published Release/tag is manual and exceptional —
documented as "don't, unless the artifact is actively harmful."

---

## Failure Modes & Maintenance

- **Release PR conflicts** (e.g. after force-ish history edits):
  release-plz regenerates the PR on the next push; worst case close it
  and let it recreate.
- **cargo-dist workflow drift**: the workflow file is generated; after
  bumping the pinned dist version in `dist-workspace.toml`, run
  `dist init` and commit the regenerated YAML. Never hand-patch it.
- **Action supply chain**: all third-party actions pinned by major
  version tag (`@v2`, `@v0.5`); the release workflow is generated with
  dist's own pins. (Pin-by-SHA is deliberately skipped — solo project,
  the maintenance cost outweighs the threat model. Revisit if the tool
  gains users.)
- **First-release validation**: after landing the pipeline, cut `v0.1.0`
  by merging the first Release PR and verify the whole chain once —
  tag → binaries → checksums → installer → `ruwana --version` says
  0.1.0. This is the pipeline's integration test; nothing simulates it
  faithfully.

---

## Roadmap

### P0 (this iteration ships)

- [ ] `ci.yml` (fmt → clippy → test, rust-cache, concurrency cancel,
      least-privilege permissions)
- [ ] `release-plz.yml` + `release-plz.toml` (release PR + tags;
      `RELEASE_PLZ_TOKEN` documented)
- [ ] cargo-dist init: `dist-workspace.toml` (5 targets: Linux gnu+musl,
      Windows x86_64 MSVC, macOS arm64+x86_64; shell + powershell
      installers) + generated `release.yml`
- [ ] `rust-version = "1.85"` in `[workspace.package]`
- [ ] Version-consistency e2e test
- [ ] README additions: install instructions (release artifacts,
      installer, cargo install --git), release-process section (how to
      cut a release, the `feat!:` agent-contract rule), manual setup
      steps (branch protection, PAT secret)
- [ ] First release `v0.1.0` cut and verified end to end

### P1 (fast follow)

- [ ] `cargo-deny` (advisories + licenses) as a scheduled weekly job +
      on `Cargo.lock` changes
- [ ] GitHub artifact attestations (build provenance) on release
      artifacts
- [ ] 3-OS CI test matrix (ubuntu/windows/macos) for the `check` job —
      release builds compile on all five targets from day one, but
      tests only run on Linux until this lands
- [ ] `aarch64-unknown-linux-gnu` target (ARM servers/containers)

### Out of scope until there's demand

- crates.io publishing (flip `publish` flags + release-plz handles
  ordering when wanted); macOS/Windows targets; signing; package
  managers; MSRV CI matrix.

---

## Decision Log

| # | Decision | Alternatives considered |
| - | -------- | ----------------------- |
| 1 | Release-PR pattern (release-plz) | Release-on-every-merge (too noisy for agent-driven repos); manual tagging (reintroduces human error) |
| 2 | release-plz over semantic-release/release-please | Rust/workspace-native: understands Cargo.toml, Cargo.lock, crate dependency order; JS tools need adapters |
| 3 | cargo-dist (astral-sh fork) for binaries | Hand-rolled matrix + `taiki-e/upload-rust-binary-action` (fine fallback, chosen against because dist is declarative, generates checksums/installers/release bodies, and regeneration prevents YAML rot); the fork choice is forced — upstream axodotdev is discontinued |
| 4 | No crates.io in this iteration | Publishing commits to a public name/semver contract a personal tool doesn't need; `cargo install --git` + binaries cover all current consumers |
| 5 | Five targets: Linux gnu+musl, Windows x86_64 MSVC, macOS arm64 + x86_64 | Covers every machine the author uses (WSL2 primary, Windows 11 AMD64 host, both macOS architectures) plus the static-musl fallback; native runners per OS avoid cross-compilation fragility; ARM Linux/Windows deferred (nothing runs there) |
| 5a | Tests stay Ubuntu-only in P0; 3-OS test matrix is P1 | Release builds already compile all five targets (catching compile-level platform breakage); a full matrix triples CI minutes — added once, cheaply, in P1 |
| 6 | Split ownership: release-plz owns tags/changelog, cargo-dist owns the GitHub Release | Letting both create Releases double-posts; letting dist own everything loses the release-PR gate |
| 7 | PAT (`RELEASE_PLZ_TOKEN`) over default token | Default-token PRs don't trigger CI → unmergeable release PRs under branch protection; a GitHub App is over-engineering for solo use |
| 8 | Actions pinned by major tag, not SHA | Solo project threat model; SHA-pinning without update automation (dependabot) rots |
| 9 | 0.x with agent-contract-as-API rule | Jumping to 1.0 promises stability the CLI hasn't earned; without the `feat!:` rule, semver would lie to agent consumers about stderr/JSON contract changes |
