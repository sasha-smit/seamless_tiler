use std::error::Error;
use std::fmt;
use std::ops::{Index, IndexMut};

use crate::{Coord2, Extent2};

/// An error constructing a [`Grid`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GridError {
    AreaOverflow { extent: Extent2 },
    LengthMismatch { expected: usize, actual: usize },
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AreaOverflow { extent } => write!(
                f,
                "grid area overflows usize: {} x {}",
                extent.width, extent.height
            ),
            Self::LengthMismatch { expected, actual } => {
                write!(f, "grid needs {expected} cells, but received {actual}")
            }
        }
    }
}

impl Error for GridError {}

/// Owned, dense, row-major rectangular storage.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Grid<T> {
    extent: Extent2,
    cells: Vec<T>,
}

impl<T> Grid<T> {
    /// Constructs a grid from row-major storage.
    pub fn from_vec(extent: Extent2, cells: Vec<T>) -> Result<Self, GridError> {
        let expected = extent
            .checked_area()
            .ok_or(GridError::AreaOverflow { extent })?;
        if cells.len() != expected {
            return Err(GridError::LengthMismatch {
                expected,
                actual: cells.len(),
            });
        }
        Ok(Self { extent, cells })
    }

    /// Constructs a grid by visiting coordinates in row-major order.
    pub fn from_fn(
        extent: Extent2,
        mut make_cell: impl FnMut(Coord2) -> T,
    ) -> Result<Self, GridError> {
        let len = extent
            .checked_area()
            .ok_or(GridError::AreaOverflow { extent })?;
        let mut cells = Vec::with_capacity(len);
        for index in 0..len {
            // Every index below a representable area has a coordinate unless a dimension
            // exceeds the signed coordinate range. Treat that as an unrepresentable grid.
            let coord = extent
                .coordinate(index)
                .ok_or(GridError::AreaOverflow { extent })?;
            cells.push(make_cell(coord));
        }
        Ok(Self { extent, cells })
    }

    pub const fn extent(&self) -> Extent2 {
        self.extent
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn get(&self, coord: Coord2) -> Option<&T> {
        self.extent
            .linear_index(coord)
            .and_then(|index| self.cells.get(index))
    }

    pub fn get_mut(&mut self, coord: Coord2) -> Option<&mut T> {
        self.extent
            .linear_index(coord)
            .and_then(|index| self.cells.get_mut(index))
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.cells.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.cells.iter_mut()
    }

    pub fn coordinates(&self) -> impl ExactSizeIterator<Item = Coord2> + '_ {
        (0..self.cells.len()).map(|index| {
            self.extent
                .coordinate(index)
                .expect("a valid grid index must have a coordinate")
        })
    }

    pub fn cells(&self) -> impl ExactSizeIterator<Item = (Coord2, &T)> + '_ {
        self.coordinates().zip(self.cells.iter())
    }

    pub fn cells_mut(&mut self) -> impl ExactSizeIterator<Item = (Coord2, &mut T)> + '_ {
        let extent = self.extent;
        self.cells.iter_mut().enumerate().map(move |(index, cell)| {
            let coord = extent
                .coordinate(index)
                .expect("a valid grid index must have a coordinate");
            (coord, cell)
        })
    }

    pub fn into_vec(self) -> Vec<T> {
        self.cells
    }
}

impl<T: Clone> Grid<T> {
    pub fn filled(extent: Extent2, value: T) -> Result<Self, GridError> {
        let len = extent
            .checked_area()
            .ok_or(GridError::AreaOverflow { extent })?;
        Self::from_vec(extent, vec![value; len])
    }
}

impl<T> Index<Coord2> for Grid<T> {
    type Output = T;

    fn index(&self, coord: Coord2) -> &Self::Output {
        self.get(coord).unwrap_or_else(|| {
            panic!(
                "grid coordinate ({}, {}) is outside {} x {}",
                coord.x, coord.y, self.extent.width, self.extent.height
            )
        })
    }
}

impl<T> IndexMut<Coord2> for Grid<T> {
    fn index_mut(&mut self, coord: Coord2) -> &mut Self::Output {
        let extent = self.extent;
        self.get_mut(coord).unwrap_or_else(|| {
            panic!(
                "grid coordinate ({}, {}) is outside {} x {}",
                coord.x, coord.y, extent.width, extent.height
            )
        })
    }
}

impl<T> IntoIterator for Grid<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Grid<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Grid<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_and_visits_in_row_major_order() {
        let grid = Grid::from_fn(Extent2::new(3, 2), |coord| coord.x + coord.y * 10).unwrap();
        assert_eq!(grid.into_vec(), vec![0, 1, 2, 10, 11, 12]);
    }

    #[test]
    fn checked_and_index_access_work() {
        let mut grid = Grid::filled(Extent2::new(2, 2), 0).unwrap();
        grid[Coord2::new(1, 0)] = 7;
        assert_eq!(grid.get(Coord2::new(1, 0)), Some(&7));
        assert_eq!(grid.get(Coord2::new(-1, 0)), None);
        assert_eq!(grid.get(Coord2::new(2, 0)), None);
    }

    #[test]
    fn validates_storage_and_overflow() {
        assert_eq!(
            Grid::<u8>::from_vec(Extent2::new(2, 2), vec![0; 3]),
            Err(GridError::LengthMismatch {
                expected: 4,
                actual: 3
            })
        );
        assert!(matches!(
            Grid::<u8>::from_vec(Extent2::new(usize::MAX, 2), vec![]),
            Err(GridError::AreaOverflow { .. })
        ));
    }

    #[test]
    fn allows_empty_extents() {
        let grid = Grid::<u8>::from_vec(Extent2::new(0, 4), vec![]).unwrap();
        assert!(grid.is_empty());
        assert_eq!(grid.coordinates().count(), 0);
    }

    #[test]
    fn iterates_coordinates_with_values() {
        let grid = Grid::from_vec(Extent2::new(2, 1), vec!['a', 'b']).unwrap();
        assert_eq!(
            grid.cells().collect::<Vec<_>>(),
            vec![(Coord2::new(0, 0), &'a'), (Coord2::new(1, 0), &'b')]
        );
    }
}
