# Repository Guidelines

## Project Structure & Module Organization

FeedWake is a Rust 2021 CLI crate. Core code lives in `src/`: `main.rs` is the executable entry point, `lib.rs` exposes shared modules, and focused modules cover configuration, feed parsing, filtering, SQLite state, delivery, and app orchestration. Integration tests live in `tests/`, for example `tests/filtering.rs`, `tests/feed_parsing.rs`, and `tests/delivery_http.rs`. Example runtime configuration is in `config/feedwake.example.toml`; do not commit local secrets or machine-specific config.

## Build, Test, and Development Commands

- `cargo build`: compile the debug binary during development.
- `cargo build --release`: produce the optimized CLI binary used for deployment.
- `cargo test`: run the full unit and integration test suite.
- `cargo clippy --all-targets -- -D warnings`: run lint checks and fail on warnings.
- `cargo fmt`: apply standard Rust formatting before review.
- `target/release/feedwake scan --dry-run`: run a scan without writing state after building a release binary.

For local runs, copy `config/feedwake.example.toml` to a private path such as `~/.config/feedwake/config.toml` and set `OPENCLAW_HOOK_TOKEN` in the environment.

## Coding Style & Naming Conventions

Use standard `rustfmt` formatting. Keep modules small and behavior-oriented; prefer adding logic to the module that owns the concern. Use `snake_case` for functions, variables, modules, and test names; use `PascalCase` for types and traits. Prefer explicit errors with `anyhow::Context` where call-site detail helps operations. Avoid silent fallbacks for invalid configuration or delivery failures.

## Testing Guidelines

Add or update tests for behavior changes. Place cross-module behavior in `tests/` and name files after the feature under test. Test names should describe the expected behavior, such as `dedupes_seen_feed_items`. Favor public APIs and realistic fixtures over test-only hooks. Run `cargo test` and Clippy before opening a pull request.

## Commit & Pull Request Guidelines

Existing commits use short imperative summaries, for example `Configure Dependabot lockfile-only updates` and `Create dependabot.yml`. Keep the first line concise and specific. This is a hobby project with no Jira tracking, so do not prompt for or require a Jira ID when creating commits or pull requests. Pull requests should include the purpose, key behavior changes, test results, and any configuration impact. Link related issues only when they exist.

## Security & Configuration Tips

Keep hook tokens in environment variables, not TOML files or commits. Treat feed URLs, hook endpoints, and SQLite state paths as operational configuration. When changing HTTP fetching or delivery, preserve bounded timeouts, response-size limits, bearer authentication, and non-2xx error handling.
