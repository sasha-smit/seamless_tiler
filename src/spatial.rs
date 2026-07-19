/// A signed coordinate or displacement in a two-dimensional integer space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Coord2 {
    pub x: i32,
    pub y: i32,
}

impl Coord2 {
    pub const ZERO: Self = Self::new(0, 0);

    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// The non-negative dimensions of a rectangular region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Extent2 {
    pub width: usize,
    pub height: usize,
}

impl Extent2 {
    pub const fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    /// Returns the number of cells, or `None` if the multiplication overflows.
    pub const fn checked_area(self) -> Option<usize> {
        self.width.checked_mul(self.height)
    }

    pub fn contains(self, coord: Coord2) -> bool {
        let Ok(x) = usize::try_from(coord.x) else {
            return false;
        };
        let Ok(y) = usize::try_from(coord.y) else {
            return false;
        };
        x < self.width && y < self.height
    }

    /// Converts an in-bounds coordinate to a row-major index.
    pub fn linear_index(self, coord: Coord2) -> Option<usize> {
        if !self.contains(coord) {
            return None;
        }
        let x = coord.x as usize;
        let y = coord.y as usize;
        y.checked_mul(self.width)?.checked_add(x)
    }

    /// Converts a row-major index to a coordinate.
    pub fn coordinate(self, index: usize) -> Option<Coord2> {
        let area = self.checked_area()?;
        if index >= area || self.width == 0 {
            return None;
        }
        let x = index % self.width;
        let y = index / self.width;
        Some(Coord2::new(i32::try_from(x).ok()?, i32::try_from(y).ok()?))
    }
}

/// An axis-aligned integer rectangle with an inclusive origin and exclusive upper bound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub origin: Coord2,
    pub extent: Extent2,
}

impl Rect {
    pub const fn new(origin: Coord2, extent: Extent2) -> Self {
        Self { origin, extent }
    }

    pub fn contains(self, coord: Coord2) -> bool {
        let Some(dx) = coord.x.checked_sub(self.origin.x) else {
            return false;
        };
        let Some(dy) = coord.y.checked_sub(self.origin.y) else {
            return false;
        };
        self.extent.contains(Coord2::new(dx, dy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extent_maps_coordinates_and_indices() {
        let extent = Extent2::new(3, 2);
        assert_eq!(extent.linear_index(Coord2::new(2, 1)), Some(5));
        assert_eq!(extent.coordinate(5), Some(Coord2::new(2, 1)));
        assert_eq!(extent.linear_index(Coord2::new(-1, 0)), None);
        assert_eq!(extent.coordinate(6), None);
    }

    #[test]
    fn empty_and_overlarge_extents_are_handled() {
        assert_eq!(Extent2::new(0, 10).checked_area(), Some(0));
        assert_eq!(Extent2::new(usize::MAX, 2).checked_area(), None);
        assert_eq!(Extent2::new(usize::MAX, 1).coordinate(usize::MAX - 1), None);
    }

    #[test]
    fn rect_contains_coordinates_relative_to_its_origin() {
        let rect = Rect::new(Coord2::new(-2, 4), Extent2::new(3, 2));
        assert!(rect.contains(Coord2::new(-2, 4)));
        assert!(rect.contains(Coord2::new(0, 5)));
        assert!(!rect.contains(Coord2::new(1, 5)));
        assert!(!rect.contains(Coord2::new(-2, 6)));
    }
}
