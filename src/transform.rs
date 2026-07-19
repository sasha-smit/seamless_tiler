use crate::{Coord2, SquareDirection};

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
}
