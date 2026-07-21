use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

use crate::{Direction, DirectionTransform, EdgeTransform};

/// Ordered samples along one tile edge.
///
/// Opposite edges must use the same screen-space sample order so compatible
/// facing strips can be compared directly. [`EdgeTransform`] supplies the
/// reversal needed to retain that order after rotations and reflections.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeStrip<T> {
    samples: Vec<T>,
}

impl<T> EdgeStrip<T> {
    pub const fn new(samples: Vec<T>) -> Self {
        Self { samples }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.samples
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.samples.iter()
    }

    pub fn into_vec(self) -> Vec<T> {
        self.samples
    }
}

impl<T: Clone> EdgeStrip<T> {
    pub fn reversed(&self) -> Self {
        Self::new(self.samples.iter().rev().cloned().collect())
    }
}

impl<T> From<Vec<T>> for EdgeStrip<T> {
    fn from(samples: Vec<T>) -> Self {
        Self::new(samples)
    }
}

/// One value for every member of a finite [`Direction`] set.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SocketMap<D, S> {
    sockets: Vec<S>,
    direction: PhantomData<fn(D) -> D>,
}

impl<D: Direction, S> SocketMap<D, S> {
    /// Constructs a complete map in [`Direction::ALL`] order.
    pub fn from_fn(mut make_socket: impl FnMut(D) -> S) -> Self {
        Self {
            sockets: D::ALL.iter().copied().map(&mut make_socket).collect(),
            direction: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.sockets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sockets.is_empty()
    }

    pub fn get(&self, direction: D) -> Option<&S> {
        self.sockets.get(direction.index())
    }

    pub fn get_mut(&mut self, direction: D) -> Option<&mut S> {
        self.sockets.get_mut(direction.index())
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (D, &S)> + '_ {
        D::ALL.iter().copied().zip(self.sockets.iter())
    }
}

impl<D: Direction, S> Index<D> for SocketMap<D, S> {
    type Output = S;

    fn index(&self, direction: D) -> &Self::Output {
        self.get(direction)
            .expect("Direction::index must refer to a member of Direction::ALL")
    }
}

impl<D: Direction, S> IndexMut<D> for SocketMap<D, S> {
    fn index_mut(&mut self, direction: D) -> &mut Self::Output {
        self.get_mut(direction)
            .expect("Direction::index must refer to a member of Direction::ALL")
    }
}

/// Application payload and edge sockets for one canonical tile.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tile<P, D, S> {
    pub payload: P,
    pub sockets: SocketMap<D, S>,
}

impl<P, D, S> Tile<P, D, S> {
    pub const fn new(payload: P, sockets: SocketMap<D, S>) -> Self {
        Self { payload, sockets }
    }
}

impl<P, D: Direction, S> Tile<P, D, S> {
    /// Looks up the socket presented in a world-space direction after transformation.
    pub fn oriented_socket<T: DirectionTransform<D>>(
        &self,
        transform: T,
        world_direction: D,
    ) -> &S {
        let local_direction = transform.inverse().apply_direction(world_direction);
        &self.sockets[local_direction]
    }
}

impl<P, D: Direction, S: Clone> Tile<P, D, EdgeStrip<S>> {
    /// Returns the edge presented in a world-space direction after transformation.
    ///
    /// The returned strip is normalized to the canonical sample order documented
    /// by [`EdgeStrip`], so facing strips can use direct equality matching.
    pub fn oriented_edge<T: EdgeTransform<D>>(
        &self,
        transform: T,
        world_direction: D,
    ) -> EdgeStrip<S> {
        let local_direction = transform.inverse().apply_direction(world_direction);
        let edge = &self.sockets[local_direction];
        if transform.reverses_edge(local_direction) {
            edge.reversed()
        } else {
            edge.clone()
        }
    }
}

/// A stable identifier for a tile within one [`TileSet`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileId(usize);

impl TileId {
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    pub const fn index(self) -> usize {
        self.0
    }
}

/// A base tile paired with an orientation without copying its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrientedTileId<T> {
    pub tile: TileId,
    pub transform: T,
}

impl<T> OrientedTileId<T> {
    pub const fn new(tile: TileId, transform: T) -> Self {
        Self { tile, transform }
    }
}

/// An append-only collection whose tile IDs remain stable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TileSet<P, D, S> {
    tiles: Vec<Tile<P, D, S>>,
}

impl<P, D, S> Default for TileSet<P, D, S> {
    fn default() -> Self {
        Self { tiles: Vec::new() }
    }
}

impl<P, D, S> TileSet<P, D, S> {
    pub const fn new() -> Self {
        Self { tiles: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            tiles: Vec::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, tile: Tile<P, D, S>) -> TileId {
        let id = TileId::new(self.tiles.len());
        self.tiles.push(tile);
        id
    }

    pub fn get(&self, id: TileId) -> Option<&Tile<P, D, S>> {
        self.tiles.get(id.index())
    }

    pub fn get_mut(&mut self, id: TileId) -> Option<&mut Tile<P, D, S>> {
        self.tiles.get_mut(id.index())
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (TileId, &Tile<P, D, S>)> + '_ {
        self.tiles
            .iter()
            .enumerate()
            .map(|(index, tile)| (TileId::new(index), tile))
    }
}

/// Decides whether two facing sockets may touch.
///
/// `direction` points from the first tile (the `outgoing` socket) toward the second tile
/// (the `incoming` socket, normally read at `direction.opposite()`).
pub trait SocketMatcher<D, S> {
    fn matches(&self, direction: D, outgoing: &S, incoming: &S) -> bool;
}

/// A socket matcher that accepts exactly equal values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EqualityMatcher;

impl<D, S: PartialEq> SocketMatcher<D, S> for EqualityMatcher {
    fn matches(&self, _direction: D, outgoing: &S, incoming: &S) -> bool {
        outgoing == incoming
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{D4, D6, HexDirection, QuarterTurns, SixthTurns, SocketMatcher, SquareDirection};

    fn labeled_tile() -> Tile<&'static str, SquareDirection, char> {
        Tile::new(
            "road",
            SocketMap::from_fn(|direction| match direction {
                SquareDirection::North => 'n',
                SquareDirection::East => 'e',
                SquareDirection::South => 's',
                SquareDirection::West => 'w',
            }),
        )
    }

    #[test]
    fn socket_map_is_complete_and_direction_indexed() {
        let sockets = labeled_tile().sockets;
        assert_eq!(sockets.len(), 4);
        assert_eq!(sockets[SquareDirection::North], 'n');
        assert_eq!(sockets[SquareDirection::West], 'w');
    }

    #[test]
    fn tile_ids_are_stable_and_dense() {
        let mut tiles = TileSet::new();
        let first = tiles.insert(labeled_tile());
        let second = tiles.insert(labeled_tile());
        assert_eq!(first.index(), 0);
        assert_eq!(second.index(), 1);
        assert_eq!(tiles.get(first).unwrap().payload, "road");
    }

    #[test]
    fn oriented_socket_uses_inverse_direction_mapping() {
        let tile = labeled_tile();
        let clockwise = D4::new(QuarterTurns::One, false);
        assert_eq!(tile.oriented_socket(clockwise, SquareDirection::East), &'n');
        let reflection = D4::new(QuarterTurns::Zero, true);
        assert_eq!(
            tile.oriented_socket(reflection, SquareDirection::West),
            &'e'
        );
    }

    #[test]
    fn oriented_socket_supports_hex_transforms() {
        let tile = Tile::new(
            "road",
            SocketMap::from_fn(|direction| match direction {
                HexDirection::NorthEast => 'a',
                HexDirection::East => 'b',
                HexDirection::SouthEast => 'c',
                HexDirection::SouthWest => 'd',
                HexDirection::West => 'e',
                HexDirection::NorthWest => 'f',
            }),
        );
        let clockwise = D6::new(SixthTurns::One, false);
        assert_eq!(tile.oriented_socket(clockwise, HexDirection::East), &'a');
        let reflection = D6::new(SixthTurns::Zero, true);
        assert_eq!(tile.oriented_socket(reflection, HexDirection::West), &'b');
    }

    #[test]
    fn equality_matcher_compares_values() {
        assert!(EqualityMatcher.matches(SquareDirection::East, &"road", &"road"));
        assert!(!EqualityMatcher.matches(SquareDirection::East, &"road", &"wall"));
    }

    #[test]
    fn edge_strips_preserve_and_reverse_sample_order() {
        let strip = EdgeStrip::new(vec!['a', 'b', 'c']);
        assert_eq!(strip.len(), 3);
        assert!(!strip.is_empty());
        assert_eq!(strip.as_slice(), &['a', 'b', 'c']);
        assert_eq!(
            strip.iter().copied().collect::<Vec<_>>(),
            vec!['a', 'b', 'c']
        );
        assert_eq!(strip.reversed().into_vec(), vec!['c', 'b', 'a']);
    }

    #[test]
    fn edge_strip_equality_rejects_one_changed_component() {
        let outgoing = EdgeStrip::new(vec![[10, 20, 30, 255], [40, 50, 60, 255]]);
        let incoming = EdgeStrip::new(vec![[10, 20, 30, 255], [40, 50, 60, 254]]);
        assert!(!EqualityMatcher.matches(SquareDirection::East, &outgoing, &incoming));
    }

    #[test]
    fn oriented_square_edges_map_direction_and_order() {
        let tile = Tile::new(
            (),
            SocketMap::from_fn(|direction| match direction {
                SquareDirection::North => EdgeStrip::new(vec!['n', 'N']),
                SquareDirection::East => EdgeStrip::new(vec!['e', 'E']),
                SquareDirection::South => EdgeStrip::new(vec!['s', 'S']),
                SquareDirection::West => EdgeStrip::new(vec!['w', 'W']),
            }),
        );
        let clockwise = D4::new(QuarterTurns::One, false);
        assert_eq!(
            tile.oriented_edge(clockwise, SquareDirection::East),
            EdgeStrip::new(vec!['n', 'N'])
        );
        assert_eq!(
            tile.oriented_edge(clockwise, SquareDirection::South),
            EdgeStrip::new(vec!['E', 'e'])
        );
    }

    #[test]
    fn oriented_hex_edges_support_all_d6_symmetries() {
        let tile = Tile::new(
            (),
            SocketMap::from_fn(|direction: HexDirection| {
                let label = direction.index() as u8;
                EdgeStrip::new(vec![label, label + 10, label + 20])
            }),
        );
        for transform in D6::ALL {
            for local_direction in HexDirection::ALL.iter().copied() {
                let world_direction = transform.apply_direction(local_direction);
                let mut expected = tile.sockets[local_direction].clone();
                if transform.reverses_edge(local_direction) {
                    expected = expected.reversed();
                }
                assert_eq!(
                    tile.oriented_edge(transform, world_direction),
                    expected,
                    "{transform:?} {local_direction:?}"
                );
            }
        }
    }

    #[test]
    fn custom_matcher_can_be_directional_and_asymmetric() {
        struct PlugToSocket;

        impl SocketMatcher<SquareDirection, &'static str> for PlugToSocket {
            fn matches(
                &self,
                direction: SquareDirection,
                outgoing: &&'static str,
                incoming: &&'static str,
            ) -> bool {
                direction == SquareDirection::East && *outgoing == "plug" && *incoming == "socket"
            }
        }

        assert!(PlugToSocket.matches(SquareDirection::East, &"plug", &"socket"));
        assert!(!PlugToSocket.matches(SquareDirection::West, &"plug", &"socket"));
        assert!(!PlugToSocket.matches(SquareDirection::East, &"socket", &"plug"));
    }
}
