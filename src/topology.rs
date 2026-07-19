use std::error::Error;
use std::fmt;
use std::hash::Hash;

use crate::{Coord2, Extent2};

/// A stable, dense identifier for a cell within one topology.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellId(usize);

impl CellId {
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    pub const fn index(self) -> usize {
        self.0
    }
}

/// A finite direction set usable for dense direction-indexed storage.
///
/// Implementations must list every value exactly once in `ALL`, and `index` must return
/// that value's position in `ALL`. `opposite` must be an involution.
pub trait Direction: Copy + Eq + Hash + 'static {
    const ALL: &'static [Self];

    fn index(self) -> usize;
    fn opposite(self) -> Self;
}

/// Defines cells and directional neighbor relationships independently of stored values.
pub trait Topology {
    type Coord: Copy + Eq + Hash;
    type Direction: Direction;

    fn cell_count(&self) -> usize;
    fn cell_at(&self, coord: Self::Coord) -> Option<CellId>;
    fn coordinate(&self, cell: CellId) -> Option<Self::Coord>;
    fn neighbor(&self, cell: CellId, direction: Self::Direction) -> Option<CellId>;

    fn contains(&self, cell: CellId) -> bool {
        cell.index() < self.cell_count()
    }
}

/// The four edge-sharing directions of a square cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum SquareDirection {
    North,
    East,
    South,
    West,
}

impl Direction for SquareDirection {
    const ALL: &'static [Self] = &[Self::North, Self::East, Self::South, Self::West];

    fn index(self) -> usize {
        self as usize
    }

    fn opposite(self) -> Self {
        match self {
            Self::North => Self::South,
            Self::East => Self::West,
            Self::South => Self::North,
            Self::West => Self::East,
        }
    }
}

impl SquareDirection {
    pub const fn offset(self) -> Coord2 {
        match self {
            Self::North => Coord2::new(0, -1),
            Self::East => Coord2::new(1, 0),
            Self::South => Coord2::new(0, 1),
            Self::West => Coord2::new(-1, 0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Boundary {
    #[default]
    Bounded,
    Wrap,
}

/// Boundary behavior selected independently for both axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AxisBoundaries {
    pub horizontal: Boundary,
    pub vertical: Boundary,
}

impl AxisBoundaries {
    pub const BOUNDED: Self = Self::new(Boundary::Bounded, Boundary::Bounded);
    pub const TOROIDAL: Self = Self::new(Boundary::Wrap, Boundary::Wrap);

    pub const fn new(horizontal: Boundary, vertical: Boundary) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TopologyError {
    CellCountOverflow { extent: Extent2 },
    CoordinateRangeExceeded { extent: Extent2 },
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CellCountOverflow { extent } => write!(
                f,
                "topology cell count overflows usize: {} x {}",
                extent.width, extent.height
            ),
            Self::CoordinateRangeExceeded { extent } => write!(
                f,
                "topology extent {} x {} cannot be represented by signed coordinates",
                extent.width, extent.height
            ),
        }
    }
}

impl Error for TopologyError {}

/// A four-neighbor rectangular square lattice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SquareTopology {
    extent: Extent2,
    boundaries: AxisBoundaries,
    cell_count: usize,
}

impl SquareTopology {
    pub fn new(extent: Extent2, boundaries: AxisBoundaries) -> Result<Self, TopologyError> {
        let cell_count = extent
            .checked_area()
            .ok_or(TopologyError::CellCountOverflow { extent })?;
        if extent.width > i32::MAX as usize + 1 || extent.height > i32::MAX as usize + 1 {
            return Err(TopologyError::CoordinateRangeExceeded { extent });
        }
        Ok(Self {
            extent,
            boundaries,
            cell_count,
        })
    }

    pub fn bounded(extent: Extent2) -> Result<Self, TopologyError> {
        Self::new(extent, AxisBoundaries::BOUNDED)
    }

    pub fn toroidal(extent: Extent2) -> Result<Self, TopologyError> {
        Self::new(extent, AxisBoundaries::TOROIDAL)
    }

    pub const fn extent(self) -> Extent2 {
        self.extent
    }

    pub const fn boundaries(self) -> AxisBoundaries {
        self.boundaries
    }

    fn neighbor_coordinate(&self, coord: Coord2, direction: SquareDirection) -> Option<Coord2> {
        let mut x = usize::try_from(coord.x).ok()?;
        let mut y = usize::try_from(coord.y).ok()?;

        match direction {
            SquareDirection::North if y > 0 => y -= 1,
            SquareDirection::North if self.boundaries.vertical == Boundary::Wrap => {
                y = self.extent.height.checked_sub(1)?;
            }
            SquareDirection::South if y + 1 < self.extent.height => y += 1,
            SquareDirection::South if self.boundaries.vertical == Boundary::Wrap => y = 0,
            SquareDirection::West if x > 0 => x -= 1,
            SquareDirection::West if self.boundaries.horizontal == Boundary::Wrap => {
                x = self.extent.width.checked_sub(1)?;
            }
            SquareDirection::East if x + 1 < self.extent.width => x += 1,
            SquareDirection::East if self.boundaries.horizontal == Boundary::Wrap => x = 0,
            _ => return None,
        }

        Some(Coord2::new(x as i32, y as i32))
    }
}

impl Topology for SquareTopology {
    type Coord = Coord2;
    type Direction = SquareDirection;

    fn cell_count(&self) -> usize {
        self.cell_count
    }

    fn cell_at(&self, coord: Self::Coord) -> Option<CellId> {
        self.extent.linear_index(coord).map(CellId::new)
    }

    fn coordinate(&self, cell: CellId) -> Option<Self::Coord> {
        self.extent.coordinate(cell.index())
    }

    fn neighbor(&self, cell: CellId, direction: Self::Direction) -> Option<CellId> {
        let coord = self.coordinate(cell)?;
        let neighbor = self.neighbor_coordinate(coord, direction)?;
        self.cell_at(neighbor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(topology: &SquareTopology, x: i32, y: i32) -> CellId {
        topology.cell_at(Coord2::new(x, y)).unwrap()
    }

    #[test]
    fn directions_have_opposites_and_dense_indices() {
        for (index, direction) in SquareDirection::ALL.iter().copied().enumerate() {
            assert_eq!(direction.index(), index);
            assert_eq!(direction.opposite().opposite(), direction);
        }
    }

    #[test]
    fn bounded_neighbors_stop_at_edges() {
        let topology = SquareTopology::bounded(Extent2::new(3, 2)).unwrap();
        let top_left = cell(&topology, 0, 0);
        assert_eq!(topology.neighbor(top_left, SquareDirection::North), None);
        assert_eq!(topology.neighbor(top_left, SquareDirection::West), None);
        assert_eq!(
            topology.coordinate(topology.neighbor(top_left, SquareDirection::East).unwrap()),
            Some(Coord2::new(1, 0))
        );
    }

    #[test]
    fn axes_wrap_independently() {
        let topology = SquareTopology::new(
            Extent2::new(3, 2),
            AxisBoundaries::new(Boundary::Wrap, Boundary::Bounded),
        )
        .unwrap();
        let top_left = cell(&topology, 0, 0);
        assert_eq!(
            topology.coordinate(topology.neighbor(top_left, SquareDirection::West).unwrap()),
            Some(Coord2::new(2, 0))
        );
        assert_eq!(topology.neighbor(top_left, SquareDirection::North), None);
    }

    #[test]
    fn torus_wraps_both_axes() {
        let topology = SquareTopology::toroidal(Extent2::new(3, 2)).unwrap();
        let top_left = cell(&topology, 0, 0);
        assert_eq!(
            topology.coordinate(topology.neighbor(top_left, SquareDirection::North).unwrap()),
            Some(Coord2::new(0, 1))
        );
        assert_eq!(
            topology.coordinate(topology.neighbor(top_left, SquareDirection::West).unwrap()),
            Some(Coord2::new(2, 0))
        );
    }

    #[test]
    fn one_cell_torus_has_self_neighbors() {
        let topology = SquareTopology::toroidal(Extent2::new(1, 1)).unwrap();
        let only = cell(&topology, 0, 0);
        for direction in SquareDirection::ALL {
            assert_eq!(topology.neighbor(only, *direction), Some(only));
        }
    }

    #[test]
    fn empty_topology_has_no_cells_or_neighbors() {
        let topology = SquareTopology::toroidal(Extent2::new(0, 4)).unwrap();
        assert_eq!(topology.cell_count(), 0);
        assert_eq!(topology.cell_at(Coord2::ZERO), None);
        assert_eq!(
            topology.neighbor(CellId::new(0), SquareDirection::East),
            None
        );
    }

    #[test]
    fn coordinates_and_ids_round_trip() {
        let topology = SquareTopology::bounded(Extent2::new(4, 3)).unwrap();
        for index in 0..topology.cell_count() {
            let id = CellId::new(index);
            assert_eq!(topology.cell_at(topology.coordinate(id).unwrap()), Some(id));
        }
    }
}
