use seamless_tiler::{
    AxisBoundaries, Coord2, D4, Direction, EqualityMatcher, Extent2, Grid, OrientedTileId,
    SocketMap, SocketMatcher, SquareDirection, SquareTopology, Tile, TileId, TileSet, Topology,
    Wfc, WfcRules, WfcStatus,
};

pub(crate) const DEFAULT_EXTENT: Extent2 = Extent2::new(12, 8);
pub(crate) const MAX_DIMENSION: usize = 64;
pub(crate) const DEFAULT_SEED: u64 = 1;

pub(crate) type Placement = OrientedTileId<D4>;
pub(crate) type DemoTileSet = TileSet<TileStyle, SquareDirection, bool>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TileStyle {
    pub(crate) name: &'static str,
    pub(crate) color: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TileVariant {
    pub(crate) placement: Placement,
    pub(crate) sockets: [bool; 4],
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

pub(crate) struct EditorModel {
    pins: Grid<Option<Placement>>,
    topology: SquareTopology,
    tiles: DemoTileSet,
    variants: Vec<TileVariant>,
    enabled: Vec<bool>,
    pattern_variants: Vec<usize>,
    wave: Option<Wfc<SquareTopology>>,
    seed: u64,
    selected_cell: Option<Coord2>,
    tool: CanvasTool,
    running: bool,
    observations: usize,
    last_observed: Option<Coord2>,
}

impl Default for EditorModel {
    fn default() -> Self {
        Self::new(DEFAULT_EXTENT)
    }
}

impl EditorModel {
    pub(crate) fn new(extent: Extent2) -> Self {
        assert!(
            Self::valid_extent(extent),
            "editor extent must be between 1 and 64"
        );
        let tiles = demo_tiles();
        let variants = distinct_variants(&tiles);
        let mut model = Self {
            pins: Grid::filled(extent, None).expect("editor dimensions have a valid area"),
            topology: SquareTopology::bounded(extent)
                .expect("editor dimensions fit signed coordinates"),
            enabled: vec![true; variants.len()],
            tiles,
            variants,
            pattern_variants: Vec::new(),
            wave: None,
            seed: DEFAULT_SEED,
            selected_cell: None,
            tool: CanvasTool::Inspect,
            running: false,
            observations: 0,
            last_observed: None,
        };
        model.rebuild_wave();
        model
    }

    pub(crate) fn valid_extent(extent: Extent2) -> bool {
        (1..=MAX_DIMENSION).contains(&extent.width) && (1..=MAX_DIMENSION).contains(&extent.height)
    }

    pub(crate) fn extent(&self) -> Extent2 {
        self.pins.extent()
    }

    pub(crate) fn boundaries(&self) -> AxisBoundaries {
        self.topology.boundaries()
    }

    pub(crate) fn topology(&self) -> &SquareTopology {
        &self.topology
    }

    pub(crate) fn tiles(&self) -> &DemoTileSet {
        &self.tiles
    }

    pub(crate) fn variants(&self) -> &[TileVariant] {
        &self.variants
    }

    pub(crate) fn variant_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    pub(crate) fn variants_for_tile(&self, tile: TileId) -> impl Iterator<Item = usize> + '_ {
        self.variants
            .iter()
            .enumerate()
            .filter_map(move |(index, variant)| (variant.placement.tile == tile).then_some(index))
    }

    pub(crate) fn enabled_variant_count(&self) -> usize {
        self.pattern_variants.len()
    }

    pub(crate) fn seed(&self) -> u64 {
        self.seed
    }

    pub(crate) fn selected_cell(&self) -> Option<Coord2> {
        self.selected_cell
    }

    pub(crate) fn tool(&self) -> CanvasTool {
        self.tool
    }

    pub(crate) fn set_tool(&mut self, tool: CanvasTool) {
        if let CanvasTool::Pin(index) = tool
            && !self.variant_enabled(index)
        {
            return;
        }
        self.tool = tool;
    }

    pub(crate) fn running(&self) -> bool {
        self.running
    }

    pub(crate) fn observations(&self) -> usize {
        self.observations
    }

    pub(crate) fn last_observed(&self) -> Option<Coord2> {
        self.last_observed
    }

    pub(crate) fn status(&self) -> Option<WfcStatus> {
        self.wave.as_ref().map(Wfc::status)
    }

    pub(crate) fn initial_contradiction(&self) -> bool {
        self.observations == 0 && matches!(self.status(), Some(WfcStatus::Contradiction { .. }))
    }

    pub(crate) fn unresolved_count(&self) -> usize {
        let Some(wave) = &self.wave else {
            return 0;
        };
        (0..self.topology.cell_count())
            .filter(|index| wave.candidate_count(seamless_tiler::CellId::new(*index)) > Some(1))
            .count()
    }

    pub(crate) fn pin_at(&self, coord: Coord2) -> Option<Placement> {
        self.pins.get(coord).copied().flatten()
    }

    pub(crate) fn cell_visual(&self, coord: Coord2) -> CellVisual {
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
                    pinned: self.pin_at(coord).is_some(),
                }
            }
            Some(candidates) => CellVisual::Superposition {
                candidates,
                entropy: wave.entropy(cell).expect("a non-empty domain has entropy"),
            },
            None => CellVisual::Unavailable,
        }
    }

    pub(crate) fn candidate_variants(&self, coord: Coord2) -> Vec<usize> {
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

    pub(crate) fn resize(&mut self, extent: Extent2) -> bool {
        if !Self::valid_extent(extent) {
            return false;
        }

        let mut resized = Grid::filled(extent, None).expect("editor dimensions have a valid area");
        for (coord, pin) in self.pins.cells() {
            if let Some(target) = resized.get_mut(coord) {
                *target = *pin;
            }
        }
        self.pins = resized;
        self.topology = SquareTopology::new(extent, self.boundaries())
            .expect("editor dimensions fit signed coordinates");
        if self
            .selected_cell
            .is_some_and(|coord| !extent.contains(coord))
        {
            self.selected_cell = None;
        }
        self.rebuild_wave();
        true
    }

    pub(crate) fn set_boundaries(&mut self, boundaries: AxisBoundaries) {
        if boundaries == self.boundaries() {
            return;
        }
        self.topology = SquareTopology::new(self.extent(), boundaries)
            .expect("existing editor dimensions fit signed coordinates");
        self.rebuild_wave();
    }

    pub(crate) fn set_seed(&mut self, seed: u64) {
        if seed != self.seed {
            self.seed = seed;
            self.rebuild_wave();
        }
    }

    pub(crate) fn set_variant_enabled(&mut self, index: usize, enabled: bool) -> usize {
        let Some(current) = self.enabled.get_mut(index) else {
            return 0;
        };
        if *current == enabled {
            return 0;
        }
        *current = enabled;

        let mut cleared = 0;
        if !enabled {
            let placement = self.variants[index].placement;
            for pin in &mut self.pins {
                if *pin == Some(placement) {
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

    pub(crate) fn apply_tool(&mut self, coord: Coord2, secondary: bool) -> bool {
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
            CanvasTool::Pin(index) => self.set_pin(coord, Some(self.variants[index].placement)),
            CanvasTool::Unpin => self.set_pin(coord, None),
        }
    }

    pub(crate) fn clear_pins(&mut self) -> usize {
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

    pub(crate) fn reset_wave(&mut self) {
        if let Some(wave) = &mut self.wave {
            wave.restart(self.seed);
        }
        self.running = false;
        self.observations = 0;
        self.last_observed = None;
    }

    pub(crate) fn retry(&mut self) -> bool {
        if self.initial_contradiction()
            || !matches!(self.status(), Some(WfcStatus::Contradiction { .. }))
        {
            return false;
        }
        self.seed = self.seed.wrapping_add(1);
        self.rebuild_wave();
        true
    }

    pub(crate) fn step(&mut self) -> bool {
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

    pub(crate) fn finish(&mut self) {
        self.running = false;
        while self.step() {}
    }

    pub(crate) fn toggle_running(&mut self) {
        if matches!(self.status(), Some(WfcStatus::Running)) {
            self.running = !self.running;
        } else {
            self.running = false;
        }
    }

    pub(crate) fn reset_all(&mut self) {
        *self = Self::default();
    }

    fn set_pin(&mut self, coord: Coord2, placement: Option<Placement>) -> bool {
        let Some(pin) = self.pins.get_mut(coord) else {
            return false;
        };
        if *pin == placement {
            return false;
        }
        *pin = placement;
        self.rebuild_wave();
        true
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
            enabled_per_tile[self.variants[*variant_index].placement.tile.index()] += 1;
        }
        let weights = self.pattern_variants.iter().map(|variant_index| {
            let tile = self.variants[*variant_index].placement.tile;
            1.0 / enabled_per_tile[tile.index()] as f64
        });
        let rules = WfcRules::new(weights, |direction, source, neighbor| {
            let source = self.variants[self.pattern_variants[source.index()]].placement;
            let neighbor = self.variants[self.pattern_variants[neighbor.index()]].placement;
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
            let variant = self.variants[self.pattern_variants[pattern.index()]];
            let tile = self
                .tiles
                .get(variant.placement.tile)
                .expect("catalog variants refer to demo tiles");
            let pin_matches = topology
                .coordinate(cell)
                .and_then(|coord| self.pins.get(coord))
                .is_none_or(|pin| pin.is_none_or(|pin| pin == variant.placement));
            pin_matches
                && SquareDirection::ALL.iter().copied().all(|direction| {
                    topology.neighbor(cell, direction).is_some()
                        || !tile.oriented_socket(variant.placement.transform, direction)
                })
        });
        self.wave = Some(wave);
    }
}

fn distinct_variants(tiles: &DemoTileSet) -> Vec<TileVariant> {
    let mut variants = Vec::new();
    for (tile_id, tile) in tiles.iter() {
        let mut signatures = Vec::new();
        for transform in D4::ALL {
            let sockets = std::array::from_fn(|index| {
                *tile.oriented_socket(transform, SquareDirection::ALL[index])
            });
            if signatures.contains(&sockets) {
                continue;
            }
            signatures.push(sockets);
            variants.push(TileVariant {
                placement: Placement::new(tile_id, transform),
                sockets,
            });
        }
    }
    variants
}

fn demo_tiles() -> DemoTileSet {
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

fn insert_demo_tile(
    tiles: &mut DemoTileSet,
    name: &'static str,
    color: [u8; 3],
    socket: impl FnMut(SquareDirection) -> bool,
) -> TileId {
    tiles.insert(Tile::new(
        TileStyle { name, color },
        SocketMap::from_fn(socket),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use seamless_tiler::PatternId;

    #[test]
    fn catalog_deduplicates_socket_equivalent_transforms() {
        let model = EditorModel::default();
        let counts: Vec<_> = model
            .tiles
            .iter()
            .map(|(tile, _)| model.variants_for_tile(tile).count())
            .collect();
        assert_eq!(counts, vec![1, 2, 4, 4, 1]);
        assert_eq!(model.variants.len(), 12);
    }

    #[test]
    fn enabled_orientations_split_each_base_tile_weight() {
        let mut model = EditorModel::default();
        let straight = TileId::new(1);
        let disabled = model.variants_for_tile(straight).next().unwrap();
        model.set_variant_enabled(disabled, false);

        let wave = model.wave.as_ref().unwrap();
        let mut totals = vec![0.0; model.tiles.len()];
        for (pattern_index, variant_index) in model.pattern_variants.iter().copied().enumerate() {
            let tile = model.variants[variant_index].placement.tile;
            totals[tile.index()] += wave.rules().weight(PatternId::new(pattern_index)).unwrap();
        }
        for total in totals {
            assert!((total - 1.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn bounded_edges_close_paths_while_wrapped_edges_connect() {
        let mut model = EditorModel::new(Extent2::new(1, 1));
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
            CellVisual::Superposition { candidates: 4, .. }
        ));
    }

    #[test]
    fn pins_survive_restart_and_retry() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        let cross = model.variants_for_tile(TileId::new(4)).next().unwrap();
        model.set_tool(CanvasTool::Pin(cross));
        assert!(model.apply_tool(Coord2::new(1, 1), false));
        let pin = model.pin_at(Coord2::new(1, 1));

        model.reset_wave();
        assert_eq!(model.pin_at(Coord2::new(1, 1)), pin);
        model.finish();
        if matches!(model.status(), Some(WfcStatus::Contradiction { .. })) {
            assert!(model.retry());
            assert_eq!(model.pin_at(Coord2::new(1, 1)), pin);
        }
    }

    #[test]
    fn conflicting_pins_are_an_initial_contradiction() {
        let mut model = EditorModel::new(Extent2::new(2, 1));
        model.set_boundaries(AxisBoundaries::TOROIDAL);
        let blank = model.variants_for_tile(TileId::new(0)).next().unwrap();
        let horizontal = model
            .variants_for_tile(TileId::new(1))
            .find(|index| {
                let sockets = model.variants[*index].sockets;
                sockets[SquareDirection::East.index()]
            })
            .unwrap();
        model.set_tool(CanvasTool::Pin(blank));
        model.apply_tool(Coord2::new(0, 0), false);
        model.set_tool(CanvasTool::Pin(horizontal));
        model.apply_tool(Coord2::new(1, 0), false);

        assert!(model.initial_contradiction());
        assert!(!model.retry());
    }

    #[test]
    fn disabling_a_variant_clears_its_pins_and_pin_tool() {
        let mut model = EditorModel::default();
        let variant = model.variants_for_tile(TileId::new(0)).next().unwrap();
        model.set_tool(CanvasTool::Pin(variant));
        model.apply_tool(Coord2::ZERO, false);

        assert_eq!(model.set_variant_enabled(variant, false), 1);
        assert_eq!(model.pin_at(Coord2::ZERO), None);
        assert_eq!(model.tool(), CanvasTool::Inspect);
    }

    #[test]
    fn resizing_preserves_overlapping_pins_and_drops_selection() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        let blank = model.variants_for_tile(TileId::new(0)).next().unwrap();
        model.set_tool(CanvasTool::Pin(blank));
        model.apply_tool(Coord2::new(0, 0), false);
        model.apply_tool(Coord2::new(2, 2), false);

        assert!(model.resize(Extent2::new(2, 2)));
        assert!(model.pin_at(Coord2::new(0, 0)).is_some());
        assert_eq!(model.selected_cell(), None);
        assert!(!model.resize(Extent2::new(0, 2)));
    }

    #[test]
    fn playback_stops_at_a_terminal_state() {
        let mut model = EditorModel::new(Extent2::new(2, 2));
        model.toggle_running();
        assert!(model.running());
        model.finish();
        assert!(!model.running());
        assert_ne!(model.status(), Some(WfcStatus::Running));
    }

    #[test]
    fn default_seed_solves_the_default_grid() {
        let mut model = EditorModel::default();
        model.finish();
        assert_eq!(model.status(), Some(WfcStatus::Solved));
    }
}
