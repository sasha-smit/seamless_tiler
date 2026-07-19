# Repository Guidelines

## Project Structure & Module Organization

This is a Rust 2024 library crate. `src/lib.rs` defines the public surface and
re-exports types from focused private modules:

- `spatial.rs`: coordinates, extents, and rectangles.
- `grid.rs`: owned row-major rectangular storage.
- `topology.rs`: generic neighborhood traits and square topology.
- `transform.rs`: square rotations and reflections (`D4`).
- `tile.rs`: sockets, tiles, stable IDs, and matching policies.

Unit tests live beside their implementation in `#[cfg(test)]` modules. Crate-level
examples are doctests in `src/lib.rs`. There are currently no runtime assets or
binary target; place future integration tests in `tests/` and binaries in
`src/bin/`.

## Architecture Overview

Keep storage independent from topology: `Grid<T>` owns rectangular values, while
algorithms should use `Topology` and dense `CellId` values for adjacency. Keep the
core renderer-independent and avoid adding image, UI, or solver concerns to these
foundational modules without a demonstrated shared abstraction.

## Build, Test, and Development Commands

- `cargo build`: compile the library in development mode.
- `cargo test --all-targets`: run all unit and target tests.
- `cargo test --doc`: compile and run public documentation examples.
- `cargo fmt --all -- --check`: verify standard Rust formatting.
- `cargo clippy --all-targets -- -D warnings`: enforce lint-clean code.
- `cargo doc --no-deps`: build local API documentation.

Run formatting, tests, and Clippy before submitting changes.

## Coding Style & Naming Conventions

Use `rustfmt` defaults (four-space indentation). Follow Rust naming conventions:
`snake_case` for modules/functions, `UpperCamelCase` for types/traits, and
`SCREAMING_SNAKE_CASE` for constants. Prefer small, domain-focused modules,
checked arithmetic at coordinate boundaries, explicit error types, and rustdoc
for public behavior or invariants. Re-export intended public APIs from `lib.rs`.

## Testing Guidelines

Use Rust's built-in test framework. Name tests as behavioral statements, such as
`axes_wrap_independently`. Cover normal behavior, invalid inputs, empty or
degenerate dimensions, overflow, and algebraic laws. Every public example must
remain a passing doctest. No coverage percentage is mandated, but new behavior
must have focused regression tests.

## Commit & Pull Request Guidelines

Use an imperative, descriptive subject followed by a detailed multi-paragraph
body; wrap every commit-message line at 72 columns. Attribute agent-authored work
with `Co-authored-by: Name (<exact-model>, <effort>) <email>`; ask
for missing runtime identity rather than guessing.
