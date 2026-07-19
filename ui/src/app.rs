use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use seamless_tiler::{
    AxisBoundaries, Boundary, Coord2, D4, Direction, Extent2, Grid, OrientedTileId, QuarterTurns,
    SocketMap, SquareDirection, SquareTopology, Tile, TileId, TileSet, Topology,
};

const DEFAULT_EXTENT: Extent2 = Extent2::new(12, 8);
const MAX_DIMENSION: usize = 64;
const DEFAULT_CELL_SIZE: f32 = 52.0;

type Placement = OrientedTileId<D4>;
type DemoTileSet = TileSet<TileStyle, SquareDirection, bool>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TileStyle {
    name: &'static str,
    color: [u8; 3],
}

impl TileStyle {
    fn color32(self) -> Color32 {
        Color32::from_rgb(self.color[0], self.color[1], self.color[2])
    }
}

struct EditorModel {
    grid: Grid<Option<Placement>>,
    topology: SquareTopology,
    tiles: DemoTileSet,
    selected_tile: TileId,
    selected_transform: D4,
    selected_cell: Option<Coord2>,
}

impl Default for EditorModel {
    fn default() -> Self {
        Self::new(DEFAULT_EXTENT)
    }
}

impl EditorModel {
    fn new(extent: Extent2) -> Self {
        assert!(
            Self::valid_extent(extent),
            "editor extent must be between 1 and 64"
        );
        let tiles = demo_tiles();
        Self {
            grid: Grid::filled(extent, None).expect("editor dimensions have a valid area"),
            topology: SquareTopology::bounded(extent)
                .expect("editor dimensions fit signed coordinates"),
            tiles,
            selected_tile: TileId::new(0),
            selected_transform: D4::IDENTITY,
            selected_cell: None,
        }
    }

    fn valid_extent(extent: Extent2) -> bool {
        (1..=MAX_DIMENSION).contains(&extent.width) && (1..=MAX_DIMENSION).contains(&extent.height)
    }

    fn extent(&self) -> Extent2 {
        self.grid.extent()
    }

    fn boundaries(&self) -> AxisBoundaries {
        self.topology.boundaries()
    }

    fn paint(&mut self, coord: Coord2) -> bool {
        let placement = Placement::new(self.selected_tile, self.selected_transform);
        let Some(cell) = self.grid.get_mut(coord) else {
            return false;
        };
        *cell = Some(placement);
        self.selected_cell = Some(coord);
        true
    }

    fn erase(&mut self, coord: Coord2) -> bool {
        let Some(cell) = self.grid.get_mut(coord) else {
            return false;
        };
        *cell = None;
        self.selected_cell = Some(coord);
        true
    }

    fn resize(&mut self, extent: Extent2) -> bool {
        if !Self::valid_extent(extent) {
            return false;
        }

        let mut resized = Grid::filled(extent, None).expect("editor dimensions have a valid area");
        for (coord, placement) in self.grid.cells() {
            if let Some(target) = resized.get_mut(coord) {
                *target = *placement;
            }
        }

        self.grid = resized;
        self.topology = SquareTopology::new(extent, self.boundaries())
            .expect("editor dimensions fit signed coordinates");
        if self
            .selected_cell
            .is_some_and(|coord| !extent.contains(coord))
        {
            self.selected_cell = None;
        }
        true
    }

    fn set_boundaries(&mut self, boundaries: AxisBoundaries) {
        self.topology = SquareTopology::new(self.extent(), boundaries)
            .expect("existing editor dimensions fit signed coordinates");
    }

    fn clear(&mut self) {
        for cell in &mut self.grid {
            *cell = None;
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    fn rotate_clockwise(&mut self) {
        let clockwise = D4::new(QuarterTurns::One, false);
        self.selected_transform = clockwise.compose(self.selected_transform);
    }

    fn reflect_horizontally(&mut self) {
        let reflection = D4::new(QuarterTurns::Zero, true);
        self.selected_transform = reflection.compose(self.selected_transform);
    }
}

pub struct TilerApp {
    model: EditorModel,
    width_input: usize,
    height_input: usize,
    cell_size: f32,
}

impl Default for TilerApp {
    fn default() -> Self {
        Self {
            model: EditorModel::default(),
            width_input: DEFAULT_EXTENT.width,
            height_input: DEFAULT_EXTENT.height,
            cell_size: DEFAULT_CELL_SIZE,
        }
    }
}

impl eframe::App for TilerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("instructions").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Seamless Tiler");
                ui.separator();
                ui.label("Left-drag to paint · Right-drag to erase · Select a cell to inspect its topology");
            });
        });

        egui::Panel::left("controls")
            .default_size(250.0)
            .resizable(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.show_controls(ui));
            });

        egui::CentralPanel::default().show(ui, |ui| {
            self.show_canvas(ui);
        });
    }
}

impl TilerApp {
    fn show_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Grid");
        egui::Grid::new("dimensions").show(ui, |ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut self.width_input).range(1..=MAX_DIMENSION));
            ui.end_row();
            ui.label("Height");
            ui.add(egui::DragValue::new(&mut self.height_input).range(1..=MAX_DIMENSION));
            ui.end_row();
        });

        ui.horizontal(|ui| {
            if ui.button("Apply size").clicked() {
                self.model
                    .resize(Extent2::new(self.width_input, self.height_input));
            }
            if ui.button("Clear").clicked() {
                self.model.clear();
            }
            if ui.button("Reset all").clicked() {
                self.model.reset();
                self.width_input = DEFAULT_EXTENT.width;
                self.height_input = DEFAULT_EXTENT.height;
                self.cell_size = DEFAULT_CELL_SIZE;
            }
        });

        ui.add_space(6.0);
        ui.add(egui::Slider::new(&mut self.cell_size, 28.0..=80.0).text("Cell size"));

        ui.separator();
        ui.heading("Topology");
        let current = self.model.boundaries();
        let mut horizontal = current.horizontal;
        let mut vertical = current.vertical;
        boundary_selector(ui, "Horizontal", &mut horizontal);
        boundary_selector(ui, "Vertical", &mut vertical);
        let boundaries = AxisBoundaries::new(horizontal, vertical);
        if boundaries != current {
            self.model.set_boundaries(boundaries);
        }

        ui.separator();
        ui.heading("Tile palette");
        let mut chosen_tile = None;
        for (id, tile) in self.model.tiles.iter() {
            let selected = self.model.selected_tile == id;
            let button = egui::Button::new(tile.payload.name)
                .fill(tile.payload.color32())
                .selected(selected)
                .min_size(Vec2::new(ui.available_width(), 28.0));
            if ui.add(button).clicked() {
                chosen_tile = Some(id);
            }
        }
        if let Some(id) = chosen_tile {
            self.model.selected_tile = id;
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Rotate ↻").clicked() {
                self.model.rotate_clockwise();
            }
            if ui.button("Reflect ↔").clicked() {
                self.model.reflect_horizontally();
            }
        });
        ui.label(format!(
            "Orientation: {}",
            transform_label(self.model.selected_transform)
        ));

        ui.separator();
        self.show_inspector(ui);
    }

    fn show_inspector(&self, ui: &mut egui::Ui) {
        ui.heading("Inspector");
        let Some(coord) = self.model.selected_cell else {
            ui.label("Select a cell to see its contents and neighbors.");
            return;
        };

        let cell = self
            .model
            .topology
            .cell_at(coord)
            .expect("selected coordinates are in bounds");
        ui.monospace(format!(
            "Cell ({}, {}) · id {}",
            coord.x,
            coord.y,
            cell.index()
        ));

        match self.model.grid[coord] {
            Some(placement) => {
                let tile = self
                    .model
                    .tiles
                    .get(placement.tile)
                    .expect("placements refer to demo tiles");
                ui.label(format!(
                    "{} · {}",
                    tile.payload.name,
                    transform_label(placement.transform)
                ));
                ui.horizontal_wrapped(|ui| {
                    ui.label("Sockets:");
                    for direction in SquareDirection::ALL {
                        let open = tile.oriented_socket(placement.transform, *direction);
                        ui.colored_label(
                            if *open {
                                direction_color(*direction)
                            } else {
                                Color32::GRAY
                            },
                            format!(
                                "{}: {}",
                                direction_short(*direction),
                                if *open { "open" } else { "closed" }
                            ),
                        );
                    }
                });
            }
            None => {
                ui.weak("Empty");
            }
        }

        ui.add_space(4.0);
        for direction in SquareDirection::ALL {
            let neighbor = self.model.topology.neighbor(cell, *direction);
            let text = match neighbor.and_then(|id| self.model.topology.coordinate(id)) {
                Some(neighbor_coord) => {
                    let wrap = is_wrapped_neighbor(coord, neighbor_coord, *direction);
                    format!(
                        "{} -> ({}, {}){}",
                        direction_short(*direction),
                        neighbor_coord.x,
                        neighbor_coord.y,
                        if wrap { " · wrap" } else { "" }
                    )
                }
                None => format!("{} -> boundary", direction_short(*direction)),
            };
            ui.colored_label(direction_color(*direction), text);
        }
    }

    fn show_canvas(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let extent = self.model.extent();
                let desired_size = Vec2::new(
                    extent.width as f32 * self.cell_size,
                    extent.height as f32 * self.cell_size,
                );
                let (response, painter) =
                    ui.allocate_painter(desired_size, Sense::click_and_drag());
                let canvas = response.rect;

                if response.hovered()
                    && let Some(pointer) = response.hover_pos()
                    && let Some(coord) = pointer_coordinate(canvas, pointer, self.cell_size, extent)
                {
                    let (paint, erase) = ui.input(|input| {
                        (input.pointer.primary_down(), input.pointer.secondary_down())
                    });
                    if erase {
                        self.model.erase(coord);
                    } else if paint {
                        self.model.paint(coord);
                    }
                }

                self.paint_grid(&painter, canvas);
            });
    }

    fn paint_grid(&self, painter: &egui::Painter, canvas: Rect) {
        for (coord, placement) in self.model.grid.cells() {
            let rect = cell_rect(canvas, coord, self.cell_size).shrink(1.0);
            let fill = placement
                .and_then(|placement| self.model.tiles.get(placement.tile))
                .map_or(Color32::from_gray(32), |tile| tile.payload.color32());
            painter.rect_filled(rect, 3.0, fill);
            painter.rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, Color32::from_gray(80)),
                StrokeKind::Inside,
            );
            painter.text(
                rect.left_top() + Vec2::splat(4.0),
                Align2::LEFT_TOP,
                format!("{},{}", coord.x, coord.y),
                FontId::monospace(9.0),
                Color32::from_white_alpha(150),
            );

            if let Some(placement) = placement
                && let Some(tile) = self.model.tiles.get(placement.tile)
            {
                paint_tile_sockets(painter, rect, tile, placement.transform);
            }
        }

        if let Some(selected) = self.model.selected_cell {
            self.paint_topology_overlay(painter, canvas, selected);
        }
    }

    fn paint_topology_overlay(&self, painter: &egui::Painter, canvas: Rect, selected: Coord2) {
        let selected_rect = cell_rect(canvas, selected, self.cell_size).shrink(2.0);
        painter.rect_stroke(
            selected_rect,
            3.0,
            Stroke::new(3.0, Color32::WHITE),
            StrokeKind::Inside,
        );

        let cell = self
            .model
            .topology
            .cell_at(selected)
            .expect("selected coordinates are in bounds");
        for direction in SquareDirection::ALL {
            let color = direction_color(*direction);
            let Some(neighbor) = self.model.topology.neighbor(cell, *direction) else {
                paint_boundary_marker(painter, selected_rect, *direction, color, "boundary");
                continue;
            };
            let neighbor_coord = self
                .model
                .topology
                .coordinate(neighbor)
                .expect("neighbor IDs belong to this topology");
            let neighbor_rect = cell_rect(canvas, neighbor_coord, self.cell_size).shrink(3.0);
            paint_neighbor_badge(painter, neighbor_rect, *direction, color);

            if is_wrapped_neighbor(selected, neighbor_coord, *direction) {
                paint_boundary_marker(painter, selected_rect, *direction, color, "wrap");
                paint_boundary_marker(
                    painter,
                    neighbor_rect,
                    direction.opposite(),
                    color,
                    direction_short(*direction),
                );
            } else {
                painter.line_segment(
                    [selected_rect.center(), neighbor_rect.center()],
                    Stroke::new(2.0, color),
                );
            }
        }
    }
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

fn boundary_selector(ui: &mut egui::Ui, label: &str, boundary: &mut Boundary) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.selectable_value(boundary, Boundary::Bounded, "Bounded");
        ui.selectable_value(boundary, Boundary::Wrap, "Wrap");
    });
}

fn transform_label(transform: D4) -> String {
    let rotation = match transform.rotation() {
        QuarterTurns::Zero => "0°",
        QuarterTurns::One => "90°",
        QuarterTurns::Two => "180°",
        QuarterTurns::Three => "270°",
    };
    if transform.is_reflected() {
        format!("{rotation}, reflected")
    } else {
        rotation.to_owned()
    }
}

fn direction_short(direction: SquareDirection) -> &'static str {
    match direction {
        SquareDirection::North => "N",
        SquareDirection::East => "E",
        SquareDirection::South => "S",
        SquareDirection::West => "W",
    }
}

fn direction_color(direction: SquareDirection) -> Color32 {
    match direction {
        SquareDirection::North => Color32::from_rgb(80, 170, 255),
        SquareDirection::East => Color32::from_rgb(255, 180, 65),
        SquareDirection::South => Color32::from_rgb(90, 215, 120),
        SquareDirection::West => Color32::from_rgb(230, 100, 160),
    }
}

fn direction_vector(direction: SquareDirection) -> Vec2 {
    match direction {
        SquareDirection::North => Vec2::new(0.0, -1.0),
        SquareDirection::East => Vec2::new(1.0, 0.0),
        SquareDirection::South => Vec2::new(0.0, 1.0),
        SquareDirection::West => Vec2::new(-1.0, 0.0),
    }
}

fn pointer_coordinate(
    canvas: Rect,
    pointer: Pos2,
    cell_size: f32,
    extent: Extent2,
) -> Option<Coord2> {
    let local = pointer - canvas.min;
    let coord = Coord2::new(
        (local.x / cell_size).floor() as i32,
        (local.y / cell_size).floor() as i32,
    );
    extent.contains(coord).then_some(coord)
}

fn cell_rect(canvas: Rect, coord: Coord2, cell_size: f32) -> Rect {
    let min = canvas.min + Vec2::new(coord.x as f32 * cell_size, coord.y as f32 * cell_size);
    Rect::from_min_size(min, Vec2::splat(cell_size))
}

fn is_wrapped_neighbor(source: Coord2, neighbor: Coord2, direction: SquareDirection) -> bool {
    let offset = direction.offset();
    source.x.checked_add(offset.x) != Some(neighbor.x)
        || source.y.checked_add(offset.y) != Some(neighbor.y)
}

fn paint_tile_sockets(
    painter: &egui::Painter,
    rect: Rect,
    tile: &Tile<TileStyle, SquareDirection, bool>,
    transform: D4,
) {
    let center = rect.center();
    for direction in SquareDirection::ALL {
        if !tile.oriented_socket(transform, *direction) {
            continue;
        }
        let vector = direction_vector(*direction);
        let start = center + vector * (rect.width() * 0.12);
        let end = center + vector * (rect.width() * 0.46);
        painter.line_segment([start, end], Stroke::new(5.0, Color32::WHITE));
    }
    painter.circle_filled(center, rect.width() * 0.09, Color32::WHITE);
}

fn paint_neighbor_badge(
    painter: &egui::Painter,
    rect: Rect,
    direction: SquareDirection,
    color: Color32,
) {
    let position = rect.center() + direction_vector(direction) * (rect.width() * 0.2);
    painter.circle_filled(position, 9.0, color);
    painter.text(
        position,
        Align2::CENTER_CENTER,
        direction_short(direction),
        FontId::monospace(9.0),
        Color32::BLACK,
    );
}

fn paint_boundary_marker(
    painter: &egui::Painter,
    rect: Rect,
    direction: SquareDirection,
    color: Color32,
    label: &str,
) {
    let vector = direction_vector(direction);
    let edge = rect.center() + vector * (rect.width() * 0.48);
    let inner = edge - vector * 12.0;
    painter.line_segment([inner, edge], Stroke::new(4.0, color));
    painter.text(
        inner - vector * 3.0,
        match direction {
            SquareDirection::North => Align2::CENTER_BOTTOM,
            SquareDirection::East => Align2::RIGHT_CENTER,
            SquareDirection::South => Align2::CENTER_TOP,
            SquareDirection::West => Align2::LEFT_CENTER,
        },
        label,
        FontId::monospace(8.0),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn painting_and_erasing_use_the_selected_orientation() {
        let mut model = EditorModel::new(Extent2::new(3, 2));
        model.selected_tile = TileId::new(2);
        model.rotate_clockwise();
        model.reflect_horizontally();
        let expected = Placement::new(model.selected_tile, model.selected_transform);

        assert!(model.paint(Coord2::new(1, 1)));
        assert_eq!(model.grid[Coord2::new(1, 1)], Some(expected));
        assert_eq!(model.selected_cell, Some(Coord2::new(1, 1)));

        assert!(model.erase(Coord2::new(1, 1)));
        assert_eq!(model.grid[Coord2::new(1, 1)], None);
        assert!(!model.paint(Coord2::new(3, 1)));
    }

    #[test]
    fn resizing_preserves_overlap_and_drops_outside_selection() {
        let mut model = EditorModel::new(Extent2::new(3, 3));
        model.paint(Coord2::new(0, 0));
        let retained = model.grid[Coord2::new(0, 0)];
        model.paint(Coord2::new(2, 2));

        assert!(model.resize(Extent2::new(2, 2)));
        assert_eq!(model.grid[Coord2::new(0, 0)], retained);
        assert_eq!(model.grid.len(), 4);
        assert_eq!(model.selected_cell, None);
        assert!(!model.resize(Extent2::new(0, 2)));
    }

    #[test]
    fn changing_boundaries_keeps_grid_contents_and_changes_neighbors() {
        let mut model = EditorModel::new(Extent2::new(3, 2));
        model.paint(Coord2::new(0, 0));
        let placement = model.grid[Coord2::new(0, 0)];
        let left = model.topology.cell_at(Coord2::new(0, 0)).unwrap();
        assert_eq!(model.topology.neighbor(left, SquareDirection::West), None);

        model.set_boundaries(AxisBoundaries::new(Boundary::Wrap, Boundary::Bounded));

        let neighbor = model
            .topology
            .neighbor(left, SquareDirection::West)
            .and_then(|cell| model.topology.coordinate(cell));
        assert_eq!(neighbor, Some(Coord2::new(2, 0)));
        assert_eq!(model.grid[Coord2::new(0, 0)], placement);
    }

    #[test]
    fn one_cell_torus_reports_four_wrapped_self_neighbors() {
        let mut model = EditorModel::new(Extent2::new(1, 1));
        model.set_boundaries(AxisBoundaries::TOROIDAL);
        let only = model.topology.cell_at(Coord2::ZERO).unwrap();

        for direction in SquareDirection::ALL {
            let neighbor = model.topology.neighbor(only, *direction).unwrap();
            let coord = model.topology.coordinate(neighbor).unwrap();
            assert_eq!(coord, Coord2::ZERO);
            assert!(is_wrapped_neighbor(Coord2::ZERO, coord, *direction));
        }
    }

    #[test]
    fn demo_tiles_expose_oriented_sockets() {
        let model = EditorModel::default();
        let corner = model.tiles.get(TileId::new(2)).unwrap();
        let clockwise = D4::new(QuarterTurns::One, false);

        assert!(*corner.oriented_socket(clockwise, SquareDirection::East));
        assert!(*corner.oriented_socket(clockwise, SquareDirection::South));
        assert!(!*corner.oriented_socket(clockwise, SquareDirection::North));
    }
}
