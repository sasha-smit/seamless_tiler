use std::collections::HashMap;
use std::marker::PhantomData;

use seamless_tiler::{
    AxisBoundaries, CellId, Coord2, D4, D6, Direction, DirectionTransform, EdgeStrip,
    EqualityMatcher, Extent2, Grid, HexDirection, HexTopology, OrientedTileId, SocketMap,
    SocketMatcher, SquareDirection, SquareTopology, Tile, TileId, TileSet, Topology, Wfc, WfcRules,
    WfcStatus,
};

#[cfg(test)]
use crate::raster::{DEFAULT_PAINT_COLOR, closed_edge};
use crate::raster::{Raster, Rgba, generate_raster, is_closed_edge};

pub(crate) const DEFAULT_EXTENT: Extent2 = Extent2::new(12, 8);
pub(crate) const MAX_DIMENSION: usize = 64;
pub(crate) const DEFAULT_SEED: u64 = 1;
const DEFAULT_HEX_SEED: u64 = 3;
const NEW_TILE_COLOR: [u8; 3] = [120, 120, 120];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum GridMode {
    #[default]
    Square,
    Hex,
}

impl GridMode {
    pub(crate) const ALL: [Self; 2] = [Self::Square, Self::Hex];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Square => "Square",
            Self::Hex => "Hex",
        }
    }

    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TileStyle {
    pub(crate) name: String,
    pub(crate) color: [u8; 3],
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SquareTile {
    style: TileStyle,
    raster: Raster,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Orientation {
    Square(D4),
    Hex(D6),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct VariantView {
    pub(crate) tile: TileId,
    pub(crate) orientation: Orientation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum CanvasTool {
    #[default]
    Inspect,
    Pin(usize),
    Unpin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CellVisual {
    Unavailable,
    Contradiction,
    Superposition { candidates: usize, entropy: f64 },
    Collapsed { variant: usize, pinned: bool },
}

trait ModeSpec: 'static {
    type Direction: Direction;
    type Transform: DirectionTransform<Self::Direction> + Copy + Eq + std::hash::Hash;
    type Topology: Topology<Coord = Coord2, Direction = Self::Direction> + Copy;
    type Payload: Clone;
    type Socket: Clone + PartialEq;

    fn topology(extent: Extent2, boundaries: AxisBoundaries) -> Self::Topology;
    fn transforms() -> &'static [Self::Transform];
    fn orientation(transform: Self::Transform) -> Orientation;
    fn demo_tiles() -> TileSet<Self::Payload, Self::Direction, Self::Socket>;
    fn new_tile(name: String) -> Tile<Self::Payload, Self::Direction, Self::Socket>;
    fn style(payload: &Self::Payload) -> &TileStyle;
    fn style_mut(payload: &mut Self::Payload) -> &mut TileStyle;
    fn edge_controls(tile: &Tile<Self::Payload, Self::Direction, Self::Socket>) -> Vec<bool>;
    fn set_edge(
        tile: &mut Tile<Self::Payload, Self::Direction, Self::Socket>,
        direction: Self::Direction,
        value: bool,
    ) -> bool;
    fn set_color(
        tile: &mut Tile<Self::Payload, Self::Direction, Self::Socket>,
        color: [u8; 3],
    ) -> bool;
    fn oriented_socket(
        tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        transform: Self::Transform,
        world_direction: Self::Direction,
    ) -> Self::Socket;
    fn boundary_allows(socket: &Self::Socket) -> bool;
    fn variants_equal(
        tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        left: Self::Transform,
        right: Self::Transform,
    ) -> bool;
    fn variant_raster(
        tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        transform: Self::Transform,
    ) -> Option<Raster>;
    fn socket_active(socket: &Self::Socket) -> bool;
    fn default_seed() -> u64;
}

struct SquareMode;

impl ModeSpec for SquareMode {
    type Direction = SquareDirection;
    type Transform = D4;
    type Topology = SquareTopology;
    type Payload = SquareTile;
    type Socket = EdgeStrip<Rgba>;

    fn topology(extent: Extent2, boundaries: AxisBoundaries) -> Self::Topology {
        SquareTopology::new(extent, boundaries).expect("editor dimensions fit signed coordinates")
    }

    fn transforms() -> &'static [Self::Transform] {
        &D4::ALL
    }

    fn orientation(transform: Self::Transform) -> Orientation {
        Orientation::Square(transform)
    }

    fn demo_tiles() -> TileSet<Self::Payload, Self::Direction, Self::Socket> {
        let mut tiles = TileSet::with_capacity(5);
        insert_square_demo_tile(&mut tiles, "Blank", [72, 79, 89], |_| false);
        insert_square_demo_tile(&mut tiles, "Straight", [55, 118, 171], |direction| {
            matches!(direction, SquareDirection::North | SquareDirection::South)
        });
        insert_square_demo_tile(&mut tiles, "Corner", [46, 139, 87], |direction| {
            matches!(direction, SquareDirection::North | SquareDirection::East)
        });
        insert_square_demo_tile(&mut tiles, "T junction", [157, 112, 40], |direction| {
            direction != SquareDirection::South
        });
        insert_square_demo_tile(&mut tiles, "Cross", [135, 80, 156], |_| true);
        tiles
    }

    fn new_tile(name: String) -> Tile<Self::Payload, Self::Direction, Self::Socket> {
        square_tile(name, NEW_TILE_COLOR, [false; 4])
    }

    fn style(payload: &Self::Payload) -> &TileStyle {
        &payload.style
    }

    fn style_mut(payload: &mut Self::Payload) -> &mut TileStyle {
        &mut payload.style
    }

    fn edge_controls(tile: &Tile<Self::Payload, Self::Direction, Self::Socket>) -> Vec<bool> {
        tile.sockets
            .iter()
            .map(|(_, socket)| !is_closed_edge(socket))
            .collect()
    }

    fn set_edge(
        tile: &mut Tile<Self::Payload, Self::Direction, Self::Socket>,
        direction: Self::Direction,
        value: bool,
    ) -> bool {
        let _ = (tile, direction, value);
        false
    }

    fn set_color(
        tile: &mut Tile<Self::Payload, Self::Direction, Self::Socket>,
        color: [u8; 3],
    ) -> bool {
        if tile.payload.style.color == color {
            return false;
        }
        tile.payload.style.color = color;
        true
    }

    fn oriented_socket(
        tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        transform: Self::Transform,
        world_direction: Self::Direction,
    ) -> Self::Socket {
        tile.oriented_edge(transform, world_direction)
    }

    fn boundary_allows(socket: &Self::Socket) -> bool {
        is_closed_edge(socket)
    }

    fn variants_equal(
        tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        left: Self::Transform,
        right: Self::Transform,
    ) -> bool {
        tile.payload.raster.transformed(left) == tile.payload.raster.transformed(right)
    }

    fn variant_raster(
        tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        transform: Self::Transform,
    ) -> Option<Raster> {
        Some(tile.payload.raster.transformed(transform))
    }

    fn socket_active(socket: &Self::Socket) -> bool {
        !is_closed_edge(socket)
    }

    fn default_seed() -> u64 {
        DEFAULT_SEED
    }
}

struct HexMode;

impl ModeSpec for HexMode {
    type Direction = HexDirection;
    type Transform = D6;
    type Topology = HexTopology;
    type Payload = TileStyle;
    type Socket = bool;

    fn topology(extent: Extent2, boundaries: AxisBoundaries) -> Self::Topology {
        HexTopology::new(extent, boundaries).expect("editor dimensions fit signed coordinates")
    }

    fn transforms() -> &'static [Self::Transform] {
        &D6::ALL
    }

    fn orientation(transform: Self::Transform) -> Orientation {
        Orientation::Hex(transform)
    }

    fn demo_tiles() -> TileSet<Self::Payload, Self::Direction, Self::Socket> {
        let mut tiles = TileSet::with_capacity(5);
        insert_bool_demo_tile(&mut tiles, "Blank", [72, 79, 89], |_| false);
        insert_bool_demo_tile(&mut tiles, "Straight", [55, 118, 171], |direction| {
            matches!(direction, HexDirection::NorthEast | HexDirection::SouthWest)
        });
        insert_bool_demo_tile(&mut tiles, "Bend", [46, 139, 87], |direction| {
            matches!(direction, HexDirection::NorthEast | HexDirection::East)
        });
        insert_bool_demo_tile(&mut tiles, "Y junction", [157, 112, 40], |direction| {
            matches!(
                direction,
                HexDirection::NorthEast | HexDirection::SouthEast | HexDirection::West
            )
        });
        insert_bool_demo_tile(&mut tiles, "Hub", [135, 80, 156], |_| true);
        tiles
    }

    fn new_tile(name: String) -> Tile<Self::Payload, Self::Direction, Self::Socket> {
        Tile::new(
            TileStyle {
                name,
                color: NEW_TILE_COLOR,
            },
            SocketMap::from_fn(|_| false),
        )
    }

    fn style(payload: &Self::Payload) -> &TileStyle {
        payload
    }

    fn style_mut(payload: &mut Self::Payload) -> &mut TileStyle {
        payload
    }

    fn edge_controls(tile: &Tile<Self::Payload, Self::Direction, Self::Socket>) -> Vec<bool> {
        tile.sockets.iter().map(|(_, socket)| *socket).collect()
    }

    fn set_edge(
        tile: &mut Tile<Self::Payload, Self::Direction, Self::Socket>,
        direction: Self::Direction,
        value: bool,
    ) -> bool {
        let socket = &mut tile.sockets[direction];
        if *socket == value {
            return false;
        }
        *socket = value;
        true
    }

    fn set_color(
        tile: &mut Tile<Self::Payload, Self::Direction, Self::Socket>,
        color: [u8; 3],
    ) -> bool {
        if tile.payload.color == color {
            return false;
        }
        tile.payload.color = color;
        true
    }

    fn oriented_socket(
        tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        transform: Self::Transform,
        world_direction: Self::Direction,
    ) -> Self::Socket {
        *tile.oriented_socket(transform, world_direction)
    }

    fn boundary_allows(socket: &Self::Socket) -> bool {
        !socket
    }

    fn variants_equal(
        tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        left: Self::Transform,
        right: Self::Transform,
    ) -> bool {
        Self::Direction::ALL.iter().copied().all(|direction| {
            Self::oriented_socket(tile, left, direction)
                == Self::oriented_socket(tile, right, direction)
        })
    }

    fn variant_raster(
        _tile: &Tile<Self::Payload, Self::Direction, Self::Socket>,
        _transform: Self::Transform,
    ) -> Option<Raster> {
        None
    }

    fn socket_active(socket: &Self::Socket) -> bool {
        *socket
    }

    fn default_seed() -> u64 {
        DEFAULT_HEX_SEED
    }
}

struct Session<M: ModeSpec> {
    pins: Grid<Option<usize>>,
    topology: M::Topology,
    boundaries: AxisBoundaries,
    tiles: TileSet<M::Payload, M::Direction, M::Socket>,
    variants: Vec<OrientedTileId<M::Transform>>,
    variant_sockets: Vec<Vec<M::Socket>>,
    variant_rasters: Vec<Option<Raster>>,
    enabled: Vec<bool>,
    pattern_variants: Vec<usize>,
    wave: Option<Wfc<M::Topology>>,
    seed: u64,
    selected_cell: Option<Coord2>,
    selected_tile: Option<TileId>,
    tool: CanvasTool,
    running: bool,
    observations: usize,
    last_observed: Option<Coord2>,
    version: u64,
    mode: PhantomData<M>,
}

impl<M: ModeSpec> Session<M> {
    fn new(extent: Extent2) -> Self {
        assert!(
            EditorModel::valid_extent(extent),
            "editor extent must be between 1 and 64"
        );
        let tiles = M::demo_tiles();
        let selected_tile = (!tiles.is_empty()).then_some(TileId::new(0));
        let derived = distinct_variants::<M>(&tiles);
        let mut session = Self {
            pins: Grid::filled(extent, None).expect("editor dimensions have a valid area"),
            topology: M::topology(extent, AxisBoundaries::BOUNDED),
            boundaries: AxisBoundaries::BOUNDED,
            enabled: vec![true; derived.variants.len()],
            tiles,
            variants: derived.variants,
            variant_sockets: derived.sockets,
            variant_rasters: derived.rasters,
            pattern_variants: Vec::new(),
            wave: None,
            seed: M::default_seed(),
            selected_cell: None,
            selected_tile,
            tool: CanvasTool::Inspect,
            running: false,
            observations: 0,
            last_observed: None,
            version: 0,
            mode: PhantomData,
        };
        session.rebuild_wave();
        session
    }

    fn extent(&self) -> Extent2 {
        self.pins.extent()
    }

    fn resize(&mut self, extent: Extent2) -> bool {
        if !EditorModel::valid_extent(extent) {
            return false;
        }
        let mut resized = Grid::filled(extent, None).expect("editor dimensions have a valid area");
        for (coord, pin) in self.pins.cells() {
            if let Some(target) = resized.get_mut(coord) {
                *target = *pin;
            }
        }
        self.pins = resized;
        self.topology = M::topology(extent, self.boundaries);
        if self
            .selected_cell
            .is_some_and(|coord| !extent.contains(coord))
        {
            self.selected_cell = None;
        }
        self.rebuild_wave();
        true
    }

    fn set_boundaries(&mut self, boundaries: AxisBoundaries) {
        if boundaries == self.boundaries {
            return;
        }
        self.boundaries = boundaries;
        self.topology = M::topology(self.extent(), boundaries);
        self.rebuild_wave();
    }

    fn set_seed(&mut self, seed: u64) {
        if seed != self.seed {
            self.seed = seed;
            self.rebuild_wave();
        }
    }

    fn set_tool(&mut self, tool: CanvasTool) {
        if let CanvasTool::Pin(index) = tool
            && !self.enabled.get(index).copied().unwrap_or(false)
        {
            return;
        }
        self.tool = tool;
    }

    fn set_variant_enabled(&mut self, index: usize, enabled: bool) -> usize {
        let Some(current) = self.enabled.get_mut(index) else {
            return 0;
        };
        if *current == enabled {
            return 0;
        }
        *current = enabled;
        let mut cleared = 0;
        if !enabled {
            for pin in &mut self.pins {
                if *pin == Some(index) {
                    *pin = None;
                    cleared += 1;
                }
            }
            if self.tool == CanvasTool::Pin(index) {
                self.tool = CanvasTool::Inspect;
            }
        }
        self.rebuild_wave();
        cleared
    }

    fn add_tile(&mut self) {
        let name = format!("Tile {}", self.tiles.len() + 1);
        self.selected_tile = Some(self.tiles.insert(M::new_tile(name)));
        self.refresh_catalog(Some);
    }

    fn remove_tile(&mut self, tile: TileId) {
        if self.tiles.get(tile).is_none() {
            return;
        }
        let removed = tile.index();
        let mut rebuilt = TileSet::with_capacity(self.tiles.len().saturating_sub(1));
        for (id, value) in self.tiles.iter() {
            if id != tile {
                rebuilt.insert(value.clone());
            }
        }
        self.tiles = rebuilt;
        self.selected_tile = match self.selected_tile {
            Some(selected) if selected.index() < removed => Some(selected),
            Some(selected) if selected.index() > removed => Some(TileId::new(selected.index() - 1)),
            Some(_) if self.tiles.is_empty() => None,
            Some(_) => Some(TileId::new(removed.min(self.tiles.len() - 1))),
            None => None,
        };
        self.refresh_catalog(|old| {
            let index = old.index();
            match index.cmp(&removed) {
                std::cmp::Ordering::Less => Some(old),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(TileId::new(index - 1)),
            }
        });
    }

    fn set_tile_name(&mut self, tile: TileId, name: String) {
        if let Some(value) = self.tiles.get_mut(tile) {
            M::style_mut(&mut value.payload).name = name;
        }
    }

    fn set_selected_tile(&mut self, tile: TileId) -> bool {
        if self.tiles.get(tile).is_none() || self.selected_tile == Some(tile) {
            return false;
        }
        self.selected_tile = Some(tile);
        true
    }

    fn set_tile_color(&mut self, tile: TileId, color: [u8; 3]) {
        let Some(value) = self.tiles.get_mut(tile) else {
            return;
        };
        if M::set_color(value, color) {
            self.refresh_variant_rasters();
            self.version = self.version.wrapping_add(1);
        }
    }

    fn set_tile_socket(&mut self, tile: TileId, direction_index: usize, value: bool) {
        let Some(direction) = M::Direction::ALL.get(direction_index).copied() else {
            return;
        };
        let Some(entry) = self.tiles.get_mut(tile) else {
            return;
        };
        if !M::set_edge(entry, direction, value) {
            return;
        }
        self.refresh_catalog(Some);
    }

    /// Re-derives variants after a catalog edit and reconciles enable/disable
    /// toggles, pins, and the pin tool by `(TileId, transform)` identity.
    ///
    /// `remap_tile` translates old tile IDs to their post-edit IDs (`None` if the
    /// tile was removed); it is `Some` (identity) for edits that do not renumber.
    fn refresh_catalog(&mut self, remap_tile: impl Fn(TileId) -> Option<TileId>) {
        let old_variants = std::mem::take(&mut self.variants);
        let old_enabled = std::mem::take(&mut self.enabled);

        let derived = distinct_variants::<M>(&self.tiles);
        let variants = derived.variants;

        let new_index_of: HashMap<OrientedTileId<M::Transform>, usize> = variants
            .iter()
            .enumerate()
            .map(|(index, variant)| (*variant, index))
            .collect();

        let old_to_new: Vec<Option<usize>> = old_variants
            .iter()
            .map(|variant| {
                remap_tile(variant.tile).and_then(|tile| {
                    new_index_of
                        .get(&OrientedTileId::new(tile, variant.transform))
                        .copied()
                })
            })
            .collect();

        let mut enabled = vec![true; variants.len()];
        for (old_index, maybe_new) in old_to_new.iter().enumerate() {
            if let Some(new_index) = maybe_new {
                enabled[*new_index] = old_enabled[old_index];
            }
        }

        for pin in self.pins.iter_mut() {
            if let Some(old_index) = *pin {
                *pin = old_to_new.get(old_index).copied().flatten();
            }
        }

        if let CanvasTool::Pin(old_index) = self.tool {
            self.tool = match old_to_new.get(old_index).copied().flatten() {
                Some(new_index) if enabled[new_index] => CanvasTool::Pin(new_index),
                _ => CanvasTool::Inspect,
            };
        }

        self.variants = variants;
        self.variant_sockets = derived.sockets;
        self.variant_rasters = derived.rasters;
        self.enabled = enabled;
        self.version = self.version.wrapping_add(1);
        self.rebuild_wave();
    }

    fn tile_sockets(&self, tile: TileId) -> Vec<bool> {
        self.tiles
            .get(tile)
            .map(M::edge_controls)
            .unwrap_or_default()
    }

    fn refresh_variant_rasters(&mut self) {
        self.variant_rasters = self
            .variants
            .iter()
            .map(|variant| {
                self.tiles
                    .get(variant.tile)
                    .and_then(|tile| M::variant_raster(tile, variant.transform))
            })
            .collect();
    }

    fn variant_raster(&self, index: usize) -> Option<&Raster> {
        self.variant_rasters.get(index).and_then(Option::as_ref)
    }

    fn variant_count(&self) -> usize {
        self.variants.len()
    }

    fn catalog_version(&self) -> u64 {
        self.version
    }

    fn apply_tool(&mut self, coord: Coord2, secondary: bool) -> bool {
        if !self.extent().contains(coord) {
            return false;
        }
        self.selected_cell = Some(coord);
        let action = if secondary {
            CanvasTool::Unpin
        } else {
            self.tool
        };
        match action {
            CanvasTool::Inspect => false,
            CanvasTool::Pin(index) => self.set_pin(coord, Some(index)),
            CanvasTool::Unpin => self.set_pin(coord, None),
        }
    }

    fn set_pin(&mut self, coord: Coord2, variant: Option<usize>) -> bool {
        let Some(pin) = self.pins.get_mut(coord) else {
            return false;
        };
        if *pin == variant {
            return false;
        }
        *pin = variant;
        self.rebuild_wave();
        true
    }

    fn clear_pins(&mut self) -> usize {
        let mut cleared = 0;
        for pin in &mut self.pins {
            if pin.take().is_some() {
                cleared += 1;
            }
        }
        if cleared > 0 {
            self.rebuild_wave();
        }
        cleared
    }

    fn reset_wave(&mut self) {
        if let Some(wave) = &mut self.wave {
            wave.restart(self.seed);
        }
        self.running = false;
        self.observations = 0;
        self.last_observed = None;
    }

    fn retry(&mut self) -> bool {
        if self.initial_contradiction()
            || !matches!(self.status(), Some(WfcStatus::Contradiction { .. }))
        {
            return false;
        }
        self.seed = self.seed.wrapping_add(1);
        self.rebuild_wave();
        true
    }

    fn step(&mut self) -> bool {
        let Some(wave) = &mut self.wave else {
            self.running = false;
            return false;
        };
        let Some(step) = wave.step() else {
            self.running = false;
            return false;
        };
        self.observations += 1;
        self.last_observed = self.topology.coordinate(step.cell);
        if step.status != WfcStatus::Running {
            self.running = false;
        }
        true
    }

    fn finish(&mut self) {
        self.running = false;
        while self.step() {}
    }

    fn toggle_running(&mut self) {
        if matches!(self.status(), Some(WfcStatus::Running)) {
            self.running = !self.running;
        } else {
            self.running = false;
        }
    }

    fn status(&self) -> Option<WfcStatus> {
        self.wave.as_ref().map(Wfc::status)
    }

    fn initial_contradiction(&self) -> bool {
        self.observations == 0 && matches!(self.status(), Some(WfcStatus::Contradiction { .. }))
    }

    fn unresolved_count(&self) -> usize {
        let Some(wave) = &self.wave else {
            return 0;
        };
        (0..self.topology.cell_count())
            .filter(|index| {
                wave.candidate_count(CellId::new(*index))
                    .is_some_and(|candidates| candidates > 1)
            })
            .count()
    }

    fn cell_visual(&self, coord: Coord2) -> CellVisual {
        let Some(cell) = self.topology.cell_at(coord) else {
            return CellVisual::Unavailable;
        };
        let Some(wave) = &self.wave else {
            return CellVisual::Unavailable;
        };
        match wave.candidate_count(cell) {
            Some(0) => CellVisual::Contradiction,
            Some(1) => {
                let pattern = wave
                    .collapsed_pattern(cell)
                    .expect("a singleton domain has a pattern");
                CellVisual::Collapsed {
                    variant: self.pattern_variants[pattern.index()],
                    pinned: self.pins[coord].is_some(),
                }
            }
            Some(candidates) => CellVisual::Superposition {
                candidates,
                entropy: wave.entropy(cell).expect("a non-empty domain has entropy"),
            },
            None => CellVisual::Unavailable,
        }
    }

    fn candidate_variants(&self, coord: Coord2) -> Vec<usize> {
        let Some(cell) = self.topology.cell_at(coord) else {
            return Vec::new();
        };
        let Some(wave) = &self.wave else {
            return Vec::new();
        };
        wave.candidates(cell)
            .into_iter()
            .flatten()
            .map(|pattern| self.pattern_variants[pattern.index()])
            .collect()
    }

    fn rebuild_wave(&mut self) {
        self.running = false;
        self.observations = 0;
        self.last_observed = None;
        self.pattern_variants = self
            .enabled
            .iter()
            .enumerate()
            .filter_map(|(index, enabled)| enabled.then_some(index))
            .collect();
        if self.pattern_variants.is_empty() {
            self.wave = None;
            return;
        }

        let mut enabled_per_tile = vec![0_usize; self.tiles.len()];
        for variant_index in &self.pattern_variants {
            enabled_per_tile[self.variants[*variant_index].tile.index()] += 1;
        }
        let weights = self.pattern_variants.iter().map(|variant_index| {
            let tile = self.variants[*variant_index].tile;
            1.0 / enabled_per_tile[tile.index()] as f64
        });
        let rules = WfcRules::new(weights, |direction: M::Direction, source, neighbor| {
            let source = self.pattern_variants[source.index()];
            let neighbor = self.pattern_variants[neighbor.index()];
            EqualityMatcher.matches(
                direction,
                &self.variant_sockets[source][direction.index()],
                &self.variant_sockets[neighbor][direction.opposite().index()],
            )
        })
        .expect("enabled catalog patterns have valid weights");

        let topology = self.topology;
        let wave = Wfc::with_constraints(topology, rules, self.seed, |cell, pattern| {
            let variant_index = self.pattern_variants[pattern.index()];
            let pin_matches = topology
                .coordinate(cell)
                .and_then(|coord| self.pins.get(coord))
                .is_none_or(|pin| pin.is_none_or(|pin| pin == variant_index));
            pin_matches
                && M::Direction::ALL.iter().copied().all(|direction| {
                    topology.neighbor(cell, direction).is_some()
                        || M::boundary_allows(
                            &self.variant_sockets[variant_index][direction.index()],
                        )
                })
        });
        self.wave = Some(wave);
    }
}

struct DerivedVariants<M: ModeSpec> {
    variants: Vec<OrientedTileId<M::Transform>>,
    sockets: Vec<Vec<M::Socket>>,
    rasters: Vec<Option<Raster>>,
}

fn distinct_variants<M: ModeSpec>(
    tiles: &TileSet<M::Payload, M::Direction, M::Socket>,
) -> DerivedVariants<M> {
    let mut variants = Vec::new();
    let mut sockets = Vec::new();
    let mut rasters = Vec::new();
    for (tile_id, tile) in tiles.iter() {
        let mut representatives = Vec::new();
        for transform in M::transforms().iter().copied() {
            if representatives
                .iter()
                .copied()
                .any(|representative| M::variants_equal(tile, representative, transform))
            {
                continue;
            }
            representatives.push(transform);
            let signature = M::Direction::ALL
                .iter()
                .copied()
                .map(|direction| M::oriented_socket(tile, transform, direction))
                .collect();
            variants.push(OrientedTileId::new(tile_id, transform));
            sockets.push(signature);
            rasters.push(M::variant_raster(tile, transform));
        }
    }
    DerivedVariants {
        variants,
        sockets,
        rasters,
    }
}

fn insert_bool_demo_tile<D: Direction>(
    tiles: &mut TileSet<TileStyle, D, bool>,
    name: &str,
    color: [u8; 3],
    socket: impl FnMut(D) -> bool,
) -> TileId {
    tiles.insert(Tile::new(
        TileStyle {
            name: name.to_owned(),
            color,
        },
        SocketMap::from_fn(socket),
    ))
}

fn square_tile(
    name: String,
    color: [u8; 3],
    edge_mask: [bool; 4],
) -> Tile<SquareTile, SquareDirection, EdgeStrip<Rgba>> {
    let raster = generate_raster(&edge_mask, color);
    let sockets = raster.edges();
    Tile::new(
        SquareTile {
            style: TileStyle { name, color },
            raster,
        },
        sockets,
    )
}

impl Session<SquareMode> {
    fn selected_raster(&self) -> Option<&Raster> {
        let tile = self.selected_tile?;
        self.tiles.get(tile).map(|tile| &tile.payload.raster)
    }

    fn paint_selected_tile(
        &mut self,
        from: Coord2,
        to: Coord2,
        brush_size: usize,
        color: Rgba,
    ) -> bool {
        let Some(tile_id) = self.selected_tile else {
            return false;
        };
        let Some(tile) = self.tiles.get_mut(tile_id) else {
            return false;
        };
        if !tile
            .payload
            .raster
            .paint_stroke(from, to, brush_size, color)
        {
            return false;
        }
        tile.sockets = tile.payload.raster.edges();
        self.refresh_catalog(Some);
        true
    }
}

fn insert_square_demo_tile(
    tiles: &mut TileSet<SquareTile, SquareDirection, EdgeStrip<Rgba>>,
    name: &str,
    color: [u8; 3],
    mut socket: impl FnMut(SquareDirection) -> bool,
) -> TileId {
    let mut edge_mask = [false; 4];
    for direction in SquareDirection::ALL.iter().copied() {
        edge_mask[direction.index()] = socket(direction);
    }
    tiles.insert(square_tile(name.to_owned(), color, edge_mask))
}

trait SessionAccess {
    fn extent(&self) -> Extent2;
    fn boundaries(&self) -> AxisBoundaries;
    fn tiles(&self) -> Vec<(TileId, TileStyle)>;
    fn tile_style(&self, tile: TileId) -> Option<TileStyle>;
    fn variant(&self, index: usize) -> Option<VariantView>;
    fn variant_sockets(&self, index: usize) -> Vec<bool>;
    fn variant_raster(&self, index: usize) -> Option<&Raster>;
    fn variant_enabled(&self, index: usize) -> bool;
    fn tile_sockets(&self, tile: TileId) -> Vec<bool>;
    fn variant_count(&self) -> usize;
    fn catalog_version(&self) -> u64;
    fn variants_for_tile(&self, tile: TileId) -> Vec<usize>;
    fn enabled_variant_count(&self) -> usize;
    fn seed(&self) -> u64;
    fn selected_tile(&self) -> Option<TileId>;
    fn selected_cell(&self) -> Option<Coord2>;
    fn tool(&self) -> CanvasTool;
    fn running(&self) -> bool;
    fn observations(&self) -> usize;
    fn last_observed(&self) -> Option<Coord2>;
    fn status(&self) -> Option<WfcStatus>;
    fn initial_contradiction(&self) -> bool;
    fn unresolved_count(&self) -> usize;
    fn pin_variant_at(&self, coord: Coord2) -> Option<usize>;
    fn cell_visual(&self, coord: Coord2) -> CellVisual;
    fn candidate_variants(&self, coord: Coord2) -> Vec<usize>;
    fn cell_at(&self, coord: Coord2) -> Option<CellId>;
    fn coordinate(&self, cell: CellId) -> Option<Coord2>;
    fn cell_count(&self) -> usize;
    fn resize(&mut self, extent: Extent2) -> bool;
    fn set_boundaries(&mut self, boundaries: AxisBoundaries);
    fn set_seed(&mut self, seed: u64);
    fn set_tool(&mut self, tool: CanvasTool);
    fn set_variant_enabled(&mut self, index: usize, enabled: bool) -> usize;
    fn add_tile(&mut self);
    fn remove_tile(&mut self, tile: TileId);
    fn set_tile_name(&mut self, tile: TileId, name: String);
    fn set_tile_color(&mut self, tile: TileId, color: [u8; 3]);
    fn set_tile_socket(&mut self, tile: TileId, direction_index: usize, value: bool);
    fn set_selected_tile(&mut self, tile: TileId) -> bool;
    fn apply_tool(&mut self, coord: Coord2, secondary: bool) -> bool;
    fn clear_pins(&mut self) -> usize;
    fn reset_wave(&mut self);
    fn retry(&mut self) -> bool;
    fn step(&mut self) -> bool;
    fn finish(&mut self);
    fn toggle_running(&mut self);
}

impl<M: ModeSpec> SessionAccess for Session<M> {
    fn extent(&self) -> Extent2 {
        self.extent()
    }
    fn boundaries(&self) -> AxisBoundaries {
        self.boundaries
    }
    fn tiles(&self) -> Vec<(TileId, TileStyle)> {
        self.tiles
            .iter()
            .map(|(tile, value)| (tile, M::style(&value.payload).clone()))
            .collect()
    }
    fn tile_style(&self, tile: TileId) -> Option<TileStyle> {
        self.tiles
            .get(tile)
            .map(|value| M::style(&value.payload).clone())
    }
    fn variant(&self, index: usize) -> Option<VariantView> {
        let placement = *self.variants.get(index)?;
        Some(VariantView {
            tile: placement.tile,
            orientation: M::orientation(placement.transform),
        })
    }
    fn variant_sockets(&self, index: usize) -> Vec<bool> {
        self.variant_sockets
            .get(index)
            .map(|sockets| sockets.iter().map(M::socket_active).collect())
            .unwrap_or_default()
    }
    fn variant_raster(&self, index: usize) -> Option<&Raster> {
        self.variant_raster(index)
    }
    fn variant_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }
    fn tile_sockets(&self, tile: TileId) -> Vec<bool> {
        self.tile_sockets(tile)
    }
    fn variant_count(&self) -> usize {
        self.variant_count()
    }
    fn catalog_version(&self) -> u64 {
        self.catalog_version()
    }
    fn variants_for_tile(&self, tile: TileId) -> Vec<usize> {
        self.variants
            .iter()
            .enumerate()
            .filter_map(|(index, variant)| (variant.tile == tile).then_some(index))
            .collect()
    }
    fn enabled_variant_count(&self) -> usize {
        self.pattern_variants.len()
    }
    fn seed(&self) -> u64 {
        self.seed
    }
    fn selected_tile(&self) -> Option<TileId> {
        self.selected_tile
    }
    fn selected_cell(&self) -> Option<Coord2> {
        self.selected_cell
    }
    fn tool(&self) -> CanvasTool {
        self.tool
    }
    fn running(&self) -> bool {
        self.running
    }
    fn observations(&self) -> usize {
        self.observations
    }
    fn last_observed(&self) -> Option<Coord2> {
        self.last_observed
    }
    fn status(&self) -> Option<WfcStatus> {
        self.status()
    }
    fn initial_contradiction(&self) -> bool {
        self.initial_contradiction()
    }
    fn unresolved_count(&self) -> usize {
        self.unresolved_count()
    }
    fn pin_variant_at(&self, coord: Coord2) -> Option<usize> {
        self.pins.get(coord).copied().flatten()
    }
    fn cell_visual(&self, coord: Coord2) -> CellVisual {
        self.cell_visual(coord)
    }
    fn candidate_variants(&self, coord: Coord2) -> Vec<usize> {
        self.candidate_variants(coord)
    }
    fn cell_at(&self, coord: Coord2) -> Option<CellId> {
        self.topology.cell_at(coord)
    }
    fn coordinate(&self, cell: CellId) -> Option<Coord2> {
        self.topology.coordinate(cell)
    }
    fn cell_count(&self) -> usize {
        self.topology.cell_count()
    }
    fn resize(&mut self, extent: Extent2) -> bool {
        self.resize(extent)
    }
    fn set_boundaries(&mut self, boundaries: AxisBoundaries) {
        self.set_boundaries(boundaries);
    }
    fn set_seed(&mut self, seed: u64) {
        self.set_seed(seed);
    }
    fn set_tool(&mut self, tool: CanvasTool) {
        self.set_tool(tool);
    }
    fn set_variant_enabled(&mut self, index: usize, enabled: bool) -> usize {
        self.set_variant_enabled(index, enabled)
    }
    fn add_tile(&mut self) {
        self.add_tile();
    }
    fn remove_tile(&mut self, tile: TileId) {
        self.remove_tile(tile);
    }
    fn set_tile_name(&mut self, tile: TileId, name: String) {
        self.set_tile_name(tile, name);
    }
    fn set_tile_color(&mut self, tile: TileId, color: [u8; 3]) {
        self.set_tile_color(tile, color);
    }
    fn set_tile_socket(&mut self, tile: TileId, direction_index: usize, value: bool) {
        self.set_tile_socket(tile, direction_index, value);
    }
    fn set_selected_tile(&mut self, tile: TileId) -> bool {
        self.set_selected_tile(tile)
    }
    fn apply_tool(&mut self, coord: Coord2, secondary: bool) -> bool {
        self.apply_tool(coord, secondary)
    }
    fn clear_pins(&mut self) -> usize {
        self.clear_pins()
    }
    fn reset_wave(&mut self) {
        self.reset_wave();
    }
    fn retry(&mut self) -> bool {
        self.retry()
    }
    fn step(&mut self) -> bool {
        self.step()
    }
    fn finish(&mut self) {
        self.finish();
    }
    fn toggle_running(&mut self) {
        self.toggle_running();
    }
}

pub(crate) struct EditorModel {
    mode: GridMode,
    square: Session<SquareMode>,
    hex: Session<HexMode>,
}

impl Default for EditorModel {
    fn default() -> Self {
        Self::new(DEFAULT_EXTENT)
    }
}

impl EditorModel {
    pub(crate) fn new(extent: Extent2) -> Self {
        Self {
            mode: GridMode::Square,
            square: Session::new(extent),
            hex: Session::new(extent),
        }
    }

    fn active(&self) -> &dyn SessionAccess {
        match self.mode {
            GridMode::Square => &self.square,
            GridMode::Hex => &self.hex,
        }
    }

    fn active_mut(&mut self) -> &mut dyn SessionAccess {
        match self.mode {
            GridMode::Square => &mut self.square,
            GridMode::Hex => &mut self.hex,
        }
    }

    pub(crate) fn valid_extent(extent: Extent2) -> bool {
        (1..=MAX_DIMENSION).contains(&extent.width) && (1..=MAX_DIMENSION).contains(&extent.height)
    }

    pub(crate) const fn mode(&self) -> GridMode {
        self.mode
    }

    pub(crate) fn set_mode(&mut self, mode: GridMode) -> bool {
        if mode == self.mode {
            return false;
        }
        self.mode = mode;
        true
    }

    pub(crate) fn extent(&self) -> Extent2 {
        self.active().extent()
    }
    pub(crate) fn boundaries(&self) -> AxisBoundaries {
        self.active().boundaries()
    }
    pub(crate) fn tiles(&self) -> Vec<(TileId, TileStyle)> {
        self.active().tiles()
    }
    pub(crate) fn tile_style(&self, tile: TileId) -> Option<TileStyle> {
        self.active().tile_style(tile)
    }
    pub(crate) fn variant(&self, index: usize) -> Option<VariantView> {
        self.active().variant(index)
    }
    pub(crate) fn variant_sockets(&self, index: usize) -> Vec<bool> {
        self.active().variant_sockets(index)
    }
    pub(crate) fn variant_raster(&self, index: usize) -> Option<&Raster> {
        self.active().variant_raster(index)
    }
    pub(crate) fn variant_enabled(&self, index: usize) -> bool {
        self.active().variant_enabled(index)
    }
    pub(crate) fn tile_sockets(&self, tile: TileId) -> Vec<bool> {
        self.active().tile_sockets(tile)
    }
    pub(crate) fn variant_count(&self) -> usize {
        self.active().variant_count()
    }
    pub(crate) fn catalog_version(&self) -> u64 {
        self.active().catalog_version()
    }
    pub(crate) fn variants_for_tile(&self, tile: TileId) -> Vec<usize> {
        self.active().variants_for_tile(tile)
    }
    pub(crate) fn enabled_variant_count(&self) -> usize {
        self.active().enabled_variant_count()
    }
    pub(crate) fn seed(&self) -> u64 {
        self.active().seed()
    }
    pub(crate) fn selected_tile(&self) -> Option<TileId> {
        self.active().selected_tile()
    }
    pub(crate) fn selected_raster(&self) -> Option<&Raster> {
        (self.mode == GridMode::Square)
            .then(|| self.square.selected_raster())
            .flatten()
    }
    pub(crate) fn selected_cell(&self) -> Option<Coord2> {
        self.active().selected_cell()
    }
    pub(crate) fn tool(&self) -> CanvasTool {
        self.active().tool()
    }
    pub(crate) fn running(&self) -> bool {
        self.active().running()
    }
    pub(crate) fn observations(&self) -> usize {
        self.active().observations()
    }
    pub(crate) fn last_observed(&self) -> Option<Coord2> {
        self.active().last_observed()
    }
    pub(crate) fn status(&self) -> Option<WfcStatus> {
        self.active().status()
    }
    pub(crate) fn initial_contradiction(&self) -> bool {
        self.active().initial_contradiction()
    }
    pub(crate) fn unresolved_count(&self) -> usize {
        self.active().unresolved_count()
    }
    pub(crate) fn pin_variant_at(&self, coord: Coord2) -> Option<usize> {
        self.active().pin_variant_at(coord)
    }
    pub(crate) fn cell_visual(&self, coord: Coord2) -> CellVisual {
        self.active().cell_visual(coord)
    }
    pub(crate) fn candidate_variants(&self, coord: Coord2) -> Vec<usize> {
        self.active().candidate_variants(coord)
    }
    pub(crate) fn cell_at(&self, coord: Coord2) -> Option<CellId> {
        self.active().cell_at(coord)
    }
    pub(crate) fn coordinate(&self, cell: CellId) -> Option<Coord2> {
        self.active().coordinate(cell)
    }
    pub(crate) fn cell_count(&self) -> usize {
        self.active().cell_count()
    }
    pub(crate) fn resize(&mut self, extent: Extent2) -> bool {
        self.active_mut().resize(extent)
    }
    pub(crate) fn set_boundaries(&mut self, boundaries: AxisBoundaries) {
        self.active_mut().set_boundaries(boundaries);
    }
    pub(crate) fn set_seed(&mut self, seed: u64) {
        self.active_mut().set_seed(seed);
    }
    pub(crate) fn set_tool(&mut self, tool: CanvasTool) {
        self.active_mut().set_tool(tool);
    }
    pub(crate) fn set_variant_enabled(&mut self, index: usize, enabled: bool) -> usize {
        self.active_mut().set_variant_enabled(index, enabled)
    }
    pub(crate) fn add_tile(&mut self) {
        self.active_mut().add_tile();
    }
    pub(crate) fn remove_tile(&mut self, tile: TileId) {
        self.active_mut().remove_tile(tile);
    }
    pub(crate) fn set_tile_name(&mut self, tile: TileId, name: String) {
        self.active_mut().set_tile_name(tile, name);
    }
    pub(crate) fn set_tile_color(&mut self, tile: TileId, color: [u8; 3]) {
        self.active_mut().set_tile_color(tile, color);
    }
    pub(crate) fn set_tile_socket(&mut self, tile: TileId, direction_index: usize, value: bool) {
        self.active_mut()
            .set_tile_socket(tile, direction_index, value);
    }
    pub(crate) fn set_selected_tile(&mut self, tile: TileId) -> bool {
        self.active_mut().set_selected_tile(tile)
    }
    pub(crate) fn paint_selected_tile(
        &mut self,
        from: Coord2,
        to: Coord2,
        brush_size: usize,
        color: Rgba,
    ) -> bool {
        if self.mode != GridMode::Square {
            return false;
        }
        self.square.paint_selected_tile(from, to, brush_size, color)
    }
    pub(crate) fn apply_tool(&mut self, coord: Coord2, secondary: bool) -> bool {
        self.active_mut().apply_tool(coord, secondary)
    }
    pub(crate) fn clear_pins(&mut self) -> usize {
        self.active_mut().clear_pins()
    }
    pub(crate) fn reset_wave(&mut self) {
        self.active_mut().reset_wave();
    }
    pub(crate) fn retry(&mut self) -> bool {
        self.active_mut().retry()
    }
    pub(crate) fn step(&mut self) -> bool {
        self.active_mut().step()
    }
    pub(crate) fn finish(&mut self) {
        self.active_mut().finish();
    }
    pub(crate) fn toggle_running(&mut self) {
        self.active_mut().toggle_running();
    }

    pub(crate) fn reset_active(&mut self) {
        match self.mode {
            GridMode::Square => self.square = Session::new(DEFAULT_EXTENT),
            GridMode::Hex => self.hex = Session::new(DEFAULT_EXTENT),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seamless_tiler::{Boundary, PatternId};

    fn select_hex(model: &mut EditorModel) {
        assert!(model.set_mode(GridMode::Hex));
    }

    #[test]
    fn catalogs_deduplicate_socket_equivalent_transforms() {
        let mut model = EditorModel::default();
        let square_counts: Vec<_> = model
            .tiles()
            .into_iter()
            .map(|(tile, _)| model.variants_for_tile(tile).len())
            .collect();
        assert_eq!(square_counts, vec![1, 2, 4, 4, 1]);

        select_hex(&mut model);
        let hex_counts: Vec<_> = model
            .tiles()
            .into_iter()
            .map(|(tile, _)| model.variants_for_tile(tile).len())
            .collect();
        assert_eq!(hex_counts, vec![1, 3, 6, 2, 1]);
    }

    #[test]
    fn bounded_hex_edges_close_paths_while_wrapped_edges_connect() {
        let mut model = EditorModel::new(Extent2::new(1, 1));
        select_hex(&mut model);
        assert_eq!(
            model.cell_visual(Coord2::ZERO),
            CellVisual::Collapsed {
                variant: 0,
                pinned: false,
            }
        );
        model.set_boundaries(AxisBoundaries::TOROIDAL);
        assert!(matches!(
            model.cell_visual(Coord2::ZERO),
            CellVisual::Superposition { candidates: 5, .. }
        ));
    }

    fn assert_base_tile_weights<M: ModeSpec>(session: &Session<M>) {
        let wave = session.wave.as_ref().unwrap();
        let mut totals = vec![0.0; session.tiles.len()];
        for (pattern_index, variant_index) in session.pattern_variants.iter().copied().enumerate() {
            let tile = session.variants[variant_index].tile;
            totals[tile.index()] += wave.rules().weight(PatternId::new(pattern_index)).unwrap();
        }
        for total in totals {
            assert!((total - 1.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn enabled_orientations_split_each_base_tile_weight_in_both_modes() {
        let mut model = EditorModel::default();
        let square_straight = model.square.variants_for_tile(TileId::new(1))[0];
        model.square.set_variant_enabled(square_straight, false);
        assert_base_tile_weights(&model.square);

        let hex_straight = model.hex.variants_for_tile(TileId::new(1))[0];
        model.hex.set_variant_enabled(hex_straight, false);
        assert_base_tile_weights(&model.hex);
    }

    #[test]
    fn pins_and_solver_progress_survive_mode_switches() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        let square_blank = model.variants_for_tile(TileId::new(0))[0];
        let disabled_square = model.variants_for_tile(TileId::new(1))[0];
        model.set_boundaries(AxisBoundaries::new(Boundary::Wrap, Boundary::Bounded));
        model.set_variant_enabled(disabled_square, false);
        model.set_tool(CanvasTool::Pin(square_blank));
        assert!(model.apply_tool(Coord2::ZERO, false));
        model.step();
        let square_observations = model.observations();

        select_hex(&mut model);
        model.resize(Extent2::new(4, 2));
        model.set_seed(99);
        model.set_boundaries(AxisBoundaries::TOROIDAL);
        let hex_hub = model.variants_for_tile(TileId::new(4))[0];
        model.set_tool(CanvasTool::Pin(hex_hub));
        assert!(model.apply_tool(Coord2::new(1, 1), false));

        model.set_mode(GridMode::Square);
        assert_eq!(model.extent(), Extent2::new(3, 3));
        assert_eq!(model.pin_variant_at(Coord2::ZERO), Some(square_blank));
        assert_eq!(model.observations(), square_observations);
        assert!(!model.variant_enabled(disabled_square));
        assert_eq!(
            model.boundaries(),
            AxisBoundaries::new(Boundary::Wrap, Boundary::Bounded)
        );

        model.set_mode(GridMode::Hex);
        assert_eq!(model.extent(), Extent2::new(4, 2));
        assert_eq!(model.seed(), 99);
        assert_eq!(model.boundaries(), AxisBoundaries::TOROIDAL);
        assert_eq!(model.pin_variant_at(Coord2::new(1, 1)), Some(hex_hub));
    }

    #[test]
    fn disabling_a_variant_clears_its_pins_and_pin_tool() {
        let mut model = EditorModel::default();
        let variant = model.variants_for_tile(TileId::new(0))[0];
        model.set_tool(CanvasTool::Pin(variant));
        model.apply_tool(Coord2::ZERO, false);
        assert_eq!(model.set_variant_enabled(variant, false), 1);
        assert_eq!(model.pin_variant_at(Coord2::ZERO), None);
        assert_eq!(model.tool(), CanvasTool::Inspect);
    }

    #[test]
    fn resizing_preserves_overlapping_pins_and_drops_selection() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        select_hex(&mut model);
        let blank = model.variants_for_tile(TileId::new(0))[0];
        model.set_tool(CanvasTool::Pin(blank));
        model.apply_tool(Coord2::new(0, 0), false);
        model.apply_tool(Coord2::new(2, 2), false);
        assert!(model.resize(Extent2::new(2, 2)));
        assert_eq!(model.pin_variant_at(Coord2::new(0, 0)), Some(blank));
        assert_eq!(model.selected_cell(), None);
        assert!(!model.resize(Extent2::new(0, 2)));
    }

    #[test]
    fn pinned_hex_tiles_survive_wave_restart() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        select_hex(&mut model);
        let hub = model.variants_for_tile(TileId::new(4))[0];
        model.set_tool(CanvasTool::Pin(hub));
        model.apply_tool(Coord2::new(1, 1), false);
        model.step();
        model.reset_wave();
        assert_eq!(model.pin_variant_at(Coord2::new(1, 1)), Some(hub));
        assert_eq!(model.observations(), 0);
    }

    #[test]
    fn playback_stops_at_terminal_states_in_both_modes() {
        let mut model = EditorModel::new(Extent2::new(2, 2));
        for mode in GridMode::ALL {
            model.set_mode(mode);
            model.toggle_running();
            assert!(model.running());
            model.finish();
            assert!(!model.running());
            assert_ne!(model.status(), Some(WfcStatus::Running));
        }
    }

    #[test]
    fn adding_a_tile_appends_variants_without_disturbing_existing_pins() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        let corner = model.variants_for_tile(TileId::new(2))[0];
        model.set_tool(CanvasTool::Pin(corner));
        assert!(model.apply_tool(Coord2::ZERO, false));
        let before = model.tiles().len();

        model.add_tile();

        assert_eq!(model.tiles().len(), before + 1);
        // New variants append at the end, so existing indices and pins are stable.
        assert_eq!(model.pin_variant_at(Coord2::ZERO), Some(corner));
        assert_eq!(model.variants_for_tile(TileId::new(2))[0], corner);
    }

    #[test]
    fn painting_preserves_enabled_toggles_by_identity() {
        let mut model = EditorModel::default();
        let before = model.variants_for_tile(TileId::new(2))[1];
        model.set_variant_enabled(before, false);
        assert!(!model.variant_enabled(before));

        // Make Blank (tile 0) asymmetric: it gains variants ahead of Corner,
        // shifting Corner's dense indices.
        assert!(model.paint_selected_tile(
            Coord2::new(3, 7),
            Coord2::new(3, 7),
            1,
            [255, 0, 255, 255],
        ));

        let after = model.variants_for_tile(TileId::new(2))[1];
        assert_ne!(
            after, before,
            "editing an earlier tile should shift indices"
        );
        assert!(
            !model.variant_enabled(after),
            "the disabled orientation stays disabled by identity"
        );
    }

    #[test]
    fn deleting_a_tile_compacts_ids_and_drops_only_its_pins() {
        let mut model = EditorModel::new(Extent2::new(4, 4));
        let straight = model.variants_for_tile(TileId::new(1))[0];
        let cross = model.variants_for_tile(TileId::new(4))[0];
        model.set_tool(CanvasTool::Pin(straight));
        assert!(model.apply_tool(Coord2::new(0, 0), false));
        model.set_tool(CanvasTool::Pin(cross));
        assert!(model.apply_tool(Coord2::new(2, 2), false));

        model.remove_tile(TileId::new(1));

        // Straight's pin is gone; Cross's pin survives, remapped by identity.
        assert_eq!(model.pin_variant_at(Coord2::new(0, 0)), None);
        let cross_now = model.variants_for_tile(TileId::new(3))[0];
        assert_eq!(model.pin_variant_at(Coord2::new(2, 2)), Some(cross_now));

        // TileIds compact: Corner slides into slot 1.
        let names: Vec<_> = model
            .tiles()
            .into_iter()
            .map(|(_, style)| style.name)
            .collect();
        assert_eq!(names, vec!["Blank", "Corner", "T junction", "Cross"]);
    }

    #[test]
    fn renaming_and_recoloring_leave_the_wave_untouched() {
        let mut model = EditorModel::default();
        model.step();
        model.step();
        let observations = model.observations();
        let status = model.status();
        let raster_before = model.variant_raster(0).cloned().unwrap();
        let sockets_before = model.square.variant_sockets[0].clone();

        model.set_tile_name(TileId::new(0), "Empty".to_owned());
        model.set_tile_color(TileId::new(0), [1, 2, 3]);

        assert_eq!(model.observations(), observations);
        assert_eq!(model.status(), status);
        let style = model.tile_style(TileId::new(0)).unwrap();
        assert_eq!(style.name, "Empty");
        assert_eq!(style.color, [1, 2, 3]);
        assert_eq!(model.variant_raster(0).unwrap(), &raster_before);
        assert_eq!(model.square.variant_sockets[0], sockets_before);
    }

    #[test]
    fn square_tile_sockets_are_always_extracted_from_the_owned_raster() {
        let mut model = EditorModel::default();
        for (_, tile) in model.square.tiles.iter() {
            assert_eq!(tile.sockets, tile.payload.raster.edges());
        }

        assert!(model.paint_selected_tile(
            Coord2::new(3, 0),
            Coord2::new(3, 0),
            1,
            DEFAULT_PAINT_COLOR,
        ));
        let tile = model.square.tiles.get(TileId::new(0)).unwrap();
        assert_eq!(tile.sockets, tile.payload.raster.edges());
        assert_ne!(tile.sockets[SquareDirection::North], closed_edge());
    }

    #[test]
    fn square_variants_deduplicate_by_complete_oriented_raster() {
        let mut tile = square_tile("Asymmetric".to_owned(), [80, 120, 160], [false; 4]);
        tile.payload.raster.set(3, 7, [255, 0, 255, 255]);
        tile.sockets = tile.payload.raster.edges();
        let mut tiles = TileSet::new();
        tiles.insert(tile);

        let derived = distinct_variants::<SquareMode>(&tiles);
        assert_eq!(derived.variants.len(), 8);
        let distinct = derived
            .rasters
            .into_iter()
            .flatten()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(distinct.len(), 8);
    }

    fn assert_square_neighbors_have_equal_edges(session: &Session<SquareMode>) {
        assert_eq!(session.status(), Some(WfcStatus::Solved));
        for cell_index in 0..session.topology.cell_count() {
            let cell = CellId::new(cell_index);
            let coord = session.topology.coordinate(cell).unwrap();
            let CellVisual::Collapsed {
                variant: source, ..
            } = session.cell_visual(coord)
            else {
                panic!("a solved cell must be collapsed");
            };
            for direction in SquareDirection::ALL.iter().copied() {
                let source_edge = &session.variant_sockets[source][direction.index()];
                if let Some(neighbor_cell) = session.topology.neighbor(cell, direction) {
                    let neighbor_coord = session.topology.coordinate(neighbor_cell).unwrap();
                    let CellVisual::Collapsed {
                        variant: neighbor, ..
                    } = session.cell_visual(neighbor_coord)
                    else {
                        panic!("a solved neighbor must be collapsed");
                    };
                    assert_eq!(
                        source_edge,
                        &session.variant_sockets[neighbor][direction.opposite().index()],
                        "cell {coord:?} toward {direction:?}"
                    );
                } else {
                    assert_eq!(source_edge, &closed_edge());
                }
            }
        }
    }

    #[test]
    fn solved_square_grids_are_pixel_continuous_at_bounded_and_wrapped_edges() {
        let mut bounded = Session::<SquareMode>::new(Extent2::new(4, 3));
        bounded.finish();
        assert_square_neighbors_have_equal_edges(&bounded);

        let mut wrapped = Session::<SquareMode>::new(Extent2::new(4, 3));
        wrapped.set_boundaries(AxisBoundaries::TOROIDAL);
        wrapped.finish();
        assert_square_neighbors_have_equal_edges(&wrapped);
    }

    #[test]
    fn catalog_version_bumps_on_appearance_edits_only() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        let start = model.catalog_version();

        assert!(model.paint_selected_tile(
            Coord2::new(3, 7),
            Coord2::new(3, 7),
            1,
            DEFAULT_PAINT_COLOR,
        ));
        let after_paint = model.catalog_version();
        assert!(after_paint > start);

        model.set_tile_color(TileId::new(0), [1, 2, 3]);
        let after_color = model.catalog_version();
        assert!(after_color > after_paint);

        model.add_tile();
        let after_add = model.catalog_version();
        assert!(after_add > after_color);

        model.remove_tile(TileId::new(0));
        let after_remove = model.catalog_version();
        assert!(after_remove > after_add);

        // Rename and enable-toggle do not change the tile's appearance.
        model.set_tile_name(TileId::new(0), "renamed".to_owned());
        assert_eq!(model.catalog_version(), after_remove);
        let variant = model.variants_for_tile(TileId::new(0))[0];
        model.set_variant_enabled(variant, false);
        assert_eq!(model.catalog_version(), after_remove);
    }

    #[test]
    fn painting_and_erasing_edges_rebuilds_the_wave_live() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        model.step();
        assert_eq!(model.observations(), 1);
        let start_version = model.catalog_version();

        assert!(model.paint_selected_tile(
            Coord2::new(4, 0),
            Coord2::new(4, 0),
            1,
            DEFAULT_PAINT_COLOR,
        ));
        assert_eq!(model.observations(), 0);
        assert!(model.catalog_version() > start_version);
        let tile = model.square.tiles.get(TileId::new(0)).unwrap();
        assert_ne!(tile.sockets[SquareDirection::North], closed_edge());

        let painted_version = model.catalog_version();
        assert!(!model.paint_selected_tile(
            Coord2::new(4, 0),
            Coord2::new(4, 0),
            1,
            DEFAULT_PAINT_COLOR,
        ));
        assert_eq!(model.catalog_version(), painted_version);

        assert!(model.paint_selected_tile(
            Coord2::new(4, 0),
            Coord2::new(4, 0),
            1,
            crate::raster::EDGE_BACKGROUND,
        ));
        let tile = model.square.tiles.get(TileId::new(0)).unwrap();
        assert_eq!(tile.sockets[SquareDirection::North], closed_edge());
    }

    #[test]
    fn authoring_selection_tracks_catalog_additions_and_removals() {
        let mut model = EditorModel::default();
        assert_eq!(model.selected_tile(), Some(TileId::new(0)));
        assert!(model.set_selected_tile(TileId::new(2)));

        model.remove_tile(TileId::new(1));
        assert_eq!(model.selected_tile(), Some(TileId::new(1)));
        model.remove_tile(TileId::new(1));
        assert_eq!(model.selected_tile(), Some(TileId::new(1)));

        model.add_tile();
        assert_eq!(
            model.selected_tile(),
            Some(TileId::new(model.tiles().len() - 1))
        );

        while let Some(tile) = model.selected_tile() {
            model.remove_tile(tile);
        }
        assert!(model.tiles().is_empty());
        assert!(model.selected_raster().is_none());

        model.add_tile();
        assert_eq!(model.selected_tile(), Some(TileId::new(0)));
        assert!(model.selected_raster().is_some());
    }

    #[test]
    fn pencil_editing_is_unavailable_in_hex_mode() {
        let mut model = EditorModel::default();
        select_hex(&mut model);
        let before = model.catalog_version();
        assert!(!model.paint_selected_tile(
            Coord2::ZERO,
            Coord2::new(5, 5),
            3,
            DEFAULT_PAINT_COLOR,
        ));
        assert_eq!(model.catalog_version(), before);
        assert!(model.selected_raster().is_none());
    }

    #[test]
    fn default_seed_solves_both_default_grids() {
        let mut model = EditorModel::default();
        model.finish();
        assert_eq!(model.status(), Some(WfcStatus::Solved));
        select_hex(&mut model);
        assert_eq!(model.seed(), DEFAULT_HEX_SEED);
        model.finish();
        assert_eq!(model.status(), Some(WfcStatus::Solved));
    }
}
