use seamless_tiler::{Coord2, D4, EdgeStrip, Extent2, Grid, SocketMap, SquareDirection};

pub(crate) const RASTER_SIZE: usize = 32;
pub(crate) const EDGE_BACKGROUND: Rgba = [24, 27, 31, 255];
const EDGE_PATH: Rgba = [240, 240, 240, 255];

pub(crate) type Rgba = [u8; 4];

/// A fixed-size square RGBA tile image.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Raster {
    pixels: Grid<Rgba>,
}

impl Raster {
    fn filled(color: Rgba) -> Self {
        Self {
            pixels: Grid::filled(Extent2::new(RASTER_SIZE, RASTER_SIZE), color)
                .expect("the fixed raster extent has a valid area"),
        }
    }

    pub(crate) fn get(&self, x: usize, y: usize) -> Rgba {
        self.pixels[Coord2::new(x as i32, y as i32)]
    }

    pub(crate) fn set(&mut self, x: usize, y: usize, color: Rgba) {
        self.pixels[Coord2::new(x as i32, y as i32)] = color;
    }

    pub(crate) fn bytes(&self) -> Vec<u8> {
        self.pixels.iter().flatten().copied().collect()
    }

    pub(crate) fn edge(&self, direction: SquareDirection) -> EdgeStrip<Rgba> {
        let samples = (0..RASTER_SIZE)
            .map(|index| match direction {
                SquareDirection::North => self.get(index, 0),
                SquareDirection::East => self.get(RASTER_SIZE - 1, index),
                SquareDirection::South => self.get(index, RASTER_SIZE - 1),
                SquareDirection::West => self.get(0, index),
            })
            .collect();
        EdgeStrip::new(samples)
    }

    pub(crate) fn edges(&self) -> SocketMap<SquareDirection, EdgeStrip<Rgba>> {
        SocketMap::from_fn(|direction| self.edge(direction))
    }

    /// Applies a D4 symmetry using doubled, center-relative pixel coordinates.
    pub(crate) fn transformed(&self, transform: D4) -> Self {
        let offset = RASTER_SIZE as i32 - 1;
        let mut out = Self::filled([0, 0, 0, 0]);
        for y in 0..RASTER_SIZE {
            for x in 0..RASTER_SIZE {
                let vector = Coord2::new(2 * x as i32 - offset, 2 * y as i32 - offset);
                let mapped = transform
                    .checked_apply(vector)
                    .expect("fixed raster coordinates cannot overflow");
                let nx = ((mapped.x + offset) / 2) as usize;
                let ny = ((mapped.y + offset) / 2) as usize;
                out.set(nx, ny, self.get(x, y));
            }
        }
        out
    }
}

/// Builds the temporary procedural picture used until the pencil editor lands.
/// The outer border uses catalog-wide colors so strips can match across tiles
/// whose interior tints differ.
pub(crate) fn generate_raster(edge_mask: &[bool; 4], color: [u8; 3]) -> Raster {
    let dim = |channel: u8| (channel as f32 * 0.30) as u8;
    let interior = [dim(color[0]), dim(color[1]), dim(color[2]), 255];
    let mut raster = Raster::filled(EDGE_BACKGROUND);

    for y in 1..RASTER_SIZE - 1 {
        for x in 1..RASTER_SIZE - 1 {
            raster.set(x, y, interior);
        }
    }

    let center = (RASTER_SIZE as f32 - 1.0) / 2.0;
    let reach = center;
    let arm_half = RASTER_SIZE as f32 * 0.16;
    let hub_half = RASTER_SIZE as f32 * 0.18;
    for y in 0..RASTER_SIZE {
        for x in 0..RASTER_SIZE {
            let rx = x as f32 - center;
            let ry = y as f32 - center;
            let mut on_path = rx.abs() <= hub_half && ry.abs() <= hub_half;
            if !on_path {
                for (index, active) in edge_mask.iter().copied().enumerate() {
                    if !active {
                        continue;
                    }
                    let (dx, dy) = match index {
                        0 => (0.0, -1.0),
                        1 => (1.0, 0.0),
                        2 => (0.0, 1.0),
                        _ => (-1.0, 0.0),
                    };
                    let along = rx * dx + ry * dy;
                    let perpendicular = rx * -dy + ry * dx;
                    if along >= 0.0 && along <= reach && perpendicular.abs() <= arm_half {
                        on_path = true;
                        break;
                    }
                }
            }
            if on_path {
                raster.set(x, y, EDGE_PATH);
            }
        }
    }
    raster
}

#[cfg(test)]
pub(crate) fn closed_edge() -> EdgeStrip<Rgba> {
    EdgeStrip::new(vec![EDGE_BACKGROUND; RASTER_SIZE])
}

pub(crate) fn is_closed_edge(edge: &EdgeStrip<Rgba>) -> bool {
    edge.len() == RASTER_SIZE && edge.iter().all(|pixel| *pixel == EDGE_BACKGROUND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use seamless_tiler::{Direction, EdgeTransform, QuarterTurns, Tile};

    #[test]
    fn extraction_uses_matching_orders_for_opposite_edges() {
        let raster = Raster {
            pixels: Grid::from_fn(Extent2::new(RASTER_SIZE, RASTER_SIZE), |coord| {
                [coord.x as u8, coord.y as u8, 0, 255]
            })
            .unwrap(),
        };
        assert_eq!(
            raster.edge(SquareDirection::North).as_slice()[0],
            [0, 0, 0, 255]
        );
        assert_eq!(
            raster.edge(SquareDirection::South).as_slice()[0],
            [0, 31, 0, 255]
        );
        assert_eq!(
            raster.edge(SquareDirection::East).as_slice()[0],
            [31, 0, 0, 255]
        );
        assert_eq!(
            raster.edge(SquareDirection::West).as_slice()[31],
            [0, 31, 0, 255]
        );
    }

    #[test]
    fn oriented_edges_equal_edges_extracted_from_transformed_rasters() {
        let raster = Raster {
            pixels: Grid::from_fn(Extent2::new(RASTER_SIZE, RASTER_SIZE), |coord| {
                [coord.x as u8, coord.y as u8, (coord.x + coord.y) as u8, 255]
            })
            .unwrap(),
        };
        let tile = Tile::new((), raster.edges());
        for transform in D4::ALL {
            let transformed = raster.transformed(transform);
            for world_direction in SquareDirection::ALL.iter().copied() {
                assert_eq!(
                    tile.oriented_edge(transform, world_direction),
                    transformed.edge(world_direction),
                    "{transform:?} {world_direction:?}"
                );
            }
        }
    }

    #[test]
    fn generated_edges_are_catalog_independent_and_closed_when_inactive() {
        let blank_a = generate_raster(&[false; 4], [255, 0, 0]);
        let blank_b = generate_raster(&[false; 4], [0, 255, 0]);
        assert_ne!(blank_a, blank_b);
        for direction in SquareDirection::ALL.iter().copied() {
            assert_eq!(blank_a.edge(direction), closed_edge());
            assert_eq!(blank_a.edge(direction), blank_b.edge(direction));
        }

        let north_a = generate_raster(&[true, false, false, false], [255, 0, 0]);
        let north_b = generate_raster(&[true, false, false, false], [0, 255, 0]);
        assert_eq!(
            north_a.edge(SquareDirection::North),
            north_b.edge(SquareDirection::North)
        );
        assert_ne!(north_a.edge(SquareDirection::North), closed_edge());
    }

    #[test]
    fn transform_round_trips_every_generated_raster() {
        let raster = generate_raster(&[true, true, false, false], [80, 120, 160]);
        for transform in D4::ALL {
            assert_eq!(
                raster
                    .transformed(transform)
                    .transformed(transform.inverse()),
                raster
            );
        }
    }

    #[test]
    fn quarter_turn_reversal_matches_edge_transform_contract() {
        let turn = D4::new(QuarterTurns::One, false);
        assert!(!turn.reverses_edge(SquareDirection::North));
        assert!(turn.reverses_edge(SquareDirection::East));
    }
}
