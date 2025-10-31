# Repository Guidelines

## Project Structure & Module Organization
The workspace roots two crates: `pbf-craft/` (core library) and `pbf-craft-cli/` (command-line client). Library modules live under `pbf-craft/src/`, with codecs, readers, writers, and models split into dedicated folders for focused ownership. CLI subcommands are grouped in `pbf-craft-cli/src/commands/`, while Postgres access layers sit in `pbf-craft-cli/src/db/`. Sample PBF fixtures are stored in `pbf-craft/resources/`, and build artifacts land in `target/`; keep generated files out of version control unless explicitly needed.

## Build, Test, and Development Commands
- `cargo build --workspace` – compile both crates and ensure shared interfaces stay compatible.
- `cargo run -p pbf-craft-cli -- --help` – confirm the CLI starts and prints the supported subcommands.
- `cargo test --workspace` – execute all unit and integration tests; run before every PR.
- `cargo fmt` and `cargo clippy --workspace --all-targets` – enforce formatting and linting prior to committing.

## Coding Style & Naming Conventions
The codebase targets Rust 2021 with the default 4-space indentation. Use `cargo fmt` to standardize layout and respect rustfmt defaults for imports and expression wrapping. Follow Rust idioms: modules and functions in `snake_case`, types and traits in `CamelCase`, constants in `SCREAMING_SNAKE_CASE`. Prefer explicit `Result<T, anyhow::Error>` returns and `?` propagation. Keep protobuf schema changes isolated in `pbf-craft/build.rs` inputs so generated code stays reproducible.

## Testing Guidelines
Unit tests should live beside the code under `#[cfg(test)] mod tests` blocks; mimic the patterns in `pbf-craft/src/codecs/block_builder.rs` when adding coverage. Use `cargo test -p pbf-craft --lib` to focus on the library and `cargo test -p pbf-craft-cli` when exercising CLI helpers. When a feature depends on sample data, reference files under `pbf-craft/resources/` and document the fixture in the test name. Aim to cover new public APIs with deterministic assertions and add regression tests for every bug fix.

## Commit & Pull Request Guidelines
Commit history favors concise Conventional-style prefixes (for example, `refactor: tune indexed reader cache` or `cicd: remove clippy step`). Write present-tense subjects that explain the why, not the how. Pull requests should include: a short summary, linked issues or tickets, the commands you ran (`cargo test`, `cargo clippy`, etc.), and CLI output or screenshots when a user-facing command changes. Request review from owners of affected modules and wait for CI to pass before merging.

## Data & Configuration Tips
The CLI expects a Postgres instance populated with OSM extracts; configure host, port, database, user, and password through the `DatabaseReader` constructor or equivalent wiring. Store sensitive credentials outside the repo (for example, local `.env` files ignored by Git). For quick manual checks, run `cargo run -p pbf-craft-cli -- export --help` to see required connection flags. Keep large PBF downloads in `resources/` or `.cache/` paths that are excluded from commits.
