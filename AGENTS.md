# Repository Guidelines

## Project Structure & Module Organization

This is a Rust 2024 workspace. The root package is the renderer-independent
`seamless_tiler` library; `src/lib.rs` defines its public surface and re-exports
types from focused private modules:

- `spatial.rs`: coordinates, extents, and rectangles.
- `grid.rs`: owned row-major rectangular storage.
- `topology.rs`: generic neighborhood traits plus square and pointy-top hex
  topologies.
- `transform.rs`: square (`D4`) and hexagonal (`D6`) rotations and reflections.
- `tile.rs`: sockets, tiles, stable IDs, and matching policies.
- `wfc.rs`: weighted compatibility rules, wave domains, observation, and
  constraint propagation over generic topologies.

The `ui/` workspace member is the native `seamless_tiler_ui` binary. Its
`src/main.rs` configures eframe with the wgpu renderer, `src/raster.rs` owns the
square and hex sample surfaces behind the shared `TileSurface` shape contract,
`src/seams.rs` owns shape-independent seamlessness assistance (reversal-aware
edge families, sample link components, orphan detection, and atomic edge copy
planning), `src/model.rs` owns the independent session-only square and hex WFC
configurations and tile adapters, and `src/app.rs` owns egui controls,
mode-aware geometry, hit testing, and canvas painting. `src/app/contact_sheet.rs`
is a test-only offscreen renderer that composites variant images through the
real geometry helpers, so seam coverage and orientation layout stay assertable
without a window.

Unit tests live beside their implementation in `#[cfg(test)]` modules. Crate-level
examples are doctests in `src/lib.rs`. There are currently no runtime assets;
place future library integration tests in `tests/` and keep UI-only code inside
the `ui` package.

## Architecture Overview

Keep storage independent from topology: `Grid<T>` owns rectangular values, while
algorithms should use `Topology` and dense `CellId` values for adjacency. Keep the
core renderer-independent and avoid adding image or UI concerns to foundational
modules. WFC operates on dense `PatternId` domains and compatibility rules; it
must not own tile payloads or rendering data. The UI maps patterns to oriented
tiles and applies demo-specific policies such as closed bounded edges. The UI
may depend on the library, but the library must not depend on eframe, egui, wgpu,
or UI-specific payloads. Keep the native UI session-only unless persistence or a
project format is explicitly designed.

Each tile's raster is the single authority for both its appearance and its
matching. `SquareRaster` and `HexRaster` implement the shared `TileSurface`
contract, sockets are always `EdgeStrip<Rgba>` re-extracted from the raster's
border after an edit, and boolean sockets no longer exist. The procedural
generators in `raster.rs` only seed demo tiles; never reintroduce them as a
parallel source of truth. Deduplicate orientations by the complete transformed
raster, and derive variants per tile so a pencil stroke only re-derives the
tiles it changed.

Hex samples are *points* on an axial lattice, not areas: the outermost ring lies
on the cell boundary and each corner is one sample shared by two sides. A hex
image therefore spans one texel more than the cell it depicts and must be drawn
into bounds inflated by `IMAGE_SIZE / SAMPLE_SPAN` (`hex_image_rect`), or
neighboring cells leave transparent notches along the slanted seams. The
resulting one-texel overlap is invisible because facing strips are byte-identical.
Use nearest filtering only, never resample a raster to rotate it, and keep
export, editor hit testing, and overlays all going through
`HexRaster::sample_at_texel` so they cannot disagree.

`HexTopology` uses dense odd-row offset `Coord2` coordinates: odd rows are
visually shifted half a cell to the right. Keep wrapped neighbor relationships
reciprocal for odd and even extents, including degenerate dimensions. Tile
orientation should go through `DirectionTransform`; use `D4` with
`SquareDirection` and `D6` with `HexDirection`. The UI should retain independent
square and hex dimensions, boundaries, pins, enabled variants, seeds, and solver
progress when switching modes.

## Build, Test, and Development Commands

- `cargo build --workspace`: compile the library and UI in development mode.
- `cargo run -p seamless_tiler_ui`: launch the native WFC visualizer with wgpu.
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
Keep seeded solver behavior deterministic, validate public rule inputs, and
handle wrapped self-neighbors as unary constraints during propagation. Keep
direction values dense and clockwise, preserve opposite-direction involutions,
and test transform inverse and composition laws exhaustively.

## Testing Guidelines

Use Rust's built-in test framework. Name tests as behavioral statements, such as
`axes_wrap_independently`. Cover normal behavior, invalid inputs, empty or
degenerate dimensions, overflow, and algebraic laws. Every public example must
remain a passing doctest. No coverage percentage is mandated, but new behavior
must have focused regression tests. Keep UI state transitions in testable model
methods rather than egui callbacks where practical. Manually smoke-test native
window creation, switching between square and hex modes without losing either
session, allowed-orientation toggles, inspect/pin/unpin tools, polygon hit
testing, resizing, bounded and independently wrapped axes, seed reset and retry,
and step/run/pause/finish playback after altering interactive behavior. Also
smoke-test pencil painting and erasing in both modes, per-mode brush size,
adding, selecting, renaming, recoloring and deleting tiles, orphan diagnostics,
edge-assistant copy including its reversed, no-op, invalid and conflicting
cases, and solved-grid seams at both ends of the cell-size slider.

Check rendering changes offscreen rather than by eye alone: set
`TILER_CONTACT_SHEET` to a directory and run the UI tests to write PPM sheets of
every tile orientation and of solved bounded and wrapped grids. That test also
asserts no drawn grid encloses an uncovered background pixel, which is what a
transparent seam looks like to a viewer.

## Commit & Pull Request Guidelines

Commit directly to `master` unless asked otherwise; do not create a feature
branch by default. Use an imperative, descriptive subject followed by a detailed
multi-paragraph body; wrap every commit-message line at 72 columns. Attribute
agent-authored work with `Co-authored-by: Name (<model>, <effort>) <email>`,
writing the model with a dot rather than the API's hyphen — for example
`Co-authored-by: Claude (claude-opus-4.8, high) <noreply@anthropic.com>`; ask
for missing runtime identity rather than guessing.
