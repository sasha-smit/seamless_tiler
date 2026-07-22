use std::collections::HashSet;

use seamless_tiler::{
    Coord2, D4, D6, Direction, EdgeStrip, Extent2, Grid, HexDirection, SocketMap, SquareDirection,
};

pub(crate) const SQUARE_RASTER_SIZE: usize = 32;
pub(crate) const HEX_EDGE_SAMPLES: usize = 12;
pub(crate) const EDGE_BACKGROUND: Rgba = [24, 27, 31, 255];
pub(crate) const DEFAULT_PAINT_COLOR: Rgba = [240, 240, 240, 255];
const TRANSPARENT: Rgba = [0, 0, 0, 0];
const HEX_EDGE_INTERVALS: i32 = HEX_EDGE_SAMPLES as i32 - 1;
const HEX_STORAGE_SIZE: usize = (HEX_EDGE_INTERVALS as usize) * 4 + 1;
const HEX_IMAGE_WIDTH: usize = (HEX_EDGE_INTERVALS as usize) * 6 + 1;
const HEX_IMAGE_HEIGHT: usize = HEX_STORAGE_SIZE;

pub(crate) type Rgba = [u8; 4];

/// The authoritative sample surface one editor mode paints and matches on.
///
/// Implementations expose their border geometry so the seamlessness assistant can
/// link, copy, and diagnose sides without knowing the cell shape. Every side
/// carries the same number of ordered samples, and each corner sample is shared
/// with the adjoining side so linked families stay transitive through corners.
pub(crate) trait TileSurface: Clone + PartialEq {
    type Direction: Direction;

    /// The number of ordered samples on every side.
    const EDGE_SAMPLES: usize;

    /// Returns the sample at `index` along one side, in canonical order.
    ///
    /// Panics if `index` is not below [`Self::EDGE_SAMPLES`].
    fn edge_sample(direction: Self::Direction, index: usize) -> Coord2;

    /// Writes one sample, reporting whether it existed and changed.
    fn set_sample(&mut self, coord: Coord2, color: Rgba) -> bool;

    /// Re-extracts every side's ordered samples from the current picture.
    fn edge_strips(&self) -> SocketMap<Self::Direction, EdgeStrip<Rgba>>;

    /// Returns every sample covered by a continuous clipped brush stroke.
    fn stroke(&self, from: Coord2, to: Coord2, brush_size: usize) -> Vec<Coord2>;
}

/// Rectangular RGBA data ready for upload to a renderer texture.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct VariantImage {
    size: [usize; 2],
    rgba: Vec<u8>,
}

impl VariantImage {
    fn new(size: [usize; 2], rgba: Vec<u8>) -> Self {
        debug_assert_eq!(rgba.len(), size[0] * size[1] * 4);
        Self { size, rgba }
    }

    pub(crate) const fn size(&self) -> [usize; 2] {
        self.size
    }

    pub(crate) fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// A fixed-size square RGBA tile image.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SquareRaster {
    pixels: Grid<Rgba>,
}

impl SquareRaster {
    fn filled(color: Rgba) -> Self {
        Self {
            pixels: Grid::filled(Extent2::new(SQUARE_RASTER_SIZE, SQUARE_RASTER_SIZE), color)
                .expect("the fixed raster extent has a valid area"),
        }
    }

    pub(crate) fn get(&self, x: usize, y: usize) -> Rgba {
        self.pixels[Coord2::new(x as i32, y as i32)]
    }

    pub(crate) fn set(&mut self, x: usize, y: usize, color: Rgba) {
        self.pixels[Coord2::new(x as i32, y as i32)] = color;
    }

    pub(crate) fn set_pixel(&mut self, coord: Coord2, color: Rgba) -> bool {
        let Some(pixel) = self.pixels.get_mut(coord) else {
            return false;
        };
        if *pixel == color {
            return false;
        }
        *pixel = color;
        true
    }

    /// Paints a continuous line of square brush impressions.
    ///
    /// Brush impressions are clipped to the raster. Even-sized brushes are
    /// biased toward positive coordinates so the pointer's pixel remains the
    /// upper-left member of the central pair.
    #[cfg(test)]
    pub(crate) fn paint_stroke(
        &mut self,
        from: Coord2,
        to: Coord2,
        brush_size: usize,
        color: Rgba,
    ) -> bool {
        let mut changed = false;
        for coord in self.stroke_pixels(from, to, brush_size) {
            changed |= self.set_pixel(coord, color);
        }
        changed
    }

    /// Returns every pixel covered by a continuous clipped brush stroke.
    ///
    /// Pixels appear at most once, in row-major order, so callers can expand
    /// the stroke through constraints before applying it atomically.
    pub(crate) fn stroke_pixels(&self, from: Coord2, to: Coord2, brush_size: usize) -> Vec<Coord2> {
        let extent = Extent2::new(SQUARE_RASTER_SIZE, SQUARE_RASTER_SIZE);
        if brush_size == 0
            || brush_size > SQUARE_RASTER_SIZE
            || !extent.contains(from)
            || !extent.contains(to)
        {
            return Vec::new();
        }

        let mut covered = vec![false; SQUARE_RASTER_SIZE * SQUARE_RASTER_SIZE];
        let mut x = from.x;
        let mut y = from.y;
        let dx = (to.x - from.x).abs();
        let step_x = if from.x < to.x { 1 } else { -1 };
        let dy = -(to.y - from.y).abs();
        let step_y = if from.y < to.y { 1 } else { -1 };
        let mut error = dx + dy;

        loop {
            Self::mark_brush(&mut covered, Coord2::new(x, y), brush_size);
            if x == to.x && y == to.y {
                break;
            }
            let doubled_error = error * 2;
            if doubled_error >= dy {
                error += dy;
                x += step_x;
            }
            if doubled_error <= dx {
                error += dx;
                y += step_y;
            }
        }
        covered
            .into_iter()
            .enumerate()
            .filter(|(_, covered)| *covered)
            .map(|(index, _)| extent.coordinate(index).unwrap())
            .collect()
    }

    fn mark_brush(covered: &mut [bool], center: Coord2, brush_size: usize) {
        let Ok(size) = i32::try_from(brush_size) else {
            return;
        };
        let offset = (size - 1) / 2;
        for y in center.y - offset..center.y - offset + size {
            for x in center.x - offset..center.x - offset + size {
                let Some(index) = Extent2::new(SQUARE_RASTER_SIZE, SQUARE_RASTER_SIZE)
                    .linear_index(Coord2::new(x, y))
                else {
                    continue;
                };
                covered[index] = true;
            }
        }
    }

    pub(crate) fn to_variant_image(&self) -> VariantImage {
        VariantImage::new(
            [SQUARE_RASTER_SIZE, SQUARE_RASTER_SIZE],
            self.pixels.iter().flatten().copied().collect(),
        )
    }

    pub(crate) fn edge(&self, direction: SquareDirection) -> EdgeStrip<Rgba> {
        let samples = (0..SQUARE_RASTER_SIZE)
            .map(|index| match direction {
                SquareDirection::North => self.get(index, 0),
                SquareDirection::East => self.get(SQUARE_RASTER_SIZE - 1, index),
                SquareDirection::South => self.get(index, SQUARE_RASTER_SIZE - 1),
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
        let offset = SQUARE_RASTER_SIZE as i32 - 1;
        let mut out = Self::filled([0, 0, 0, 0]);
        for y in 0..SQUARE_RASTER_SIZE {
            for x in 0..SQUARE_RASTER_SIZE {
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

impl TileSurface for SquareRaster {
    type Direction = SquareDirection;

    const EDGE_SAMPLES: usize = SQUARE_RASTER_SIZE;

    fn edge_sample(direction: SquareDirection, index: usize) -> Coord2 {
        assert!(index < Self::EDGE_SAMPLES, "edge sample index out of range");
        let index = index as i32;
        let far = (SQUARE_RASTER_SIZE - 1) as i32;
        match direction {
            SquareDirection::North => Coord2::new(index, 0),
            SquareDirection::East => Coord2::new(far, index),
            SquareDirection::South => Coord2::new(index, far),
            SquareDirection::West => Coord2::new(0, index),
        }
    }

    fn set_sample(&mut self, coord: Coord2, color: Rgba) -> bool {
        self.set_pixel(coord, color)
    }

    fn edge_strips(&self) -> SocketMap<SquareDirection, EdgeStrip<Rgba>> {
        self.edges()
    }

    fn stroke(&self, from: Coord2, to: Coord2, brush_size: usize) -> Vec<Coord2> {
        self.stroke_pixels(from, to, brush_size)
    }
}

/// A fixed pointy-top hexagonal RGBA image on an axial sample lattice.
///
/// The rectangular backing grid contains transparent padding. Only coordinates
/// inside the six integer half-planes are authoritative raster samples.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HexRaster {
    pixels: Grid<Rgba>,
}

impl HexRaster {
    /// The rectangular texel dimensions of the exported hex image.
    pub(crate) const IMAGE_SIZE: [usize; 2] = [HEX_IMAGE_WIDTH, HEX_IMAGE_HEIGHT];

    /// The texel distance between the outermost sample centers on each axis.
    ///
    /// Hex samples are *points*, not areas: the outermost ones sit exactly on
    /// the cell's boundary and corners are shared with the adjoining side. The
    /// exported image therefore spans one texel more than the cell it depicts,
    /// and a renderer must draw it into bounds inflated by
    /// `IMAGE_SIZE / SAMPLE_SPAN` so sample centers land on the true geometry.
    pub(crate) const SAMPLE_SPAN: [usize; 2] = [HEX_IMAGE_WIDTH - 1, HEX_IMAGE_HEIGHT - 1];

    pub(crate) fn filled(color: Rgba) -> Self {
        let extent = Extent2::new(HEX_STORAGE_SIZE, HEX_STORAGE_SIZE);
        let pixels = Grid::from_fn(extent, |storage| {
            let axial = Self::storage_to_axial(storage);
            if Self::contains(axial) {
                color
            } else {
                TRANSPARENT
            }
        })
        .expect("the fixed hex raster extent has a valid area");
        Self { pixels }
    }

    pub(crate) fn contains(coord: Coord2) -> bool {
        let q = i64::from(coord.x);
        let r = i64::from(coord.y);
        let limit = i64::from(3 * HEX_EDGE_INTERVALS);
        (q - r).abs().max((2 * q + r).abs()).max((q + 2 * r).abs()) <= limit
    }

    pub(crate) fn get(&self, coord: Coord2) -> Option<Rgba> {
        Self::contains(coord).then(|| self.pixels[Self::axial_to_storage(coord)])
    }

    pub(crate) fn set_pixel(&mut self, coord: Coord2, color: Rgba) -> bool {
        if !Self::contains(coord) {
            return false;
        }
        let pixel = &mut self.pixels[Self::axial_to_storage(coord)];
        if *pixel == color {
            return false;
        }
        *pixel = color;
        true
    }

    pub(crate) fn coordinates(&self) -> impl Iterator<Item = Coord2> + '_ {
        self.pixels.coordinates().filter_map(|storage| {
            let axial = Self::storage_to_axial(storage);
            Self::contains(axial).then_some(axial)
        })
    }

    /// Returns one side's start vertex and its per-sample axial step.
    fn edge_ray(direction: HexDirection) -> (Coord2, Coord2) {
        let k = HEX_EDGE_INTERVALS;
        match direction {
            HexDirection::NorthEast => (Coord2::new(k, -2 * k), Coord2::new(1, 1)),
            HexDirection::East => (Coord2::new(2 * k, -k), Coord2::new(-1, 2)),
            HexDirection::SouthEast => (Coord2::new(k, k), Coord2::new(-2, 1)),
            HexDirection::SouthWest => (Coord2::new(-2 * k, k), Coord2::new(1, 1)),
            HexDirection::West => (Coord2::new(-k, -k), Coord2::new(-1, 2)),
            HexDirection::NorthWest => (Coord2::new(k, -2 * k), Coord2::new(-2, 1)),
        }
    }

    pub(crate) fn edge_coordinates(direction: HexDirection) -> [Coord2; HEX_EDGE_SAMPLES] {
        std::array::from_fn(|index| Self::edge_sample(direction, index))
    }

    pub(crate) fn edge(&self, direction: HexDirection) -> EdgeStrip<Rgba> {
        EdgeStrip::new(
            Self::edge_coordinates(direction)
                .into_iter()
                .map(|coord| self.get(coord).expect("hex edge coordinates are valid"))
                .collect(),
        )
    }

    pub(crate) fn edges(&self) -> SocketMap<HexDirection, EdgeStrip<Rgba>> {
        SocketMap::from_fn(|direction| self.edge(direction))
    }

    pub(crate) fn transformed(&self, transform: D6) -> Self {
        let mut out = Self::filled(TRANSPARENT);
        for coord in self.coordinates() {
            let mapped = transform
                .checked_apply(coord)
                .expect("fixed hex raster coordinates cannot overflow");
            debug_assert!(Self::contains(mapped));
            out.set_pixel(
                mapped,
                self.get(coord).expect("iterated hex coordinates are valid"),
            );
        }
        out
    }

    pub(crate) fn to_variant_image(&self) -> VariantImage {
        let mut rgba = Vec::with_capacity(HEX_IMAGE_WIDTH * HEX_IMAGE_HEIGHT * 4);
        for y in 0..HEX_IMAGE_HEIGHT {
            for x in 0..HEX_IMAGE_WIDTH {
                let pixel = Self::sample_at_texel(x as i32, y as i32)
                    .and_then(|coord| self.get(coord))
                    .unwrap_or(TRANSPARENT);
                rgba.extend(pixel);
            }
        }
        VariantImage::new(Self::IMAGE_SIZE, rgba)
    }

    /// Returns the sample owning an exported texel, or `None` outside the hex.
    ///
    /// Texel columns are twice as dense as samples, so each sample owns the two
    /// columns nearest its center. Export, editor hit testing, and editor
    /// overlays share this mapping so they cannot disagree.
    pub(crate) fn sample_at_texel(x: i32, y: i32) -> Option<Coord2> {
        let k = i64::from(HEX_EDGE_INTERVALS);
        let u = i64::from(x) - 3 * k;
        let r = i64::from(y) - 2 * k;
        if u.abs() > 3 * k || (u - 3 * r).abs() > 6 * k || (u + 3 * r).abs() > 6 * k {
            return None;
        }
        let r = r as i32;
        let u = u as i32;
        let q_floor = (u - r).div_euclid(2);
        [q_floor, q_floor + 1]
            .into_iter()
            .map(|q| Coord2::new(q, r))
            .filter(|coord| Self::contains(*coord))
            .min_by_key(|coord| ((2 * coord.x + coord.y) - u).abs())
    }

    /// Returns the exported texel holding a sample's center.
    pub(crate) fn sample_texel(coord: Coord2) -> [i32; 2] {
        let k = HEX_EDGE_INTERVALS;
        [2 * coord.x + coord.y + 3 * k, coord.y + 2 * k]
    }

    /// Returns every in-mask sample covered by one brush impression.
    ///
    /// Impressions are hexagonal discs of radius `brush_size - 1`, so a
    /// single-sample brush marks only its center.
    pub(crate) fn brush_samples(center: Coord2, brush_size: usize) -> Vec<Coord2> {
        let Some(radius) = brush_size.checked_sub(1).and_then(|radius| {
            i32::try_from(radius)
                .ok()
                .filter(|radius| *radius <= HEX_EDGE_INTERVALS)
        }) else {
            return Vec::new();
        };
        let mut samples = Vec::new();
        for dq in -radius..=radius {
            for dr in (-radius).max(-dq - radius)..=radius.min(-dq + radius) {
                let coord = Coord2::new(center.x + dq, center.y + dr);
                if Self::contains(coord) {
                    samples.push(coord);
                }
            }
        }
        samples
    }

    /// Returns every sample covered by a continuous clipped brush stroke.
    ///
    /// Samples appear at most once, in storage order, so callers can expand the
    /// stroke through constraints before applying it atomically.
    pub(crate) fn stroke_samples(
        &self,
        from: Coord2,
        to: Coord2,
        brush_size: usize,
    ) -> Vec<Coord2> {
        if brush_size == 0
            || brush_size > HEX_EDGE_SAMPLES
            || !Self::contains(from)
            || !Self::contains(to)
        {
            return Vec::new();
        }
        let steps = hex_distance(from, to);
        let mut covered = HashSet::new();
        for step in 0..=steps {
            let fraction = if steps == 0 {
                0.0
            } else {
                f64::from(step) / f64::from(steps)
            };
            covered.extend(Self::brush_samples(
                interpolate_sample(from, to, fraction),
                brush_size,
            ));
        }
        self.coordinates()
            .filter(|coord| covered.contains(coord))
            .collect()
    }

    /// Paints a continuous line of hexagonal brush impressions.
    #[cfg(test)]
    pub(crate) fn paint_stroke(
        &mut self,
        from: Coord2,
        to: Coord2,
        brush_size: usize,
        color: Rgba,
    ) -> bool {
        let mut changed = false;
        for coord in self.stroke_samples(from, to, brush_size) {
            changed |= self.set_pixel(coord, color);
        }
        changed
    }

    fn axial_to_storage(coord: Coord2) -> Coord2 {
        let offset = 2 * HEX_EDGE_INTERVALS;
        Coord2::new(coord.x + offset, coord.y + offset)
    }

    fn storage_to_axial(coord: Coord2) -> Coord2 {
        let offset = 2 * HEX_EDGE_INTERVALS;
        Coord2::new(coord.x - offset, coord.y - offset)
    }
}

impl TileSurface for HexRaster {
    type Direction = HexDirection;

    const EDGE_SAMPLES: usize = HEX_EDGE_SAMPLES;

    fn edge_sample(direction: HexDirection, index: usize) -> Coord2 {
        assert!(index < Self::EDGE_SAMPLES, "edge sample index out of range");
        let (start, tangent) = Self::edge_ray(direction);
        let index = index as i32;
        Coord2::new(start.x + tangent.x * index, start.y + tangent.y * index)
    }

    fn set_sample(&mut self, coord: Coord2, color: Rgba) -> bool {
        self.set_pixel(coord, color)
    }

    fn edge_strips(&self) -> SocketMap<HexDirection, EdgeStrip<Rgba>> {
        self.edges()
    }

    fn stroke(&self, from: Coord2, to: Coord2, brush_size: usize) -> Vec<Coord2> {
        self.stroke_samples(from, to, brush_size)
    }
}

/// Builds the temporary procedural picture used until the pencil editor lands.
/// The outer border uses catalog-wide colors so strips can match across tiles
/// whose interior tints differ.
pub(crate) fn generate_square_raster(edge_mask: &[bool; 4], color: [u8; 3]) -> SquareRaster {
    let dim = |channel: u8| (channel as f32 * 0.30) as u8;
    let interior = [dim(color[0]), dim(color[1]), dim(color[2]), 255];
    let mut raster = SquareRaster::filled(EDGE_BACKGROUND);

    for y in 1..SQUARE_RASTER_SIZE - 1 {
        for x in 1..SQUARE_RASTER_SIZE - 1 {
            raster.set(x, y, interior);
        }
    }

    let center = (SQUARE_RASTER_SIZE as f32 - 1.0) / 2.0;
    let reach = center;
    let arm_half = SQUARE_RASTER_SIZE as f32 * 0.16;
    let hub_half = SQUARE_RASTER_SIZE as f32 * 0.18;
    for y in 0..SQUARE_RASTER_SIZE {
        for x in 0..SQUARE_RASTER_SIZE {
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
                raster.set(x, y, DEFAULT_PAINT_COLOR);
            }
        }
    }
    raster
}

/// Builds the temporary procedural hex picture used until the pencil editor
/// lands. Arms are generated on the axial lattice so rotations and reflections
/// remain exact sample permutations.
pub(crate) fn generate_hex_raster(edge_mask: &[bool; 6], color: [u8; 3]) -> HexRaster {
    const ARM_RADIUS: i32 = 3;

    let dim = |channel: u8| (channel as f32 * 0.30) as u8;
    let interior = [dim(color[0]), dim(color[1]), dim(color[2]), 255];
    let mut raster = HexRaster::filled(interior);

    for direction in HexDirection::ALL.iter().copied() {
        for coord in HexRaster::edge_coordinates(direction) {
            raster.set_pixel(coord, EDGE_BACKGROUND);
        }
    }

    let arm_reach = (3 * HEX_EDGE_INTERVALS) / 2;
    let rotations: [D6; 6] = std::array::from_fn(|index| {
        let direction = HexDirection::ALL[index];
        D6::ALL
            .into_iter()
            .find(|transform| {
                !transform.is_reflected()
                    && transform.apply_direction(HexDirection::NorthEast) == direction
            })
            .expect("every hex direction has a pure rotation")
    });
    let coordinates: Vec<_> = raster.coordinates().collect();
    for coord in coordinates {
        let in_hub = hex_distance(coord, Coord2::ZERO) <= ARM_RADIUS;
        let in_arm = edge_mask
            .iter()
            .copied()
            .enumerate()
            .any(|(index, active)| {
                active
                    && (0..=arm_reach).any(|step| {
                        let canonical = Coord2::new(step, -step);
                        let center = rotations[index]
                            .checked_apply(canonical)
                            .expect("fixed hex arm coordinates cannot overflow");
                        hex_distance(coord, center) <= ARM_RADIUS
                    })
            });
        if in_hub || in_arm {
            raster.set_pixel(coord, DEFAULT_PAINT_COLOR);
        }
    }

    raster
}

/// Interpolates between two axial samples in cube space.
///
/// Cube coordinates are `(q, -q - r, r)`; rounding restores their zero sum by
/// recomputing whichever component moved farthest, which keeps every step of a
/// stroke on the lattice.
fn interpolate_sample(from: Coord2, to: Coord2, fraction: f64) -> Coord2 {
    let x = f64::from(from.x) + (f64::from(to.x) - f64::from(from.x)) * fraction;
    let z = f64::from(from.y) + (f64::from(to.y) - f64::from(from.y)) * fraction;
    let y = -x - z;
    let mut rounded_x = x.round();
    let rounded_y = y.round();
    let mut rounded_z = z.round();
    let delta_x = (rounded_x - x).abs();
    let delta_y = (rounded_y - y).abs();
    let delta_z = (rounded_z - z).abs();
    if delta_x > delta_y && delta_x > delta_z {
        rounded_x = -rounded_y - rounded_z;
    } else if delta_y <= delta_z {
        rounded_z = -rounded_x - rounded_y;
    }
    Coord2::new(rounded_x as i32, rounded_z as i32)
}

fn hex_distance(left: Coord2, right: Coord2) -> i32 {
    let dq = (left.x - right.x).abs();
    let dr = (left.y - right.y).abs();
    let ds = ((left.x + left.y) - (right.x + right.y)).abs();
    dq.max(dr).max(ds)
}

#[cfg(test)]
pub(crate) fn closed_edge() -> EdgeStrip<Rgba> {
    EdgeStrip::new(vec![EDGE_BACKGROUND; SQUARE_RASTER_SIZE])
}

#[cfg(test)]
pub(crate) fn closed_hex_edge() -> EdgeStrip<Rgba> {
    EdgeStrip::new(vec![EDGE_BACKGROUND; HEX_EDGE_SAMPLES])
}

pub(crate) fn is_closed_edge(edge: &EdgeStrip<Rgba>) -> bool {
    !edge.is_empty() && edge.iter().all(|pixel| *pixel == EDGE_BACKGROUND)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use seamless_tiler::{Direction, EdgeTransform, QuarterTurns, Tile};

    fn labeled_hex_raster() -> HexRaster {
        let mut raster = HexRaster::filled(TRANSPARENT);
        let coordinates: Vec<_> = raster.coordinates().collect();
        for (index, coord) in coordinates.into_iter().enumerate() {
            let label = index as u32 + 1;
            assert!(raster.set_pixel(coord, label.to_le_bytes()));
        }
        raster
    }

    #[test]
    fn hex_mask_contains_the_expected_pointy_top_lattice() {
        let mut raster = HexRaster::filled(DEFAULT_PAINT_COLOR);
        assert_eq!(raster.coordinates().count(), 1_123);
        assert_eq!(raster.get(Coord2::ZERO), Some(DEFAULT_PAINT_COLOR));

        let k = HEX_EDGE_INTERVALS;
        for vertex in [
            Coord2::new(k, -2 * k),
            Coord2::new(2 * k, -k),
            Coord2::new(k, k),
            Coord2::new(-k, 2 * k),
            Coord2::new(-2 * k, k),
            Coord2::new(-k, -k),
        ] {
            assert_eq!(raster.get(vertex), Some(DEFAULT_PAINT_COLOR));
        }

        let outside = Coord2::new(0, 2 * k + 1);
        assert_eq!(raster.get(outside), None);
        assert!(!raster.set_pixel(outside, EDGE_BACKGROUND));
        assert!(!HexRaster::contains(Coord2::new(i32::MIN, i32::MAX)));
    }

    #[test]
    fn hex_edges_have_canonical_order_and_share_only_corners() {
        let expected_tangents = [
            (HexDirection::NorthEast, Coord2::new(1, 1)),
            (HexDirection::East, Coord2::new(-1, 2)),
            (HexDirection::SouthEast, Coord2::new(-2, 1)),
            (HexDirection::SouthWest, Coord2::new(1, 1)),
            (HexDirection::West, Coord2::new(-1, 2)),
            (HexDirection::NorthWest, Coord2::new(-2, 1)),
        ];
        let mut memberships = HashMap::new();
        for (direction, expected_tangent) in expected_tangents {
            let edge = HexRaster::edge_coordinates(direction);
            assert_eq!(edge.len(), HEX_EDGE_SAMPLES);
            for pair in edge.windows(2) {
                assert_eq!(
                    Coord2::new(pair[1].x - pair[0].x, pair[1].y - pair[0].y),
                    expected_tangent,
                );
            }
            for coord in edge {
                assert!(HexRaster::contains(coord));
                *memberships.entry(coord).or_insert(0) += 1;
            }
        }

        assert_eq!(memberships.values().filter(|count| **count == 2).count(), 6);
        assert_eq!(
            memberships.values().filter(|count| **count == 1).count(),
            6 * (HEX_EDGE_SAMPLES - 2),
        );
        assert!(memberships.values().all(|count| *count <= 2));
    }

    #[test]
    fn d6_transforms_are_exact_permutations_of_hex_samples() {
        let raster = labeled_hex_raster();
        let source_coordinates: Vec<_> = raster.coordinates().collect();

        for transform in D6::ALL {
            let transformed = raster.transformed(transform);
            assert_eq!(transformed.coordinates().count(), source_coordinates.len());
            for coord in &source_coordinates {
                let mapped = transform.checked_apply(*coord).unwrap();
                assert_eq!(transformed.get(mapped), raster.get(*coord));
            }
            assert_eq!(
                transformed.transformed(transform.inverse()),
                raster,
                "{transform:?}",
            );
        }

        for left in D6::ALL {
            for right in D6::ALL {
                assert_eq!(
                    raster.transformed(right).transformed(left),
                    raster.transformed(left.compose(right)),
                    "{left:?} after {right:?}",
                );
            }
        }
    }

    #[test]
    fn oriented_hex_edges_equal_edges_from_transformed_rasters() {
        let raster = labeled_hex_raster();
        let tile = Tile::new((), raster.edges());
        for transform in D6::ALL {
            let transformed = raster.transformed(transform);
            for world_direction in HexDirection::ALL.iter().copied() {
                assert_eq!(
                    tile.oriented_edge(transform, world_direction),
                    transformed.edge(world_direction),
                    "{transform:?} {world_direction:?}",
                );
            }
        }
    }

    #[test]
    fn hex_variant_image_is_deterministic_and_transparent_outside() {
        let raster = HexRaster::filled(DEFAULT_PAINT_COLOR);
        let image = raster.to_variant_image();
        assert_eq!(image.size(), [HEX_IMAGE_WIDTH, HEX_IMAGE_HEIGHT]);
        assert_eq!(image.rgba().len(), HEX_IMAGE_WIDTH * HEX_IMAGE_HEIGHT * 4);
        assert_eq!(image, raster.to_variant_image());

        let pixel = |x: usize, y: usize| {
            let offset = (y * HEX_IMAGE_WIDTH + x) * 4;
            <Rgba>::try_from(&image.rgba()[offset..offset + 4]).unwrap()
        };
        assert_eq!(pixel(0, 0), TRANSPARENT);
        assert_eq!(
            pixel(3 * HEX_EDGE_INTERVALS as usize, 0),
            DEFAULT_PAINT_COLOR,
        );
        assert_eq!(
            pixel(
                3 * HEX_EDGE_INTERVALS as usize,
                2 * HEX_EDGE_INTERVALS as usize,
            ),
            DEFAULT_PAINT_COLOR,
        );
        let k = HEX_EDGE_INTERVALS;
        for y in 0..HEX_IMAGE_HEIGHT {
            let r = y as i32 - 2 * k;
            for x in 0..HEX_IMAGE_WIDTH {
                let u = x as i32 - 3 * k;
                let inside =
                    u.abs() <= 3 * k && (u - 3 * r).abs() <= 6 * k && (u + 3 * r).abs() <= 6 * k;
                assert_eq!(pixel(x, y)[3] != 0, inside, "texel ({x}, {y})");
            }
        }
    }

    #[test]
    fn hex_texel_mapping_round_trips_every_sample() {
        let raster = HexRaster::filled(DEFAULT_PAINT_COLOR);
        for coord in raster.coordinates() {
            let [x, y] = HexRaster::sample_texel(coord);
            assert_eq!(HexRaster::sample_at_texel(x, y), Some(coord), "{coord:?}");
        }

        let [width, height] = HexRaster::IMAGE_SIZE;
        assert_eq!(HexRaster::sample_at_texel(0, 0), None);
        assert_eq!(
            HexRaster::sample_at_texel(width as i32 - 1, height as i32 - 1),
            None
        );
        assert_eq!(HexRaster::sample_at_texel(i32::MIN, i32::MAX), None);
        assert_eq!(
            HexRaster::sample_at_texel(3 * HEX_EDGE_INTERVALS, 2 * HEX_EDGE_INTERVALS),
            Some(Coord2::ZERO)
        );
    }

    #[test]
    fn hex_brush_impressions_clip_at_the_mask_boundary() {
        assert_eq!(
            HexRaster::brush_samples(Coord2::ZERO, 1),
            vec![Coord2::ZERO]
        );
        assert_eq!(HexRaster::brush_samples(Coord2::ZERO, 0), Vec::new());
        assert_eq!(HexRaster::brush_samples(Coord2::ZERO, 3).len(), 19);
        assert!(!HexRaster::brush_samples(Coord2::ZERO, HEX_EDGE_SAMPLES).is_empty());

        let corner = Coord2::new(HEX_EDGE_INTERVALS, -2 * HEX_EDGE_INTERVALS);
        let clipped = HexRaster::brush_samples(corner, 3);
        assert!(clipped.iter().all(|coord| HexRaster::contains(*coord)));
        assert!(clipped.contains(&corner));
        assert!(clipped.len() < 19);
    }

    #[test]
    fn hex_strokes_interpolate_between_pointer_samples() {
        let raster = HexRaster::filled(EDGE_BACKGROUND);
        let from = Coord2::new(-4, 6);
        let to = Coord2::new(5, -3);
        let stroke = raster.stroke_samples(from, to, 1);

        let distance = hex_distance(from, to);
        assert_eq!(stroke.len(), distance as usize + 1);
        assert!(stroke.contains(&from));
        assert!(stroke.contains(&to));
        for coord in &stroke {
            assert_eq!(
                hex_distance(from, *coord) + hex_distance(*coord, to),
                distance,
                "{coord:?} leaves the interpolated line",
            );
        }

        let mut ordered = stroke.clone();
        ordered.sort_by_key(|coord| hex_distance(from, *coord));
        for pair in ordered.windows(2) {
            assert_eq!(hex_distance(pair[0], pair[1]), 1);
        }
    }

    #[test]
    fn identical_and_invalid_hex_strokes_are_no_ops() {
        let mut raster = HexRaster::filled(EDGE_BACKGROUND);
        assert!(!raster.paint_stroke(Coord2::ZERO, Coord2::ZERO, 1, EDGE_BACKGROUND));
        assert!(raster.paint_stroke(Coord2::ZERO, Coord2::ZERO, 1, DEFAULT_PAINT_COLOR));
        assert!(!raster.paint_stroke(Coord2::ZERO, Coord2::ZERO, 1, DEFAULT_PAINT_COLOR));

        let outside = Coord2::new(0, 2 * HEX_EDGE_INTERVALS + 1);
        assert!(raster.stroke_samples(Coord2::ZERO, outside, 1).is_empty());
        assert!(raster.stroke_samples(outside, Coord2::ZERO, 1).is_empty());
        assert!(
            raster
                .stroke_samples(Coord2::ZERO, Coord2::ZERO, 0)
                .is_empty()
        );
        assert!(
            raster
                .stroke_samples(Coord2::ZERO, Coord2::ZERO, HEX_EDGE_SAMPLES + 1)
                .is_empty()
        );
    }

    #[test]
    fn extraction_uses_matching_orders_for_opposite_edges() {
        let raster = SquareRaster {
            pixels: Grid::from_fn(
                Extent2::new(SQUARE_RASTER_SIZE, SQUARE_RASTER_SIZE),
                |coord| [coord.x as u8, coord.y as u8, 0, 255],
            )
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
        let raster = SquareRaster {
            pixels: Grid::from_fn(
                Extent2::new(SQUARE_RASTER_SIZE, SQUARE_RASTER_SIZE),
                |coord| [coord.x as u8, coord.y as u8, (coord.x + coord.y) as u8, 255],
            )
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
        let blank_a = generate_square_raster(&[false; 4], [255, 0, 0]);
        let blank_b = generate_square_raster(&[false; 4], [0, 255, 0]);
        assert_ne!(blank_a, blank_b);
        for direction in SquareDirection::ALL.iter().copied() {
            assert_eq!(blank_a.edge(direction), closed_edge());
            assert_eq!(blank_a.edge(direction), blank_b.edge(direction));
        }

        let north_a = generate_square_raster(&[true, false, false, false], [255, 0, 0]);
        let north_b = generate_square_raster(&[true, false, false, false], [0, 255, 0]);
        assert_eq!(
            north_a.edge(SquareDirection::North),
            north_b.edge(SquareDirection::North)
        );
        assert_ne!(north_a.edge(SquareDirection::North), closed_edge());
    }

    #[test]
    fn transform_round_trips_every_generated_raster() {
        let raster = generate_square_raster(&[true, true, false, false], [80, 120, 160]);
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
    fn generated_hex_edges_are_shared_and_closed_when_inactive() {
        let blank = generate_hex_raster(&[false; 6], [255, 0, 0]);
        for direction in HexDirection::ALL.iter().copied() {
            assert_eq!(blank.edge(direction), closed_hex_edge());
        }

        let mut active_profile = None;
        for direction in HexDirection::ALL.iter().copied() {
            let mut mask = [false; 6];
            mask[direction.index()] = true;
            let red = generate_hex_raster(&mask, [255, 0, 0]);
            let green = generate_hex_raster(&mask, [0, 255, 0]);
            assert_eq!(red.edge(direction), green.edge(direction));
            assert_ne!(red.edge(direction), closed_hex_edge());
            if let Some(profile) = &active_profile {
                assert_eq!(&red.edge(direction), profile);
            } else {
                active_profile = Some(red.edge(direction));
            }
            for inactive in HexDirection::ALL.iter().copied() {
                if inactive != direction {
                    assert_eq!(red.edge(inactive), closed_hex_edge());
                }
            }
        }
    }

    #[test]
    fn generated_hex_rasters_are_d6_equivariant() {
        for bits in 0_u8..64 {
            let mask = std::array::from_fn(|index| bits & (1 << index) != 0);
            let raster = generate_hex_raster(&mask, [80, 120, 160]);
            for transform in D6::ALL {
                let mut transformed_mask = [false; 6];
                for direction in HexDirection::ALL.iter().copied() {
                    transformed_mask[transform.apply_direction(direction).index()] =
                        mask[direction.index()];
                }
                assert_eq!(
                    raster.transformed(transform),
                    generate_hex_raster(&transformed_mask, [80, 120, 160]),
                    "mask {bits:06b}, transform {transform:?}",
                );
            }
        }
    }

    #[test]
    fn quarter_turn_reversal_matches_edge_transform_contract() {
        let turn = D4::new(QuarterTurns::One, false);
        assert!(!turn.reverses_edge(SquareDirection::North));
        assert!(turn.reverses_edge(SquareDirection::East));
    }

    #[test]
    fn stroke_interpolates_between_pointer_samples() {
        let mut raster = SquareRaster::filled(EDGE_BACKGROUND);
        assert!(raster.paint_stroke(Coord2::new(2, 3), Coord2::new(8, 6), 1, DEFAULT_PAINT_COLOR,));

        let painted: Vec<_> = raster
            .pixels
            .cells()
            .filter_map(|(coord, pixel)| (*pixel == DEFAULT_PAINT_COLOR).then_some(coord))
            .collect();
        assert_eq!(painted.len(), 7);
        assert_eq!(painted.first(), Some(&Coord2::new(2, 3)));
        assert_eq!(painted.last(), Some(&Coord2::new(8, 6)));
        for pair in painted.windows(2) {
            let dx = pair[1].x - pair[0].x;
            let dy = pair[1].y - pair[0].y;
            assert!(dx.abs() <= 1 && dy.abs() <= 1);
        }
    }

    #[test]
    fn brush_footprints_clip_at_raster_edges() {
        let mut raster = SquareRaster::filled(EDGE_BACKGROUND);
        assert!(raster.paint_stroke(Coord2::ZERO, Coord2::ZERO, 4, DEFAULT_PAINT_COLOR,));

        let painted = raster
            .pixels
            .iter()
            .filter(|pixel| **pixel == DEFAULT_PAINT_COLOR)
            .count();
        assert_eq!(painted, 9);
        assert_eq!(raster.get(2, 2), DEFAULT_PAINT_COLOR);
        assert_eq!(raster.get(3, 0), EDGE_BACKGROUND);
    }

    #[test]
    fn identical_and_zero_sized_strokes_are_no_ops() {
        let mut raster = SquareRaster::filled(EDGE_BACKGROUND);
        assert!(!raster.paint_stroke(Coord2::ZERO, Coord2::ZERO, 1, EDGE_BACKGROUND));
        assert!(!raster.paint_stroke(Coord2::ZERO, Coord2::new(4, 4), 0, DEFAULT_PAINT_COLOR,));
    }
}
