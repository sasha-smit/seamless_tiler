//! Renderer-independent foundations and Wave Function Collapse for two-dimensional tiling.
//!
//! The crate deliberately separates rectangular storage ([`Grid`]) from neighborhood
//! relationships ([`Topology`]). This lets algorithms use dense [`CellId`] values while
//! concrete topologies retain whatever coordinate system suits them. [`Wfc`] operates on
//! topology cells and dense pattern IDs without owning application payloads or rendering data.
//!
//! # Example
//!
//! ```
//! use seamless_tiler::{
//!     AxisBoundaries, Boundary, Coord2, Extent2, HexDirection, HexTopology, PatternId,
//!     SocketMap, SquareDirection, SquareTopology, Tile, TileSet, Topology, Wfc, WfcRules,
//!     WfcStatus,
//! };
//!
//! let mut tiles = TileSet::new();
//! let path = tiles.insert(Tile::new(
//!     "path",
//!     SocketMap::from_fn(|direction: SquareDirection| {
//!         matches!(direction, SquareDirection::East | SquareDirection::West)
//!     }),
//! ));
//! assert_eq!(tiles.get(path).unwrap().sockets[SquareDirection::East], true);
//!
//! let extent = Extent2::new(3, 2);
//! let bounded = SquareTopology::bounded(extent)?;
//! let cylinder = SquareTopology::new(
//!     extent,
//!     AxisBoundaries::new(Boundary::Wrap, Boundary::Bounded),
//! )?;
//! let left = bounded.cell_at(Coord2::new(0, 0)).unwrap();
//!
//! assert_eq!(bounded.neighbor(left, SquareDirection::West), None);
//! assert_eq!(
//!     cylinder.coordinate(cylinder.neighbor(left, SquareDirection::West).unwrap()),
//!     Some(Coord2::new(2, 0)),
//! );
//! let hexes = HexTopology::bounded(extent)?;
//! let hex_origin = hexes.cell_at(Coord2::ZERO).unwrap();
//! assert!(hexes
//!     .neighbor(hex_origin, HexDirection::SouthEast)
//!     .is_some());
//!
//! let rules = WfcRules::new([1.0, 1.0], |_direction, source, neighbor| {
//!     source == neighbor
//! }).unwrap();
//! let wave = Wfc::with_constraints(bounded, rules, 7, |cell, pattern| {
//!     cell != left || pattern == PatternId::new(0)
//! });
//! assert_eq!(wave.status(), WfcStatus::Solved);
//! # Ok::<(), seamless_tiler::TopologyError>(())
//! ```

mod grid;
mod spatial;
mod tile;
mod topology;
mod transform;
mod wfc;

pub use grid::{Grid, GridError};
pub use spatial::{Coord2, Extent2, Rect};
pub use tile::{EqualityMatcher, OrientedTileId, SocketMap, SocketMatcher, Tile, TileId, TileSet};
pub use topology::{
    AxisBoundaries, Boundary, CellId, Direction, HexDirection, HexTopology, SquareDirection,
    SquareTopology, Topology, TopologyError,
};
pub use transform::{D4, D6, DirectionTransform, QuarterTurns, SixthTurns};
pub use wfc::{PatternId, Wfc, WfcError, WfcRules, WfcStatus, WfcStep};
