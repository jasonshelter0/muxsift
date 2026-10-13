# Repository Guidelines

## Project Structure & Module Organization

MuxSift is a Rust workspace. `crates/muxsift-core/` contains Bilibili access, downloading, ASR, AI processing, and resume checkpoints. `crates/muxsift-cli/` owns commands and configuration; `crates/muxsift-tui/` owns the terminal interface. Unit tests live beside the code in `#[cfg(test)]` modules. `scripts/` contains the Buzz bridge and Linux packaging scripts; `distribution/` holds launcher and configuration templates. `third_party/` holds packaging notices and locally fetched tools. User documentation is in `README.md` and the two `USER-GUIDE` files.

## Build, Test, and Development Commands

- `cargo build -p muxsift-cli --locked`: build the CLI/TUI executable at `target/debug/muxsift`.
- `cargo test --workspace --locked`: run workspace unit tests.
- `cargo fmt --check`: check Rust formatting.
- `cargo clippy --workspace --all-targets -- -D warnings`: reject lint warnings.
- `./scripts/fetch-linux-tools.sh`: fetch verified Linux aria2 and FFmpeg binaries when needed.
- `./scripts/package-linux.sh`: build a Linux x86_64 archive and SHA-256 file in `dist/`.

Run `target/debug/muxsift --help` for CLI options or `target/debug/muxsift tui` for the terminal UI.

## Coding Style & Naming Conventions

Use Rust 2024 with the workspace minimum compiler version, Rust 1.85. Follow `rustfmt` defaults (four-space indentation). Use `snake_case` for modules, functions, and test names; `PascalCase` for types; and descriptive CLI flag names. Keep reusable processing logic in `muxsift-core` rather than duplicating it between CLI and TUI.

## Testing Guidelines

Add focused `#[test]` or `#[tokio::test]` cases next to changed logic, especially for input parsing, output paths, stage resume, and batch selection. Name tests for the behavior they verify. Run the workspace tests and formatting/lint checks before proposing a release. No coverage percentage is currently enforced.

## Commit & Pull Request Guidelines

This repository has no local Git history yet, so no established commit convention exists. Use short, imperative subjects such as `Add local folder batch selection`. Pull requests should explain the behavior change, list verification commands, link relevant issues, and include TUI screenshots when the interface changes. Keep generated `target/`, `dist/`, and fetched runtime binaries out of source commits; attach archives and checksums to releases.
