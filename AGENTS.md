# Repository Guidelines

## Project Structure & Module Organization
`src/` contains the application crate. HTTP entrypoints live in `src/main.rs` and `src/lib.rs`; request routing is in `src/route.rs`; page handlers are under `src/pages/`; HTML helpers are in `src/render/`; Bitcoin Core REST calls are grouped in `src/rpc/`; background sync/indexing tasks live in `src/threads/`. Static assets such as CSS and the favicon are also kept in `src/`. Integration-style tests are in `tests/e2e.rs`; smaller async and utility checks are in `tests/unit.rs`. Docker and Nix packaging files live in `docker/`, `flake.nix`, and `rocksdb-overlay.nix`.

## Build, Test, and Development Commands
Use the pinned Rust toolchain from `rust-toolchain.toml` (`1.85.0`).

- `cargo run --release` starts the explorer locally on `http://localhost:3000/`.
- `cargo build` checks the default build used in CI.
- `cargo test --features download_bitcoind` runs the full test suite, including the regtest-backed end-to-end tests used in GitHub Actions.
- `cargo fmt -- --check` verifies formatting.
- `cargo clippy -- -D warnings` treats all lint warnings as errors.
- `just docker` builds the Nix-based Docker image and loads it into Docker.

For local runtime, point the app at a Bitcoin Core node with `txindex=1` and `rest=1`.

## Coding Style & Naming Conventions
Follow standard Rust formatting: 4-space indentation, `snake_case` for functions/modules, `CamelCase` for types, and small focused modules. Keep new code compatible with `cargo fmt` and warning-free under `clippy`. Match existing file organization by placing route-specific UI in `src/pages/` and reusable HTML formatting in `src/render/`. If you edit `src/css/custom.css`, regenerate `src/css/custom.min.css` before submitting.

## Testing Guidelines
Add tests beside the behavior they exercise: end-to-end HTTP and process behavior in `tests/e2e.rs`, lighter logic checks in `tests/unit.rs`. Prefer descriptive test names like `check_wrong_network`. When a change affects Bitcoin RPC behavior, verify it against regtest with `cargo test --features download_bitcoind`.

## Commit & Pull Request Guidelines
Recent history uses short, imperative commit subjects such as `add ads option` and `link preload css`. Keep commits focused and messages concise. Pull requests should describe the user-visible change, mention any required Bitcoin Core or environment settings, and include screenshots for HTML/CSS changes. Before opening a PR, run `cargo test --features download_bitcoind`, `cargo fmt -- --check`, and `cargo clippy -- -D warnings`.
