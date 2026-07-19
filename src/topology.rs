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

/// The six edge-sharing directions of a pointy-top hexagonal cell.
///
/// Values are ordered clockwise in screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum HexDirection {
    NorthEast,
    East,
    SouthEast,
    SouthWest,
    West,
    NorthWest,
}

impl Direction for HexDirection {
    const ALL: &'static [Self] = &[
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::SouthWest,
        Self::West,
        Self::NorthWest,
    ];

    fn index(self) -> usize {
        self as usize
    }

    fn opposite(self) -> Self {
        match self {
            Self::NorthEast => Self::SouthWest,
            Self::East => Self::West,
            Self::SouthEast => Self::NorthWest,
            Self::SouthWest => Self::NorthEast,
            Self::West => Self::East,
            Self::NorthWest => Self::SouthEast,
        }
    }
}

impl HexDirection {
    /// Returns this direction's offset in odd-row offset coordinates.
    pub const fn offset(self, row: i32) -> Coord2 {
        let odd = row & 1 != 0;
        match (self, odd) {
            (Self::NorthEast, false) => Coord2::new(0, -1),
            (Self::NorthEast, true) => Coord2::new(1, -1),
            (Self::East, _) => Coord2::new(1, 0),
            (Self::SouthEast, false) => Coord2::new(0, 1),
            (Self::SouthEast, true) => Coord2::new(1, 1),
            (Self::SouthWest, false) => Coord2::new(-1, 1),
            (Self::SouthWest, true) => Coord2::new(0, 1),
            (Self::West, _) => Coord2::new(-1, 0),
            (Self::NorthWest, false) => Coord2::new(-1, -1),
            (Self::NorthWest, true) => Coord2::new(0, -1),
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

/// Validates that an extent has a representable area and signed coordinates, returning its
/// cell count.
fn validated_cell_count(extent: Extent2) -> Result<usize, TopologyError> {
    let cell_count = extent
        .checked_area()
        .ok_or(TopologyError::CellCountOverflow { extent })?;
    if extent.width > i32::MAX as usize + 1 || extent.height > i32::MAX as usize + 1 {
        return Err(TopologyError::CoordinateRangeExceeded { extent });
    }
    Ok(cell_count)
}

/// A four-neighbor rectangular square lattice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SquareTopology {
    extent: Extent2,
    boundaries: AxisBoundaries,
    cell_count: usize,
}

impl SquareTopology {
    pub fn new(extent: Extent2, boundaries: AxisBoundaries) -> Result<Self, TopologyError> {
        let cell_count = validated_cell_count(extent)?;
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

/// A six-neighbor pointy-top hexagonal lattice in odd-row offset coordinates.
///
/// Coordinates remain a dense rectangle: odd rows are visually shifted half a cell to the
/// right. Wrapped vertical seams pair opposite edges reciprocally even when the height is odd.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HexTopology {
    extent: Extent2,
    boundaries: AxisBoundaries,
    cell_count: usize,
}

impl HexTopology {
    pub fn new(extent: Extent2, boundaries: AxisBoundaries) -> Result<Self, TopologyError> {
        let cell_count = validated_cell_count(extent)?;
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

    fn shift_horizontal(&self, x: usize, delta: i32) -> Option<usize> {
        match delta {
            -1 if x > 0 => Some(x - 1),
            -1 if self.boundaries.horizontal == Boundary::Wrap => self.extent.width.checked_sub(1),
            0 => Some(x),
            1 if x + 1 < self.extent.width => Some(x + 1),
            1 if self.boundaries.horizontal == Boundary::Wrap => Some(0),
            _ => None,
        }
    }

    fn neighbor_coordinate(&self, coord: Coord2, direction: HexDirection) -> Option<Coord2> {
        let x = usize::try_from(coord.x).ok()?;
        let y = usize::try_from(coord.y).ok()?;
        let odd = y & 1 != 0;

        let (delta_x, neighbor_y) = match direction {
            HexDirection::East => (1, y),
            HexDirection::West => (-1, y),
            HexDirection::NorthEast if y > 0 => (i32::from(odd), y - 1),
            HexDirection::NorthWest if y > 0 => (i32::from(odd) - 1, y - 1),
            HexDirection::SouthEast if y + 1 < self.extent.height => (i32::from(odd), y + 1),
            HexDirection::SouthWest if y + 1 < self.extent.height => (i32::from(odd) - 1, y + 1),
            HexDirection::NorthEast if self.boundaries.vertical == Boundary::Wrap => {
                (0, self.extent.height.checked_sub(1)?)
            }
            HexDirection::NorthWest if self.boundaries.vertical == Boundary::Wrap => {
                (-1, self.extent.height.checked_sub(1)?)
            }
            HexDirection::SouthEast if self.boundaries.vertical == Boundary::Wrap => (1, 0),
            HexDirection::SouthWest if self.boundaries.vertical == Boundary::Wrap => (0, 0),
            _ => return None,
        };
        let neighbor_x = self.shift_horizontal(x, delta_x)?;
        Some(Coord2::new(neighbor_x as i32, neighbor_y as i32))
    }
}

impl Topology for HexTopology {
    type Coord = Coord2;
    type Direction = HexDirection;

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
    fn square_offsets_agree_with_bounded_neighbors() {
        let topology = SquareTopology::bounded(Extent2::new(4, 3)).unwrap();
        for index in 0..topology.cell_count() {
            let id = CellId::new(index);
            let coord = topology.coordinate(id).unwrap();
            for direction in SquareDirection::ALL.iter().copied() {
                let offset = direction.offset();
                let shifted = Coord2::new(coord.x + offset.x, coord.y + offset.y);
                assert_eq!(
                    topology.neighbor(id, direction),
                    topology.cell_at(shifted),
                    "{coord:?} {direction:?}"
                );
            }
        }
    }

    #[test]
    fn coordinates_and_ids_round_trip() {
        let topology = SquareTopology::bounded(Extent2::new(4, 3)).unwrap();
        for index in 0..topology.cell_count() {
            let id = CellId::new(index);
            assert_eq!(topology.cell_at(topology.coordinate(id).unwrap()), Some(id));
        }
    }

    fn hex_cell(topology: &HexTopology, x: i32, y: i32) -> CellId {
        topology.cell_at(Coord2::new(x, y)).unwrap()
    }

    #[test]
    fn hex_directions_have_opposites_and_dense_indices() {
        for (index, direction) in HexDirection::ALL.iter().copied().enumerate() {
            assert_eq!(direction.index(), index);
            assert_eq!(direction.opposite().opposite(), direction);
        }
    }

    #[test]
    fn hex_neighbors_follow_odd_row_offsets() {
        let topology = HexTopology::bounded(Extent2::new(4, 3)).unwrap();
        let even = hex_cell(&topology, 1, 0);
        let odd = hex_cell(&topology, 1, 1);
        assert_eq!(
            topology.coordinate(topology.neighbor(even, HexDirection::SouthEast).unwrap()),
            Some(Coord2::new(1, 1))
        );
        assert_eq!(
            topology.coordinate(topology.neighbor(odd, HexDirection::SouthEast).unwrap()),
            Some(Coord2::new(2, 2))
        );
        assert_eq!(topology.neighbor(even, HexDirection::NorthEast), None);
    }

    #[test]
    fn hex_axes_wrap_independently() {
        let horizontal = HexTopology::new(
            Extent2::new(3, 3),
            AxisBoundaries::new(Boundary::Wrap, Boundary::Bounded),
        )
        .unwrap();
        let top_left = hex_cell(&horizontal, 0, 0);
        assert_eq!(
            horizontal.coordinate(horizontal.neighbor(top_left, HexDirection::West).unwrap()),
            Some(Coord2::new(2, 0))
        );
        assert_eq!(horizontal.neighbor(top_left, HexDirection::NorthEast), None);

        let vertical = HexTopology::new(
            Extent2::new(3, 3),
            AxisBoundaries::new(Boundary::Bounded, Boundary::Wrap),
        )
        .unwrap();
        assert_eq!(
            vertical.coordinate(
                vertical
                    .neighbor(top_left, HexDirection::NorthEast)
                    .unwrap()
            ),
            Some(Coord2::new(0, 2))
        );
    }

    #[test]
    fn hex_neighbors_are_reciprocal_for_all_boundary_modes() {
        for extent in [Extent2::new(1, 1), Extent2::new(4, 3), Extent2::new(3, 4)] {
            for horizontal in [Boundary::Bounded, Boundary::Wrap] {
                for vertical in [Boundary::Bounded, Boundary::Wrap] {
                    let topology =
                        HexTopology::new(extent, AxisBoundaries::new(horizontal, vertical))
                            .unwrap();
                    for index in 0..topology.cell_count() {
                        let cell = CellId::new(index);
                        for direction in HexDirection::ALL.iter().copied() {
                            let Some(neighbor) = topology.neighbor(cell, direction) else {
                                continue;
                            };
                            assert_eq!(
                                topology.neighbor(neighbor, direction.opposite()),
                                Some(cell),
                                "{extent:?} {horizontal:?} {vertical:?} {cell:?} {direction:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn one_cell_hex_torus_has_six_self_neighbors() {
        let topology = HexTopology::toroidal(Extent2::new(1, 1)).unwrap();
        let only = hex_cell(&topology, 0, 0);
        for direction in HexDirection::ALL {
            assert_eq!(topology.neighbor(only, *direction), Some(only));
        }
    }

    #[test]
    fn empty_hex_topology_has_no_cells_or_neighbors() {
        let topology = HexTopology::toroidal(Extent2::new(0, 4)).unwrap();
        assert_eq!(topology.cell_count(), 0);
        assert_eq!(topology.cell_at(Coord2::ZERO), None);
        assert_eq!(topology.neighbor(CellId::new(0), HexDirection::East), None);
    }

    #[test]
    fn hex_offsets_agree_with_bounded_neighbors() {
        let topology = HexTopology::bounded(Extent2::new(4, 5)).unwrap();
        for index in 0..topology.cell_count() {
            let id = CellId::new(index);
            let coord = topology.coordinate(id).unwrap();
            for direction in HexDirection::ALL.iter().copied() {
                let offset = direction.offset(coord.y);
                let shifted = Coord2::new(coord.x + offset.x, coord.y + offset.y);
                assert_eq!(
                    topology.neighbor(id, direction),
                    topology.cell_at(shifted),
                    "{coord:?} {direction:?}"
                );
            }
        }
    }

    #[test]
    fn hex_coordinates_and_ids_round_trip() {
        let topology = HexTopology::bounded(Extent2::new(4, 3)).unwrap();
        for index in 0..topology.cell_count() {
            let id = CellId::new(index);
            assert_eq!(topology.cell_at(topology.coordinate(id).unwrap()), Some(id));
        }
    }
}
