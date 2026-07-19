use crate::{Coord2, HexDirection, SquareDirection};

/// Maps directions through a reversible tile orientation.
pub trait DirectionTransform<D>: Copy {
    fn apply_direction(self, direction: D) -> D;
    fn inverse(self) -> Self;
}

/// Clockwise quarter-turn rotations in screen coordinates (`x` right, `y` down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum QuarterTurns {
    #[default]
    Zero,
    One,
    Two,
    Three,
}

impl QuarterTurns {
    pub const ALL: [Self; 4] = [Self::Zero, Self::One, Self::Two, Self::Three];

    const fn count(self) -> u8 {
        self as u8
    }

    const fn from_count(count: u8) -> Self {
        match count % 4 {
            0 => Self::Zero,
            1 => Self::One,
            2 => Self::Two,
            _ => Self::Three,
        }
    }
}

/// One of the eight symmetries of a square.
///
/// A reflected transform first mirrors `x` across the vertical axis, then applies its
/// clockwise rotation. Transformations act around the origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct D4 {
    rotation: QuarterTurns,
    reflected: bool,
}

impl D4 {
    pub const IDENTITY: Self = Self::new(QuarterTurns::Zero, false);
    pub const ALL: [Self; 8] = [
        Self::new(QuarterTurns::Zero, false),
        Self::new(QuarterTurns::One, false),
        Self::new(QuarterTurns::Two, false),
        Self::new(QuarterTurns::Three, false),
        Self::new(QuarterTurns::Zero, true),
        Self::new(QuarterTurns::One, true),
        Self::new(QuarterTurns::Two, true),
        Self::new(QuarterTurns::Three, true),
    ];

    pub const fn new(rotation: QuarterTurns, reflected: bool) -> Self {
        Self {
            rotation,
            reflected,
        }
    }

    pub const fn rotation(self) -> QuarterTurns {
        self.rotation
    }

    pub const fn is_reflected(self) -> bool {
        self.reflected
    }

    /// Applies the transform to a displacement, returning `None` on signed overflow.
    pub fn checked_apply(self, mut vector: Coord2) -> Option<Coord2> {
        if self.reflected {
            vector.x = vector.x.checked_neg()?;
        }
        match self.rotation {
            QuarterTurns::Zero => Some(vector),
            QuarterTurns::One => Some(Coord2::new(vector.y.checked_neg()?, vector.x)),
            QuarterTurns::Two => Some(Coord2::new(
                vector.x.checked_neg()?,
                vector.y.checked_neg()?,
            )),
            QuarterTurns::Three => Some(Coord2::new(vector.y, vector.x.checked_neg()?)),
        }
    }

    pub const fn apply_direction(self, direction: SquareDirection) -> SquareDirection {
        let mut index = direction as u8;
        if self.reflected {
            index = match index {
                1 => 3,
                3 => 1,
                other => other,
            };
        }
        match (index + self.rotation.count()) % 4 {
            0 => SquareDirection::North,
            1 => SquareDirection::East,
            2 => SquareDirection::South,
            _ => SquareDirection::West,
        }
    }

    /// Returns `self` applied after `other`.
    pub const fn compose(self, other: Self) -> Self {
        let rotation = if self.reflected {
            (self.rotation.count() + 4 - other.rotation.count()) % 4
        } else {
            (self.rotation.count() + other.rotation.count()) % 4
        };
        Self::new(
            QuarterTurns::from_count(rotation),
            self.reflected != other.reflected,
        )
    }

    pub const fn inverse(self) -> Self {
        let rotation = if self.reflected {
            self.rotation.count()
        } else {
            (4 - self.rotation.count()) % 4
        };
        Self::new(QuarterTurns::from_count(rotation), self.reflected)
    }
}

impl DirectionTransform<SquareDirection> for D4 {
    fn apply_direction(self, direction: SquareDirection) -> SquareDirection {
        D4::apply_direction(self, direction)
    }

    fn inverse(self) -> Self {
        D4::inverse(self)
    }
}

/// Clockwise sixth-turn rotations in screen coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum SixthTurns {
    #[default]
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
}

impl SixthTurns {
    pub const ALL: [Self; 6] = [
        Self::Zero,
        Self::One,
        Self::Two,
        Self::Three,
        Self::Four,
        Self::Five,
    ];

    const fn count(self) -> u8 {
        self as u8
    }

    const fn from_count(count: u8) -> Self {
        match count % 6 {
            0 => Self::Zero,
            1 => Self::One,
            2 => Self::Two,
            3 => Self::Three,
            4 => Self::Four,
            _ => Self::Five,
        }
    }
}

/// One of the twelve symmetries of a regular hexagon.
///
/// A reflected transform first mirrors the axial vector across the vertical screen axis, then
/// applies its clockwise rotation. [`Self::checked_apply`] therefore interprets [`Coord2`] as
/// an axial `(q, r)` vector rather than an offset-grid coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct D6 {
    rotation: SixthTurns,
    reflected: bool,
}

impl D6 {
    pub const IDENTITY: Self = Self::new(SixthTurns::Zero, false);
    pub const ALL: [Self; 12] = [
        Self::new(SixthTurns::Zero, false),
        Self::new(SixthTurns::One, false),
        Self::new(SixthTurns::Two, false),
        Self::new(SixthTurns::Three, false),
        Self::new(SixthTurns::Four, false),
        Self::new(SixthTurns::Five, false),
        Self::new(SixthTurns::Zero, true),
        Self::new(SixthTurns::One, true),
        Self::new(SixthTurns::Two, true),
        Self::new(SixthTurns::Three, true),
        Self::new(SixthTurns::Four, true),
        Self::new(SixthTurns::Five, true),
    ];

    pub const fn new(rotation: SixthTurns, reflected: bool) -> Self {
        Self {
            rotation,
            reflected,
        }
    }

    pub const fn rotation(self) -> SixthTurns {
        self.rotation
    }

    pub const fn is_reflected(self) -> bool {
        self.reflected
    }

    /// Applies the transform to an axial displacement, returning `None` on signed overflow.
    pub fn checked_apply(self, mut vector: Coord2) -> Option<Coord2> {
        if self.reflected {
            vector.x = vector.x.checked_neg()?.checked_sub(vector.y)?;
        }
        for _ in 0..self.rotation.count() {
            vector = Coord2::new(vector.y.checked_neg()?, vector.x.checked_add(vector.y)?);
        }
        Some(vector)
    }

    pub const fn apply_direction(self, direction: HexDirection) -> HexDirection {
        let mut index = direction as u8;
        if self.reflected {
            index = 5 - index;
        }
        match (index + self.rotation.count()) % 6 {
            0 => HexDirection::NorthEast,
            1 => HexDirection::East,
            2 => HexDirection::SouthEast,
            3 => HexDirection::SouthWest,
            4 => HexDirection::West,
            _ => HexDirection::NorthWest,
        }
    }

    /// Returns `self` applied after `other`.
    pub const fn compose(self, other: Self) -> Self {
        let rotation = if self.reflected {
            (self.rotation.count() + 6 - other.rotation.count()) % 6
        } else {
            (self.rotation.count() + other.rotation.count()) % 6
        };
        Self::new(
            SixthTurns::from_count(rotation),
            self.reflected != other.reflected,
        )
    }

    pub const fn inverse(self) -> Self {
        let rotation = if self.reflected {
            self.rotation.count()
        } else {
            (6 - self.rotation.count()) % 6
        };
        Self::new(SixthTurns::from_count(rotation), self.reflected)
    }
}

impl DirectionTransform<HexDirection> for D6 {
    fn apply_direction(self, direction: HexDirection) -> HexDirection {
        D6::apply_direction(self, direction)
    }

    fn inverse(self) -> Self {
        D6::inverse(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Direction;

    #[test]
    fn all_contains_eight_distinct_transforms() {
        let mut transforms = D4::ALL.to_vec();
        transforms.sort_unstable();
        transforms.dedup();
        assert_eq!(transforms.len(), 8);
    }

    #[test]
    fn clockwise_rotation_maps_north_to_east() {
        let rotation = D4::new(QuarterTurns::One, false);
        assert_eq!(
            rotation.apply_direction(SquareDirection::North),
            SquareDirection::East
        );
        assert_eq!(
            rotation.checked_apply(Coord2::new(0, -1)),
            Some(Coord2::new(1, 0))
        );
    }

    #[test]
    fn reflection_mirrors_east_and_west() {
        let reflection = D4::new(QuarterTurns::Zero, true);
        assert_eq!(
            reflection.apply_direction(SquareDirection::East),
            SquareDirection::West
        );
        assert_eq!(
            reflection.apply_direction(SquareDirection::North),
            SquareDirection::North
        );
    }

    #[test]
    fn inverse_and_composition_laws_hold_exhaustively() {
        let vectors = [Coord2::new(2, 3), Coord2::new(-5, 7), Coord2::new(11, -13)];
        for transform in D4::ALL {
            assert_eq!(transform.compose(transform.inverse()), D4::IDENTITY);
            assert_eq!(transform.inverse().compose(transform), D4::IDENTITY);
            for vector in vectors {
                let transformed = transform.checked_apply(vector).unwrap();
                assert_eq!(transform.inverse().checked_apply(transformed), Some(vector));
            }
        }

        for left in D4::ALL {
            for right in D4::ALL {
                for direction in SquareDirection::ALL {
                    assert_eq!(
                        left.compose(right).apply_direction(*direction),
                        left.apply_direction(right.apply_direction(*direction))
                    );
                }
            }
        }
    }

    #[test]
    fn checked_application_reports_overflow() {
        let reflection = D4::new(QuarterTurns::Zero, true);
        assert_eq!(reflection.checked_apply(Coord2::new(i32::MIN, 0)), None);
    }

    #[test]
    fn d6_contains_twelve_distinct_transforms() {
        let mut transforms = D6::ALL.to_vec();
        transforms.sort_unstable();
        transforms.dedup();
        assert_eq!(transforms.len(), 12);
    }

    #[test]
    fn d6_rotation_and_reflection_map_hex_directions() {
        let rotation = D6::new(SixthTurns::One, false);
        assert_eq!(
            rotation.apply_direction(HexDirection::NorthEast),
            HexDirection::East
        );
        assert_eq!(
            rotation.checked_apply(Coord2::new(1, -1)),
            Some(Coord2::new(1, 0))
        );

        let reflection = D6::new(SixthTurns::Zero, true);
        assert_eq!(
            reflection.apply_direction(HexDirection::NorthEast),
            HexDirection::NorthWest
        );
        assert_eq!(
            reflection.apply_direction(HexDirection::East),
            HexDirection::West
        );
    }

    #[test]
    fn d6_inverse_and_composition_laws_hold_exhaustively() {
        let vectors = [Coord2::new(2, 3), Coord2::new(-5, 7), Coord2::new(11, -13)];
        for transform in D6::ALL {
            assert_eq!(transform.compose(transform.inverse()), D6::IDENTITY);
            assert_eq!(transform.inverse().compose(transform), D6::IDENTITY);
            for vector in vectors {
                let transformed = transform.checked_apply(vector).unwrap();
                assert_eq!(transform.inverse().checked_apply(transformed), Some(vector));
            }
        }

        for left in D6::ALL {
            for right in D6::ALL {
                for direction in HexDirection::ALL {
                    assert_eq!(
                        left.compose(right).apply_direction(*direction),
                        left.apply_direction(right.apply_direction(*direction))
                    );
                }
            }
        }
    }

    #[test]
    fn d6_checked_application_reports_overflow() {
        let reflection = D6::new(SixthTurns::Zero, true);
        assert_eq!(reflection.checked_apply(Coord2::new(i32::MIN, 0)), None);
    }
}
