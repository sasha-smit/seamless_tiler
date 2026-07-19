use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, FontId, PointerButton, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    Vec2,
};
use seamless_tiler::{
    AxisBoundaries, Boundary, Coord2, D4, Direction, Extent2, QuarterTurns, SquareDirection, Tile,
    Topology, WfcStatus,
};

use crate::model::{CanvasTool, CellVisual, DEFAULT_EXTENT, EditorModel, MAX_DIMENSION, TileStyle};

const DEFAULT_CELL_SIZE: f32 = 52.0;
const DEFAULT_STEPS_PER_SECOND: f32 = 8.0;

pub struct TilerApp {
    model: EditorModel,
    width_input: usize,
    height_input: usize,
    cell_size: f32,
    steps_per_second: f32,
    last_frame_time: f64,
    step_accumulator: f64,
    notice: Option<String>,
}

impl Default for TilerApp {
    fn default() -> Self {
        Self {
            model: EditorModel::default(),
            width_input: DEFAULT_EXTENT.width,
            height_input: DEFAULT_EXTENT.height,
            cell_size: DEFAULT_CELL_SIZE,
            steps_per_second: DEFAULT_STEPS_PER_SECOND,
            last_frame_time: 0.0,
            step_accumulator: 0.0,
            notice: None,
        }
    }
}

impl eframe::App for TilerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.advance_playback(ui);

        egui::Panel::top("instructions").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("WFC Tiler");
                ui.separator();
                ui.label(
                    "Choose allowed orientations, pin exact tiles where needed, then Step or Run",
                );
                ui.separator();
                ui.weak("Right-drag always unpins");
            });
        });

        egui::Panel::left("controls")
            .default_size(300.0)
            .resizable(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.show_controls(ui));
            });

        egui::CentralPanel::default().show(ui, |ui| self.show_canvas(ui));
    }
}

impl TilerApp {
    fn advance_playback(&mut self, ui: &egui::Ui) {
        let now = ui.input(|input| input.time);
        let elapsed = (now - self.last_frame_time).clamp(0.0, 0.25);
        self.last_frame_time = now;

        if !self.model.running() {
            self.step_accumulator = 0.0;
            return;
        }
        self.step_accumulator += elapsed * f64::from(self.steps_per_second);
        let steps = self.step_accumulator.floor().min(64.0) as usize;
        self.step_accumulator -= steps as f64;
        for _ in 0..steps {
            if !self.model.step() {
                break;
            }
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }

    fn show_controls(&mut self, ui: &mut egui::Ui) {
        self.show_grid_controls(ui);
        ui.separator();
        self.show_topology_controls(ui);
        ui.separator();
        self.show_solver_controls(ui);
        ui.separator();
        self.show_variant_palette(ui);
        ui.separator();
        self.show_inspector(ui);
    }

    fn show_grid_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Grid");
        egui::Grid::new("dimensions").show(ui, |ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut self.width_input).range(1..=MAX_DIMENSION));
            ui.end_row();
            ui.label("Height");
            ui.add(egui::DragValue::new(&mut self.height_input).range(1..=MAX_DIMENSION));
            ui.end_row();
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Apply size").clicked() {
                self.model
                    .resize(Extent2::new(self.width_input, self.height_input));
            }
            if ui.button("Clear pins").clicked() {
                let cleared = self.model.clear_pins();
                self.notice = Some(format!("Cleared {cleared} pin(s)"));
            }
            if ui.button("Reset all").clicked() {
                self.model.reset_all();
                self.width_input = DEFAULT_EXTENT.width;
                self.height_input = DEFAULT_EXTENT.height;
                self.cell_size = DEFAULT_CELL_SIZE;
                self.steps_per_second = DEFAULT_STEPS_PER_SECOND;
                self.notice = Some("Restored defaults".to_owned());
            }
        });
        ui.add(egui::Slider::new(&mut self.cell_size, 28.0..=80.0).text("Cell size"));
    }

    fn show_topology_controls(&mut self, ui: &mut egui::Ui) {
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
        ui.weak("Bounded edges close outward-facing path sockets.");
    }

    fn show_solver_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Wave Function Collapse");
        let mut seed = self.model.seed();
        ui.horizontal(|ui| {
            ui.label("Seed");
            if ui.add(egui::DragValue::new(&mut seed).speed(1.0)).changed() {
                self.model.set_seed(seed);
            }
        });

        let running = matches!(self.model.status(), Some(WfcStatus::Running));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Reset").clicked() {
                self.model.reset_wave();
            }
            if ui.add_enabled(running, egui::Button::new("Step")).clicked() {
                self.model.step();
            }
            let run_label = if self.model.running() { "Pause" } else { "Run" };
            if ui
                .add_enabled(running, egui::Button::new(run_label))
                .clicked()
            {
                self.model.toggle_running();
            }
            if ui
                .add_enabled(running, egui::Button::new("Finish"))
                .clicked()
            {
                self.model.finish();
            }
            let can_retry = matches!(self.model.status(), Some(WfcStatus::Contradiction { .. }))
                && !self.model.initial_contradiction();
            if ui
                .add_enabled(can_retry, egui::Button::new("Retry seed +1"))
                .clicked()
            {
                self.model.retry();
            }
        });
        ui.add(
            egui::Slider::new(&mut self.steps_per_second, 1.0..=60.0)
                .logarithmic(true)
                .text("Steps / second"),
        );

        self.show_status(ui);
        if let Some(notice) = &self.notice {
            ui.weak(notice);
        }
    }

    fn show_status(&self, ui: &mut egui::Ui) {
        let (color, text) = match self.model.status() {
            None => (
                Color32::from_rgb(240, 180, 70),
                "Enable at least one tile orientation".to_owned(),
            ),
            Some(WfcStatus::Running) => (
                Color32::from_rgb(100, 190, 255),
                format!(
                    "{} unresolved · {} observations",
                    self.model.unresolved_count(),
                    self.model.observations()
                ),
            ),
            Some(WfcStatus::Solved) => (
                Color32::from_rgb(100, 220, 130),
                format!("Solved in {} observations", self.model.observations()),
            ),
            Some(WfcStatus::Contradiction { cell }) if self.model.initial_contradiction() => (
                Color32::from_rgb(255, 105, 105),
                format!(
                    "Constraints conflict at cell {} · change pins or allowed tiles",
                    cell.index()
                ),
            ),
            Some(WfcStatus::Contradiction { cell }) => (
                Color32::from_rgb(255, 105, 105),
                format!("Contradiction at cell {} · retry or reset", cell.index()),
            ),
        };
        ui.colored_label(color, text);
    }

    fn show_variant_palette(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tile orientations");
        ui.horizontal_wrapped(|ui| {
            let inspect = self.model.tool() == CanvasTool::Inspect;
            if ui.selectable_label(inspect, "Inspect").clicked() {
                self.model.set_tool(CanvasTool::Inspect);
            }
            let unpin = self.model.tool() == CanvasTool::Unpin;
            if ui.selectable_label(unpin, "Unpin").clicked() {
                self.model.set_tool(CanvasTool::Unpin);
            }
        });
        ui.weak("Checkbox: allowed globally · Preview: exact pin brush");

        let groups: Vec<_> = self
            .model
            .tiles()
            .iter()
            .map(|(tile_id, tile)| {
                (
                    tile_id,
                    tile.payload,
                    self.model.variants_for_tile(tile_id).collect::<Vec<_>>(),
                )
            })
            .collect();
        for (_tile_id, style, variant_indices) in groups {
            ui.add_space(5.0);
            ui.label(RichText::new(style.name).strong().color(style_color(style)));
            ui.horizontal_wrapped(|ui| {
                for variant_index in variant_indices {
                    let mut enabled = self.model.variant_enabled(variant_index);
                    ui.vertical(|ui| {
                        if ui.checkbox(&mut enabled, "Allowed").changed() {
                            let cleared = self.model.set_variant_enabled(variant_index, enabled);
                            if cleared > 0 {
                                self.notice = Some(format!(
                                    "Disabled orientation and cleared {cleared} matching pin(s)"
                                ));
                            }
                        }
                        let response = self.paint_variant_preview(ui, variant_index, enabled);
                        if response.clicked() && enabled {
                            self.model.set_tool(CanvasTool::Pin(variant_index));
                        }
                        let transform = self.model.variants()[variant_index].placement.transform;
                        ui.label(RichText::new(transform_label(transform)).small());
                    });
                }
            });
        }
        ui.weak(format!(
            "{} distinct orientations enabled",
            self.model.enabled_variant_count()
        ));
    }

    fn paint_variant_preview(
        &self,
        ui: &mut egui::Ui,
        variant_index: usize,
        enabled: bool,
    ) -> egui::Response {
        let variant = self.model.variants()[variant_index];
        let tile = self
            .model
            .tiles()
            .get(variant.placement.tile)
            .expect("variant refers to a demo tile");
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::click());
        let selected = self.model.tool() == CanvasTool::Pin(variant_index);
        let fill = if enabled {
            style_color(tile.payload)
        } else {
            Color32::from_gray(42)
        };
        ui.painter().rect_filled(rect, 5.0, fill);
        ui.painter().rect_stroke(
            rect,
            5.0,
            Stroke::new(
                if selected { 3.0 } else { 1.0 },
                if selected {
                    Color32::WHITE
                } else {
                    Color32::from_gray(100)
                },
            ),
            StrokeKind::Inside,
        );
        if enabled {
            paint_tile_sockets(
                ui.painter(),
                rect.shrink(2.0),
                tile,
                variant.placement.transform,
            );
        } else {
            ui.painter().line_segment(
                [rect.left_top(), rect.right_bottom()],
                Stroke::new(2.0, Color32::from_rgb(190, 90, 90)),
            );
        }
        response.on_hover_text(if enabled {
            format!(
                "Select {} {} as the pin brush",
                tile.payload.name,
                transform_label(variant.placement.transform)
            )
        } else {
            "Enable this orientation before pinning it".to_owned()
        })
    }

    fn show_inspector(&self, ui: &mut egui::Ui) {
        ui.heading("Inspector");
        let Some(coord) = self.model.selected_cell() else {
            ui.label("Inspect or edit a cell to see its current wave domain.");
            return;
        };
        let cell = self
            .model
            .topology()
            .cell_at(coord)
            .expect("selected coordinates are in bounds");
        ui.monospace(format!(
            "Cell ({}, {}) · id {}",
            coord.x,
            coord.y,
            cell.index()
        ));

        if let Some(pin) = self.model.pin_at(coord) {
            let tile = self
                .model
                .tiles()
                .get(pin.tile)
                .expect("pins refer to demo tiles");
            ui.colored_label(
                Color32::from_rgb(255, 205, 80),
                format!(
                    "Pinned: {} · {}",
                    tile.payload.name,
                    transform_label(pin.transform)
                ),
            );
        } else {
            ui.weak("Not pinned");
        }

        match self.model.cell_visual(coord) {
            CellVisual::Unavailable => {
                ui.weak("No candidates: enable at least one orientation.");
            }
            CellVisual::Contradiction => {
                ui.colored_label(
                    Color32::from_rgb(255, 105, 105),
                    "Contradiction: 0 candidates",
                );
            }
            CellVisual::Collapsed { variant, .. } => {
                let placement = self.model.variants()[variant].placement;
                let tile = self
                    .model
                    .tiles()
                    .get(placement.tile)
                    .expect("variant refers to a demo tile");
                ui.label(format!(
                    "Collapsed: {} · {}",
                    tile.payload.name,
                    transform_label(placement.transform)
                ));
            }
            CellVisual::Superposition {
                candidates,
                entropy,
            } => {
                ui.label(format!("{candidates} candidates · entropy {entropy:.3}"));
                ui.horizontal_wrapped(|ui| {
                    for variant_index in self.model.candidate_variants(coord) {
                        let placement = self.model.variants()[variant_index].placement;
                        let tile = self
                            .model
                            .tiles()
                            .get(placement.tile)
                            .expect("variant refers to a demo tile");
                        ui.colored_label(
                            style_color(tile.payload),
                            format!(
                                "{} {}",
                                tile.payload.name,
                                transform_label(placement.transform)
                            ),
                        );
                    }
                });
            }
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
                    let (primary, secondary) = ui.input(|input| {
                        (
                            input.pointer.button_down(PointerButton::Primary),
                            input.pointer.button_down(PointerButton::Secondary),
                        )
                    });
                    if secondary {
                        self.model.apply_tool(coord, true);
                    } else if primary {
                        self.model.apply_tool(coord, false);
                    }
                }

                self.paint_grid(&painter, canvas);
            });
    }

    fn paint_grid(&self, painter: &egui::Painter, canvas: Rect) {
        let total_candidates = self.model.enabled_variant_count().max(1);
        for index in 0..self.model.topology().cell_count() {
            let cell = seamless_tiler::CellId::new(index);
            let coord = self
                .model
                .topology()
                .coordinate(cell)
                .expect("topology cells have coordinates");
            let rect = cell_rect(canvas, coord, self.cell_size).shrink(1.0);
            let visual = self.model.cell_visual(coord);
            let fill = match visual {
                CellVisual::Unavailable => Color32::from_gray(28),
                CellVisual::Contradiction => Color32::from_rgb(135, 35, 45),
                CellVisual::Superposition { candidates, .. } => {
                    uncertainty_color(candidates, total_candidates)
                }
                CellVisual::Collapsed { variant, .. } => {
                    let tile = self
                        .model
                        .tiles()
                        .get(self.model.variants()[variant].placement.tile)
                        .expect("variant refers to a demo tile");
                    style_color(tile.payload)
                }
            };
            painter.rect_filled(rect, 3.0, fill);
            painter.rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, Color32::from_gray(82)),
                StrokeKind::Inside,
            );

            match visual {
                CellVisual::Contradiction => {
                    painter.line_segment(
                        [
                            rect.left_top() + Vec2::splat(7.0),
                            rect.right_bottom() - Vec2::splat(7.0),
                        ],
                        Stroke::new(3.0, Color32::from_rgb(255, 150, 150)),
                    );
                    painter.line_segment(
                        [
                            Pos2::new(rect.right() - 7.0, rect.top() + 7.0),
                            Pos2::new(rect.left() + 7.0, rect.bottom() - 7.0),
                        ],
                        Stroke::new(3.0, Color32::from_rgb(255, 150, 150)),
                    );
                }
                CellVisual::Superposition { candidates, .. } => {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        candidates,
                        FontId::monospace((self.cell_size * 0.25).clamp(10.0, 18.0)),
                        Color32::from_white_alpha(220),
                    );
                }
                CellVisual::Collapsed { variant, pinned } => {
                    let placement = self.model.variants()[variant].placement;
                    let tile = self
                        .model
                        .tiles()
                        .get(placement.tile)
                        .expect("variant refers to a demo tile");
                    paint_tile_sockets(painter, rect, tile, placement.transform);
                    if pinned {
                        painter.circle_filled(
                            rect.right_top() + Vec2::new(-8.0, 8.0),
                            7.0,
                            Color32::from_rgb(255, 205, 80),
                        );
                        painter.text(
                            rect.right_top() + Vec2::new(-8.0, 8.0),
                            Align2::CENTER_CENTER,
                            "P",
                            FontId::monospace(8.0),
                            Color32::BLACK,
                        );
                    }
                }
                CellVisual::Unavailable => {}
            }

            if self.model.last_observed() == Some(coord) {
                painter.rect_stroke(
                    rect.shrink(2.0),
                    3.0,
                    Stroke::new(2.0, Color32::from_rgb(90, 220, 255)),
                    StrokeKind::Inside,
                );
            }
        }

        if let Some(selected) = self.model.selected_cell() {
            painter.rect_stroke(
                cell_rect(canvas, selected, self.cell_size).shrink(2.0),
                3.0,
                Stroke::new(3.0, Color32::WHITE),
                StrokeKind::Inside,
            );
        }
    }
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
        format!("{rotation} reflected")
    } else {
        rotation.to_owned()
    }
}

fn style_color(style: TileStyle) -> Color32 {
    Color32::from_rgb(style.color[0], style.color[1], style.color[2])
}

fn uncertainty_color(candidates: usize, total: usize) -> Color32 {
    let t = if total <= 1 {
        0.0
    } else {
        (candidates.saturating_sub(1) as f32 / (total - 1) as f32).clamp(0.0, 1.0)
    };
    let low = [105.0, 72.0, 42.0];
    let high = [38.0, 48.0, 76.0];
    Color32::from_rgb(
        (low[0] + (high[0] - low[0]) * t) as u8,
        (low[1] + (high[1] - low[1]) * t) as u8,
        (low[2] + (high[2] - low[2]) * t) as u8,
    )
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

fn direction_vector(direction: SquareDirection) -> Vec2 {
    match direction {
        SquareDirection::North => Vec2::new(0.0, -1.0),
        SquareDirection::East => Vec2::new(1.0, 0.0),
        SquareDirection::South => Vec2::new(0.0, 1.0),
        SquareDirection::West => Vec2::new(-1.0, 0.0),
    }
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
        painter.line_segment(
            [start, end],
            Stroke::new((rect.width() * 0.1).clamp(3.0, 6.0), Color32::WHITE),
        );
    }
    painter.circle_filled(center, rect.width() * 0.09, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_coordinates_respect_canvas_bounds() {
        let canvas = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(100.0, 50.0));
        let extent = Extent2::new(2, 1);
        assert_eq!(
            pointer_coordinate(canvas, Pos2::new(75.0, 30.0), 50.0, extent),
            Some(Coord2::new(1, 0))
        );
        assert_eq!(
            pointer_coordinate(canvas, Pos2::new(110.0, 30.0), 50.0, extent),
            None
        );
    }

    #[test]
    fn uncertainty_colors_distinguish_small_and_large_domains() {
        assert_ne!(uncertainty_color(2, 12), uncertainty_color(12, 12));
    }

    #[test]
    fn transform_labels_include_reflection() {
        assert_eq!(transform_label(D4::IDENTITY), "0°");
        assert_eq!(
            transform_label(D4::new(QuarterTurns::One, true)),
            "90° reflected"
        );
    }
}
