//! Renderer-independent foundations for experimenting with two-dimensional tiling.
//!
//! The crate deliberately separates rectangular storage ([`Grid`]) from neighborhood
//! relationships ([`Topology`]). This lets algorithms use dense [`CellId`] values while
//! concrete topologies retain whatever coordinate system suits them.
//!
//! # Example
//!
//! ```
//! use seamless_tiler::{
//!     AxisBoundaries, Boundary, Coord2, Extent2, SocketMap, SquareDirection,
//!     SquareTopology, Tile, TileSet, Topology,
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
//! # Ok::<(), seamless_tiler::TopologyError>(())
//! ```

mod grid;
mod spatial;
mod tile;
mod topology;
mod transform;

pub use grid::{Grid, GridError};
pub use spatial::{Coord2, Extent2, Rect};
pub use tile::{EqualityMatcher, OrientedTileId, SocketMap, SocketMatcher, Tile, TileId, TileSet};
pub use topology::{
    AxisBoundaries, Boundary, CellId, Direction, SquareDirection, SquareTopology, Topology,
    TopologyError,
};
pub use transform::{D4, QuarterTurns};
