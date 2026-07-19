use std::marker::PhantomData;

use seamless_tiler::{
    AxisBoundaries, CellId, Coord2, D4, D6, Direction, DirectionTransform, EqualityMatcher,
    Extent2, Grid, HexDirection, HexTopology, OrientedTileId, SocketMap, SocketMatcher,
    SquareDirection, SquareTopology, Tile, TileId, TileSet, Topology, Wfc, WfcRules, WfcStatus,
};

pub(crate) const DEFAULT_EXTENT: Extent2 = Extent2::new(12, 8);
pub(crate) const MAX_DIMENSION: usize = 64;
pub(crate) const DEFAULT_SEED: u64 = 1;
const DEFAULT_HEX_SEED: u64 = 3;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TileStyle {
    pub(crate) name: &'static str,
    pub(crate) color: [u8; 3],
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
    type Transform: DirectionTransform<Self::Direction> + Copy + Eq;
    type Topology: Topology<Coord = Coord2, Direction = Self::Direction> + Copy;

    fn topology(extent: Extent2, boundaries: AxisBoundaries) -> Self::Topology;
    fn transforms() -> &'static [Self::Transform];
    fn orientation(transform: Self::Transform) -> Orientation;
    fn demo_tiles() -> TileSet<TileStyle, Self::Direction, bool>;
    fn default_seed() -> u64;
}

struct SquareMode;

impl ModeSpec for SquareMode {
    type Direction = SquareDirection;
    type Transform = D4;
    type Topology = SquareTopology;

    fn topology(extent: Extent2, boundaries: AxisBoundaries) -> Self::Topology {
        SquareTopology::new(extent, boundaries).expect("editor dimensions fit signed coordinates")
    }

    fn transforms() -> &'static [Self::Transform] {
        &D4::ALL
    }

    fn orientation(transform: Self::Transform) -> Orientation {
        Orientation::Square(transform)
    }

    fn demo_tiles() -> TileSet<TileStyle, Self::Direction, bool> {
        let mut tiles = TileSet::with_capacity(5);
        insert_demo_tile(&mut tiles, "Blank", [72, 79, 89], |_| false);
        insert_demo_tile(&mut tiles, "Straight", [55, 118, 171], |direction| {
            matches!(direction, SquareDirection::North | SquareDirection::South)
        });
        insert_demo_tile(&mut tiles, "Corner", [46, 139, 87], |direction| {
            matches!(direction, SquareDirection::North | SquareDirection::East)
        });
        insert_demo_tile(&mut tiles, "T junction", [157, 112, 40], |direction| {
            direction != SquareDirection::South
        });
        insert_demo_tile(&mut tiles, "Cross", [135, 80, 156], |_| true);
        tiles
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

    fn topology(extent: Extent2, boundaries: AxisBoundaries) -> Self::Topology {
        HexTopology::new(extent, boundaries).expect("editor dimensions fit signed coordinates")
    }

    fn transforms() -> &'static [Self::Transform] {
        &D6::ALL
    }

    fn orientation(transform: Self::Transform) -> Orientation {
        Orientation::Hex(transform)
    }

    fn demo_tiles() -> TileSet<TileStyle, Self::Direction, bool> {
        let mut tiles = TileSet::with_capacity(5);
        insert_demo_tile(&mut tiles, "Blank", [72, 79, 89], |_| false);
        insert_demo_tile(&mut tiles, "Straight", [55, 118, 171], |direction| {
            matches!(direction, HexDirection::NorthEast | HexDirection::SouthWest)
        });
        insert_demo_tile(&mut tiles, "Bend", [46, 139, 87], |direction| {
            matches!(direction, HexDirection::NorthEast | HexDirection::East)
        });
        insert_demo_tile(&mut tiles, "Y junction", [157, 112, 40], |direction| {
            matches!(
                direction,
                HexDirection::NorthEast | HexDirection::SouthEast | HexDirection::West
            )
        });
        insert_demo_tile(&mut tiles, "Hub", [135, 80, 156], |_| true);
        tiles
    }

    fn default_seed() -> u64 {
        DEFAULT_HEX_SEED
    }
}

struct Session<M: ModeSpec> {
    pins: Grid<Option<usize>>,
    topology: M::Topology,
    boundaries: AxisBoundaries,
    tiles: TileSet<TileStyle, M::Direction, bool>,
    variants: Vec<OrientedTileId<M::Transform>>,
    variant_sockets: Vec<Vec<bool>>,
    enabled: Vec<bool>,
    pattern_variants: Vec<usize>,
    wave: Option<Wfc<M::Topology>>,
    seed: u64,
    selected_cell: Option<Coord2>,
    tool: CanvasTool,
    running: bool,
    observations: usize,
    last_observed: Option<Coord2>,
    mode: PhantomData<M>,
}

impl<M: ModeSpec> Session<M> {
    fn new(extent: Extent2) -> Self {
        assert!(
            EditorModel::valid_extent(extent),
            "editor extent must be between 1 and 64"
        );
        let tiles = M::demo_tiles();
        let (variants, variant_sockets) = distinct_variants::<M>(&tiles);
        let mut session = Self {
            pins: Grid::filled(extent, None).expect("editor dimensions have a valid area"),
            topology: M::topology(extent, AxisBoundaries::BOUNDED),
            boundaries: AxisBoundaries::BOUNDED,
            enabled: vec![true; variants.len()],
            tiles,
            variants,
            variant_sockets,
            pattern_variants: Vec::new(),
            wave: None,
            seed: M::default_seed(),
            selected_cell: None,
            tool: CanvasTool::Inspect,
            running: false,
            observations: 0,
            last_observed: None,
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
        let rules = WfcRules::new(weights, |direction, source, neighbor| {
            let source = self.variants[self.pattern_variants[source.index()]];
            let neighbor = self.variants[self.pattern_variants[neighbor.index()]];
            let source_tile = self
                .tiles
                .get(source.tile)
                .expect("catalog variants refer to demo tiles");
            let neighbor_tile = self
                .tiles
                .get(neighbor.tile)
                .expect("catalog variants refer to demo tiles");
            EqualityMatcher.matches(
                direction,
                source_tile.oriented_socket(source.transform, direction),
                neighbor_tile.oriented_socket(neighbor.transform, direction.opposite()),
            )
        })
        .expect("enabled demo patterns have valid weights");

        let topology = self.topology;
        let wave = Wfc::with_constraints(topology, rules, self.seed, |cell, pattern| {
            let variant_index = self.pattern_variants[pattern.index()];
            let variant = self.variants[variant_index];
            let tile = self
                .tiles
                .get(variant.tile)
                .expect("catalog variants refer to demo tiles");
            let pin_matches = topology
                .coordinate(cell)
                .and_then(|coord| self.pins.get(coord))
                .is_none_or(|pin| pin.is_none_or(|pin| pin == variant_index));
            pin_matches
                && M::Direction::ALL.iter().copied().all(|direction| {
                    topology.neighbor(cell, direction).is_some()
                        || !tile.oriented_socket(variant.transform, direction)
                })
        });
        self.wave = Some(wave);
    }
}

fn distinct_variants<M: ModeSpec>(
    tiles: &TileSet<TileStyle, M::Direction, bool>,
) -> (Vec<OrientedTileId<M::Transform>>, Vec<Vec<bool>>) {
    let mut variants = Vec::new();
    let mut sockets = Vec::new();
    for (tile_id, tile) in tiles.iter() {
        let mut signatures = Vec::new();
        for transform in M::transforms().iter().copied() {
            let signature: Vec<_> = M::Direction::ALL
                .iter()
                .copied()
                .map(|direction| *tile.oriented_socket(transform, direction))
                .collect();
            if signatures.contains(&signature) {
                continue;
            }
            signatures.push(signature.clone());
            variants.push(OrientedTileId::new(tile_id, transform));
            sockets.push(signature);
        }
    }
    (variants, sockets)
}

fn insert_demo_tile<D: Direction>(
    tiles: &mut TileSet<TileStyle, D, bool>,
    name: &'static str,
    color: [u8; 3],
    socket: impl FnMut(D) -> bool,
) -> TileId {
    tiles.insert(Tile::new(
        TileStyle { name, color },
        SocketMap::from_fn(socket),
    ))
}

trait SessionAccess {
    fn extent(&self) -> Extent2;
    fn boundaries(&self) -> AxisBoundaries;
    fn tiles(&self) -> Vec<(TileId, TileStyle)>;
    fn tile_style(&self, tile: TileId) -> Option<TileStyle>;
    fn variant(&self, index: usize) -> Option<VariantView>;
    fn variant_sockets(&self, index: usize) -> &[bool];
    fn variant_enabled(&self, index: usize) -> bool;
    fn variants_for_tile(&self, tile: TileId) -> Vec<usize>;
    fn enabled_variant_count(&self) -> usize;
    fn seed(&self) -> u64;
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
            .map(|(tile, value)| (tile, value.payload))
            .collect()
    }
    fn tile_style(&self, tile: TileId) -> Option<TileStyle> {
        self.tiles.get(tile).map(|value| value.payload)
    }
    fn variant(&self, index: usize) -> Option<VariantView> {
        let placement = *self.variants.get(index)?;
        Some(VariantView {
            tile: placement.tile,
            orientation: M::orientation(placement.transform),
        })
    }
    fn variant_sockets(&self, index: usize) -> &[bool] {
        self.variant_sockets.get(index).map_or(&[], Vec::as_slice)
    }
    fn variant_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
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
    pub(crate) fn variant_sockets(&self, index: usize) -> &[bool] {
        self.active().variant_sockets(index)
    }
    pub(crate) fn variant_enabled(&self, index: usize) -> bool {
        self.active().variant_enabled(index)
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
