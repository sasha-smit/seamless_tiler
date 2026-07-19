# Repository Guidelines

## Project Structure & Module Organization

This is a Rust 2024 workspace. The root package is the renderer-independent
`seamless_tiler` library; `src/lib.rs` defines its public surface and re-exports
types from focused private modules:

- `spatial.rs`: coordinates, extents, and rectangles.
- `grid.rs`: owned row-major rectangular storage.
- `topology.rs`: generic neighborhood traits and square topology.
- `transform.rs`: square rotations and reflections (`D4`).
- `tile.rs`: sockets, tiles, stable IDs, and matching policies.

The `ui/` workspace member is the native `seamless_tiler_ui` binary. Its
`src/main.rs` configures eframe with the wgpu renderer, while `src/app.rs` owns
the editor model, egui controls, canvas painting, and focused model tests.

Unit tests live beside their implementation in `#[cfg(test)]` modules. Crate-level
examples are doctests in `src/lib.rs`. There are currently no runtime assets;
place future library integration tests in `tests/` and keep UI-only code inside
the `ui` package.

## Architecture Overview

Keep storage independent from topology: `Grid<T>` owns rectangular values, while
algorithms should use `Topology` and dense `CellId` values for adjacency. Keep the
core renderer-independent and avoid adding image, UI, or solver concerns to these
foundational modules without a demonstrated shared abstraction. The UI may
depend on the library, but the library must not depend on eframe, egui, wgpu, or
UI-specific payloads. Keep the native UI session-only unless persistence or a
project format is explicitly designed.

## Build, Test, and Development Commands

- `cargo build --workspace`: compile the library and UI in development mode.
- `cargo run -p seamless_tiler_ui`: launch the native grid editor with wgpu.
- `cargo test --workspace --all-targets`: run all library and UI tests.
- `cargo test --workspace --doc`: compile and run public documentation examples.
- `cargo fmt --all -- --check`: verify standard Rust formatting.
- `cargo clippy --workspace --all-targets -- -D warnings`: enforce lint-clean
  code.
- `cargo doc --workspace --no-deps`: build local API documentation.

Run formatting, tests, and Clippy before submitting changes. Manage dependencies
with Cargo commands such as `cargo add` and `cargo remove`, preferring the latest
compatible release unless the repository has a documented compatibility reason
to pin an older version. Keep eframe configured without default features and
explicitly enable `default_fonts`, `wgpu`, `x11`, and `wayland`; do not add a glow
fallback without a demonstrated need.

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
must have focused regression tests. Keep UI state transitions in testable model
methods where practical; manually smoke-test native window creation, painting,
erasing, resizing, orientation controls, and bounded/wrapped topology changes
after altering interactive behavior.

## Commit & Pull Request Guidelines

Use an imperative, descriptive subject followed by a detailed multi-paragraph
body; wrap every commit-message line at 72 columns. Attribute agent-authored work
with `Co-authored-by: Name (<exact-model>, <effort>) <email>`; ask
for missing runtime identity rather than guessing.
