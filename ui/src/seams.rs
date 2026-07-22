//! Shape-independent seamlessness assistance for the tile catalog.
//!
//! Two sides can face each other only when their strips are byte-identical, and
//! a rotated tile may present a side reversed, so sides whose strips are equal or
//! reversed form one *family* that must be edited together. Families are linked
//! at the level of individual border samples, which makes corner samples — shared
//! by two sides of the same tile — propagate transitively.
//!
//! Every routine here works from [`TileSurface`] geometry alone, so square and
//! hex catalogs share one implementation.

use std::collections::{HashMap, HashSet};

use seamless_tiler::{
    Coord2, Direction, EdgeStrip, HexDirection, SquareDirection, TileId, TileSet,
};

use crate::raster::{Rgba, TileSurface};

/// The largest number of sides any editor mode's cell shape has.
pub(crate) const MAX_TILE_SIDES: usize = 6;

/// Per-side orphan flags for one tile, indexed by the mode's direction index.
///
/// Slots past the mode's own side count stay `false`.
pub(crate) type OrphanEdges = [bool; MAX_TILE_SIDES];

/// One side of one catalog tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EdgeRef<D> {
    pub(crate) tile: TileId,
    pub(crate) direction: D,
}

pub(crate) type SquareEdgeRef = EdgeRef<SquareDirection>;
pub(crate) type HexEdgeRef = EdgeRef<HexDirection>;

/// One authoritative sample of one catalog tile's surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TileSample {
    pub(crate) tile: TileId,
    pub(crate) coord: Coord2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeCopyResult {
    Applied,
    NoChange,
    Conflict,
    Invalid,
}

/// Groups every side of the catalog by its reversal-independent strip.
///
/// The key is the smaller of the strip and its reverse, and each member records
/// whether it must be reversed to read in the key's order.
pub(crate) fn edge_families<S, P, D>(
    tiles: &TileSet<P, D, EdgeStrip<Rgba>>,
) -> HashMap<EdgeStrip<Rgba>, Vec<(EdgeRef<D>, bool)>>
where
    S: TileSurface<Direction = D>,
    D: Direction,
{
    let mut families: HashMap<EdgeStrip<Rgba>, Vec<(EdgeRef<D>, bool)>> = HashMap::new();
    for (tile, value) in tiles.iter() {
        for direction in D::ALL.iter().copied() {
            let edge = &value.sockets[direction];
            let reversed = edge.reversed();
            let (key, reversed_to_key) = if reversed.as_slice() < edge.as_slice() {
                (reversed, true)
            } else {
                (edge.clone(), false)
            };
            families
                .entry(key)
                .or_default()
                .push((EdgeRef { tile, direction }, reversed_to_key));
        }
    }
    families
}

/// Reports the sides that have no partner to match, per tile.
pub(crate) fn orphan_edges<S, P, D>(
    tiles: &TileSet<P, D, EdgeStrip<Rgba>>,
) -> Vec<(TileId, OrphanEdges)>
where
    S: TileSurface<Direction = D>,
    D: Direction,
{
    let mut family_sizes = vec![[0_usize; MAX_TILE_SIDES]; tiles.len()];
    for members in edge_families::<S, P, D>(tiles).into_values() {
        for (edge, _) in &members {
            family_sizes[edge.tile.index()][edge.direction.index()] = members.len();
        }
    }
    family_sizes
        .into_iter()
        .enumerate()
        .map(|(tile, sizes)| {
            (
                TileId::new(tile),
                std::array::from_fn(|side| side < D::ALL.len() && sizes[side] < 2),
            )
        })
        .collect()
}

/// Border samples of the whole catalog, grouped into components that must always
/// hold the same color.
pub(crate) struct EdgeLinkIndex {
    component_by_sample: HashMap<TileSample, usize>,
    components: Vec<Vec<TileSample>>,
}

impl EdgeLinkIndex {
    pub(crate) fn new<S, P, D>(tiles: &TileSet<P, D, EdgeStrip<Rgba>>) -> Self
    where
        S: TileSurface<Direction = D>,
        D: Direction,
    {
        let border = border_coordinates::<S, D>();
        let slots: HashMap<Coord2, usize> = border
            .iter()
            .copied()
            .enumerate()
            .map(|(slot, coord)| (coord, slot))
            .collect();
        let node = |tile: TileId, coord: Coord2| {
            let slot = slots
                .get(&coord)
                .expect("linked samples are always border samples");
            tile.index() * border.len() + slot
        };

        let mut sets = DisjointSets::new(tiles.len() * border.len());
        for members in edge_families::<S, P, D>(tiles).into_values() {
            let Some((first, first_reversed)) = members.first().copied() else {
                continue;
            };
            for canonical_index in 0..S::EDGE_SAMPLES {
                let first_node = node(
                    first.tile,
                    S::edge_sample(
                        first.direction,
                        aligned_edge_index::<S>(canonical_index, first_reversed),
                    ),
                );
                for (edge, reversed_to_key) in members.iter().copied().skip(1) {
                    let local_index = aligned_edge_index::<S>(canonical_index, reversed_to_key);
                    sets.union(
                        first_node,
                        node(edge.tile, S::edge_sample(edge.direction, local_index)),
                    );
                }
            }
        }

        let mut grouped: HashMap<usize, Vec<TileSample>> = HashMap::new();
        for tile_index in 0..tiles.len() {
            let tile = TileId::new(tile_index);
            for coord in border.iter().copied() {
                grouped
                    .entry(sets.find(node(tile, coord)))
                    .or_default()
                    .push(TileSample { tile, coord });
            }
        }

        let mut component_by_sample = HashMap::new();
        let mut components = Vec::with_capacity(grouped.len());
        for samples in grouped.into_values() {
            let component = components.len();
            for sample in &samples {
                component_by_sample.insert(*sample, component);
            }
            components.push(samples);
        }
        Self {
            component_by_sample,
            components,
        }
    }

    /// Returns every sample that must share this sample's color, or `None` for
    /// interior samples, which are linked to nothing.
    pub(crate) fn linked_samples(&self, sample: TileSample) -> Option<&[TileSample]> {
        let component = *self.component_by_sample.get(&sample)?;
        Some(&self.components[component])
    }
}

/// Plans a copy of one side onto another, or reports why it cannot apply.
///
/// Every write is collected before any is made, so a plan that would assign two
/// colors to one linked corner is rejected without touching the catalog.
pub(crate) fn plan_edge_copy<S, P, D>(
    tiles: &TileSet<P, D, EdgeStrip<Rgba>>,
    source: EdgeRef<D>,
    target_tile: TileId,
    target_direction: D,
    reverse: bool,
) -> Result<HashMap<TileSample, Rgba>, EdgeCopyResult>
where
    S: TileSurface<Direction = D>,
    D: Direction,
{
    let Some(source_value) = tiles.get(source.tile) else {
        return Err(EdgeCopyResult::Invalid);
    };
    if tiles.get(target_tile).is_none() {
        return Err(EdgeCopyResult::Invalid);
    }
    let source_edge = &source_value.sockets[source.direction];
    let desired = if reverse {
        source_edge.reversed()
    } else {
        source_edge.clone()
    };

    let links = EdgeLinkIndex::new::<S, P, D>(tiles);
    let mut assignments = HashMap::new();
    for (index, color) in desired.iter().copied().enumerate() {
        let target = TileSample {
            tile: target_tile,
            coord: S::edge_sample(target_direction, index),
        };
        let Some(linked) = links.linked_samples(target) else {
            return Err(EdgeCopyResult::Invalid);
        };
        for sample in linked {
            if assignments
                .insert(*sample, color)
                .is_some_and(|existing| existing != color)
            {
                return Err(EdgeCopyResult::Conflict);
            }
        }
    }
    Ok(assignments)
}

/// Every distinct border sample of one surface, in direction and index order.
///
/// Corners appear once, under the first side that reaches them.
fn border_coordinates<S, D>() -> Vec<Coord2>
where
    S: TileSurface<Direction = D>,
    D: Direction,
{
    let mut seen = HashSet::new();
    let mut border = Vec::new();
    for direction in D::ALL.iter().copied() {
        for index in 0..S::EDGE_SAMPLES {
            let coord = S::edge_sample(direction, index);
            if seen.insert(coord) {
                border.push(coord);
            }
        }
    }
    border
}

/// Translates a canonical family index into one member's own sample order.
fn aligned_edge_index<S: TileSurface>(canonical_index: usize, reversed_to_key: bool) -> usize {
    if reversed_to_key {
        S::EDGE_SAMPLES - 1 - canonical_index
    } else {
        canonical_index
    }
}

struct DisjointSets {
    parents: Vec<usize>,
    ranks: Vec<u8>,
}

impl DisjointSets {
    fn new(len: usize) -> Self {
        Self {
            parents: (0..len).collect(),
            ranks: vec![0; len],
        }
    }

    fn find(&mut self, node: usize) -> usize {
        if self.parents[node] != node {
            self.parents[node] = self.find(self.parents[node]);
        }
        self.parents[node]
    }

    fn union(&mut self, left: usize, right: usize) {
        let left = self.find(left);
        let right = self.find(right);
        if left == right {
            return;
        }
        match self.ranks[left].cmp(&self.ranks[right]) {
            std::cmp::Ordering::Less => self.parents[left] = right,
            std::cmp::Ordering::Greater => self.parents[right] = left,
            std::cmp::Ordering::Equal => {
                self.parents[right] = left;
                self.ranks[left] += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{HEX_EDGE_SAMPLES, HexRaster, SQUARE_RASTER_SIZE, SquareRaster};

    #[test]
    fn border_coordinates_cover_every_side_and_share_corners() {
        let square = border_coordinates::<SquareRaster, SquareDirection>();
        assert_eq!(square.len(), 4 * (SQUARE_RASTER_SIZE - 1));
        let hex = border_coordinates::<HexRaster, HexDirection>();
        assert_eq!(hex.len(), 6 * (HEX_EDGE_SAMPLES - 1));

        for border in [square, hex] {
            let distinct: HashSet<_> = border.iter().copied().collect();
            assert_eq!(distinct.len(), border.len(), "border samples repeat");
        }
    }

    #[test]
    fn aligned_indices_mirror_only_reversed_members() {
        assert_eq!(aligned_edge_index::<HexRaster>(0, false), 0);
        assert_eq!(
            aligned_edge_index::<HexRaster>(0, true),
            HEX_EDGE_SAMPLES - 1
        );
        assert_eq!(
            aligned_edge_index::<SquareRaster>(1, true),
            SQUARE_RASTER_SIZE - 2
        );
    }

    #[test]
    fn disjoint_sets_merge_transitively() {
        let mut sets = DisjointSets::new(4);
        sets.union(0, 1);
        sets.union(2, 3);
        assert_ne!(sets.find(0), sets.find(3));
        sets.union(1, 2);
        assert_eq!(sets.find(0), sets.find(3));
    }
}
