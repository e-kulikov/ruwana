# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/e-kulikov/ruwana/releases/tag/v0.1.0) - 2026-09-30

### Added

- *(cli)* JSON output for list and show
- *(cli)* clap commands, text output, and core wiring

### Fixed

- preserve output and storage invariants
- honor edit validation contracts
- address follow-up review findings
- harden task storage and release flow
- *(cli)* parse errors exit 1; show/tasks no longer accept --id

### Other

- skip release pipeline on PRs, single workspace tag, no crates.io publish
- add cargo-dist release builds (5 targets, shell+powershell installers)
- pin rust-version 1.85; test --version matches crate version
- post-review cleanup — drop unused anyhow, guard DST rule, fix stale docs
- cargo fmt
- *(cli)* end-to-end spec coverage; add README
- Scaffold Cargo workspace with core and cli crates
