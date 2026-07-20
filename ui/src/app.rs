use std::f32::consts::PI;
use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, PointerButton, Pos2, Rect, RichText, Sense, Shape,
    Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use seamless_tiler::{
    AxisBoundaries, Boundary, CellId, Coord2, D4, Extent2, QuarterTurns, SixthTurns, TileId,
    WfcStatus,
};

use crate::model::{
    CanvasTool, CellVisual, DEFAULT_EXTENT, EditorModel, GridMode, MAX_DIMENSION, Orientation,
};

const DEFAULT_CELL_SIZE: f32 = 52.0;
const DEFAULT_STEPS_PER_SECOND: f32 = 8.0;
const SQRT_3: f32 = 1.732_050_8;
const RASTER_SIZE: usize = 32;

pub struct TilerApp {
    model: EditorModel,
    dimension_inputs: [Extent2; 2],
    cell_size: f32,
    steps_per_second: f32,
    last_frame_time: f64,
    step_accumulator: f64,
    notice: Option<String>,
    raster_cache: Option<RasterCache>,
}

/// Per-variant textures for the active square catalog, rebuilt when the mode or
/// catalog version changes. Empty in hex mode, which renders flat fill + stubs.
struct RasterCache {
    mode: GridMode,
    version: u64,
    textures: Vec<Option<TextureHandle>>,
}

impl Default for TilerApp {
    fn default() -> Self {
        Self {
            model: EditorModel::default(),
            dimension_inputs: [DEFAULT_EXTENT; 2],
            cell_size: DEFAULT_CELL_SIZE,
            steps_per_second: DEFAULT_STEPS_PER_SECOND,
            last_frame_time: 0.0,
            step_accumulator: 0.0,
            notice: None,
            raster_cache: None,
        }
    }
}

impl eframe::App for TilerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.advance_playback(ui);
        self.ensure_rasters(ui.ctx());

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

    fn ensure_rasters(&mut self, ctx: &egui::Context) {
        let mode = self.model.mode();
        let version = self.model.catalog_version();
        if self
            .raster_cache
            .as_ref()
            .is_some_and(|cache| cache.mode == mode && cache.version == version)
        {
            return;
        }
        let textures = if mode == GridMode::Square {
            (0..self.model.variant_count())
                .map(|index| self.build_variant_texture(ctx, index))
                .collect()
        } else {
            Vec::new()
        };
        self.raster_cache = Some(RasterCache {
            mode,
            version,
            textures,
        });
    }

    fn build_variant_texture(&self, ctx: &egui::Context, index: usize) -> Option<TextureHandle> {
        let variant = self.model.variant(index)?;
        let Orientation::Square(transform) = variant.orientation else {
            return None;
        };
        let color = self.model.tile_style(variant.tile)?.color;
        let base = rasterize_tile(&self.model.tile_sockets(variant.tile), color);
        let raster = transform_raster(&base, transform);
        let bytes: Vec<u8> = raster.pixels.iter().flatten().copied().collect();
        let image = ColorImage::from_rgba_unmultiplied([raster.size, raster.size], &bytes);
        Some(ctx.load_texture(format!("variant-{index}"), image, TextureOptions::NEAREST))
    }

    /// The cached texture for a variant in the active square catalog, if any.
    fn variant_texture(&self, index: usize) -> Option<&TextureHandle> {
        self.raster_cache
            .as_ref()
            .filter(|cache| cache.mode == self.model.mode())
            .and_then(|cache| cache.textures.get(index))
            .and_then(Option::as_ref)
    }

    /// Draws a variant's raster into `rect`, returning whether a texture existed.
    fn paint_variant_texture(&self, painter: &egui::Painter, rect: Rect, index: usize) -> bool {
        let Some(texture) = self.variant_texture(index) else {
            return false;
        };
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        true
    }

    fn show_controls(&mut self, ui: &mut egui::Ui) {
        self.show_grid_controls(ui);
        ui.separator();
        self.show_topology_controls(ui);
        ui.separator();
        self.show_solver_controls(ui);
        ui.separator();
        self.show_tile_catalog(ui);
        ui.separator();
        self.show_variant_palette(ui);
        ui.separator();
        self.show_inspector(ui);
    }

    fn show_grid_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Grid");
        let mut mode = self.model.mode();
        ui.horizontal(|ui| {
            ui.label("Cells");
            for candidate in GridMode::ALL {
                ui.selectable_value(&mut mode, candidate, candidate.label());
            }
        });
        if self.model.set_mode(mode) {
            self.dimension_inputs[mode.index()] = self.model.extent();
            self.step_accumulator = 0.0;
            self.last_frame_time = ui.input(|input| input.time);
            self.notice = Some(format!("Switched to {} grid", mode.label()));
        }

        let dimensions = &mut self.dimension_inputs[mode.index()];
        egui::Grid::new("dimensions").show(ui, |ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut dimensions.width).range(1..=MAX_DIMENSION));
            ui.end_row();
            ui.label("Height");
            ui.add(egui::DragValue::new(&mut dimensions.height).range(1..=MAX_DIMENSION));
            ui.end_row();
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Apply size").clicked() {
                self.model.resize(*dimensions);
            }
            if ui.button("Clear pins").clicked() {
                let cleared = self.model.clear_pins();
                self.notice = Some(format!("Cleared {cleared} pin(s)"));
            }
            if ui.button("Reset mode").clicked() {
                self.model.reset_active();
                *dimensions = DEFAULT_EXTENT;
                self.notice = Some(format!("Reset {} mode", mode.label()));
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

    fn show_tile_catalog(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tile catalog");
        ui.weak("Edit name, color, and edge sockets. Orientations derive below.");
        let labels = direction_labels(self.model.mode());
        let mut pending_delete: Option<(TileId, String)> = None;
        for (tile_id, style) in self.model.tiles() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let mut color = style.color;
                if ui.color_edit_button_srgb(&mut color).changed() {
                    self.model.set_tile_color(tile_id, color);
                }
                let mut name = style.name.clone();
                if ui
                    .add(egui::TextEdit::singleline(&mut name).desired_width(120.0))
                    .changed()
                {
                    self.model.set_tile_name(tile_id, name);
                }
                if ui.button("Delete").clicked() {
                    pending_delete = Some((tile_id, style.name.clone()));
                }
            });
            ui.horizontal_wrapped(|ui| {
                let sockets = self.model.tile_sockets(tile_id);
                for (index, (label, value)) in labels.iter().zip(sockets.iter()).enumerate() {
                    let mut socket = *value;
                    if ui.checkbox(&mut socket, *label).changed() {
                        self.model.set_tile_socket(tile_id, index, socket);
                    }
                }
            });
        }
        if let Some((tile_id, name)) = pending_delete {
            self.model.remove_tile(tile_id);
            self.notice = Some(format!("Deleted {name}"));
        }
        ui.add_space(4.0);
        if ui.button("Add tile").clicked() {
            self.model.add_tile();
            self.notice = Some("Added a tile".to_owned());
        }
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
            .into_iter()
            .map(|(tile_id, style)| (tile_id, style, self.model.variants_for_tile(tile_id)))
            .collect();
        for (_tile_id, style, variant_indices) in groups {
            ui.add_space(5.0);
            ui.label(
                RichText::new(style.name.clone())
                    .strong()
                    .color(style_color(style.color)),
            );
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
                        let orientation = self
                            .model
                            .variant(variant_index)
                            .expect("palette contains catalog variants")
                            .orientation;
                        ui.label(RichText::new(orientation_label(orientation)).small());
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
        let variant = self
            .model
            .variant(variant_index)
            .expect("preview contains a catalog variant");
        let style = self
            .model
            .tile_style(variant.tile)
            .expect("variant refers to a demo tile");
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::click());
        let selected = self.model.tool() == CanvasTool::Pin(variant_index);
        let fill = if enabled {
            style_color(style.color)
        } else {
            Color32::from_gray(42)
        };
        paint_preview_cell(
            ui.painter(),
            rect,
            self.model.mode(),
            fill,
            Stroke::new(
                if selected { 3.0 } else { 1.0 },
                if selected {
                    Color32::WHITE
                } else {
                    Color32::from_gray(100)
                },
            ),
        );
        if enabled {
            let drew_raster = self.model.mode() == GridMode::Square
                && self.paint_variant_texture(ui.painter(), rect.shrink(4.0), variant_index);
            if !drew_raster {
                paint_sockets(
                    ui.painter(),
                    rect.center(),
                    21.0,
                    self.model.mode(),
                    self.model.variant_sockets(variant_index),
                );
            }
        } else {
            ui.painter().line_segment(
                [rect.left_top(), rect.right_bottom()],
                Stroke::new(2.0, Color32::from_rgb(190, 90, 90)),
            );
        }
        response.on_hover_text(if enabled {
            format!(
                "Select {} {} as the pin brush",
                style.name,
                orientation_label(variant.orientation)
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
            .cell_at(coord)
            .expect("selected coordinates are in bounds");
        ui.monospace(format!(
            "Cell ({}, {}) · id {}",
            coord.x,
            coord.y,
            cell.index()
        ));

        if let Some(variant_index) = self.model.pin_variant_at(coord) {
            let variant = self
                .model
                .variant(variant_index)
                .expect("pins refer to catalog variants");
            let style = self
                .model
                .tile_style(variant.tile)
                .expect("pins refer to demo tiles");
            ui.colored_label(
                Color32::from_rgb(255, 205, 80),
                format!(
                    "Pinned: {} · {}",
                    style.name,
                    orientation_label(variant.orientation)
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
                let view = self
                    .model
                    .variant(variant)
                    .expect("wave refers to a catalog variant");
                let style = self
                    .model
                    .tile_style(view.tile)
                    .expect("variant refers to a demo tile");
                ui.label(format!(
                    "Collapsed: {} · {}",
                    style.name,
                    orientation_label(view.orientation)
                ));
                if self.model.mode() == GridMode::Square {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    self.paint_variant_texture(ui.painter(), rect, variant);
                }
            }
            CellVisual::Superposition {
                candidates,
                entropy,
            } => {
                ui.label(format!("{candidates} candidates · entropy {entropy:.3}"));
                ui.horizontal_wrapped(|ui| {
                    for variant_index in self.model.candidate_variants(coord) {
                        let variant = self
                            .model
                            .variant(variant_index)
                            .expect("wave refers to a catalog variant");
                        let style = self
                            .model
                            .tile_style(variant.tile)
                            .expect("variant refers to a demo tile");
                        ui.colored_label(
                            style_color(style.color),
                            format!("{} {}", style.name, orientation_label(variant.orientation)),
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
                let mode = self.model.mode();
                let desired_size = canvas_size(mode, extent, self.cell_size);
                let (response, painter) =
                    ui.allocate_painter(desired_size, Sense::click_and_drag());
                let canvas = response.rect;

                if response.hovered()
                    && let Some(pointer) = response.hover_pos()
                    && let Some(coord) =
                        pointer_coordinate(mode, canvas, pointer, self.cell_size, extent)
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
        let mode = self.model.mode();
        let total_candidates = self.model.enabled_variant_count().max(1);
        for index in 0..self.model.cell_count() {
            let coord = self
                .model
                .coordinate(CellId::new(index))
                .expect("topology cells have coordinates");
            let visual = self.model.cell_visual(coord);
            let fill = match visual {
                CellVisual::Unavailable => Color32::from_gray(28),
                CellVisual::Contradiction => Color32::from_rgb(135, 35, 45),
                CellVisual::Superposition { candidates, .. } => {
                    uncertainty_color(candidates, total_candidates)
                }
                CellVisual::Collapsed { variant, .. } => {
                    let variant = self
                        .model
                        .variant(variant)
                        .expect("wave refers to a catalog variant");
                    style_color(
                        self.model
                            .tile_style(variant.tile)
                            .expect("variant refers to a demo tile")
                            .color,
                    )
                }
            };
            paint_cell(
                painter,
                mode,
                canvas,
                coord,
                self.cell_size,
                fill,
                Stroke::new(1.0, Color32::from_gray(82)),
            );

            let center = cell_center(mode, canvas, coord, self.cell_size);
            match visual {
                CellVisual::Contradiction => {
                    let radius = self.cell_size * 0.5 * 0.55;
                    painter.line_segment(
                        [
                            center + Vec2::new(-radius, -radius),
                            center + Vec2::new(radius, radius),
                        ],
                        Stroke::new(3.0, Color32::from_rgb(255, 150, 150)),
                    );
                    painter.line_segment(
                        [
                            center + Vec2::new(radius, -radius),
                            center + Vec2::new(-radius, radius),
                        ],
                        Stroke::new(3.0, Color32::from_rgb(255, 150, 150)),
                    );
                }
                CellVisual::Superposition { candidates, .. } => {
                    painter.text(
                        center,
                        Align2::CENTER_CENTER,
                        candidates,
                        FontId::monospace((self.cell_size * 0.25).clamp(10.0, 18.0)),
                        Color32::from_white_alpha(220),
                    );
                }
                CellVisual::Collapsed { variant, pinned } => {
                    let drew_raster = mode == GridMode::Square && {
                        let rect = square_rect(canvas, coord, self.cell_size).shrink(1.0);
                        self.paint_variant_texture(painter, rect, variant)
                    };
                    if !drew_raster {
                        paint_sockets(
                            painter,
                            center,
                            self.cell_size * 0.5,
                            mode,
                            self.model.variant_sockets(variant),
                        );
                    }
                    if pinned {
                        let badge = center
                            + match mode {
                                GridMode::Square => Vec2::new(0.35, -0.35) * self.cell_size,
                                GridMode::Hex => Vec2::new(0.28, -0.30) * self.cell_size,
                            };
                        painter.circle_filled(badge, 7.0, Color32::from_rgb(255, 205, 80));
                        painter.text(
                            badge,
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
                paint_cell_outline(
                    painter,
                    mode,
                    canvas,
                    coord,
                    self.cell_size,
                    3.0,
                    Stroke::new(2.0, Color32::from_rgb(90, 220, 255)),
                );
            }
        }

        if let Some(selected) = self.model.selected_cell() {
            paint_cell_outline(
                painter,
                mode,
                canvas,
                selected,
                self.cell_size,
                3.0,
                Stroke::new(3.0, Color32::WHITE),
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

fn orientation_label(orientation: Orientation) -> String {
    let (degrees, reflected) = match orientation {
        Orientation::Square(transform) => (
            match transform.rotation() {
                QuarterTurns::Zero => 0,
                QuarterTurns::One => 90,
                QuarterTurns::Two => 180,
                QuarterTurns::Three => 270,
            },
            transform.is_reflected(),
        ),
        Orientation::Hex(transform) => (
            match transform.rotation() {
                SixthTurns::Zero => 0,
                SixthTurns::One => 60,
                SixthTurns::Two => 120,
                SixthTurns::Three => 180,
                SixthTurns::Four => 240,
                SixthTurns::Five => 300,
            },
            transform.is_reflected(),
        ),
    };
    if reflected {
        format!("{degrees}° reflected")
    } else {
        format!("{degrees}°")
    }
}

fn direction_labels(mode: GridMode) -> &'static [&'static str] {
    match mode {
        GridMode::Square => &["N", "E", "S", "W"],
        GridMode::Hex => &["NE", "E", "SE", "SW", "W", "NW"],
    }
}

fn style_color(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
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

fn canvas_size(mode: GridMode, extent: Extent2, cell_size: f32) -> Vec2 {
    match mode {
        GridMode::Square => Vec2::new(
            extent.width as f32 * cell_size,
            extent.height as f32 * cell_size,
        ),
        GridMode::Hex => {
            let width = hex_width(cell_size);
            Vec2::new(
                extent.width as f32 * width + if extent.height > 1 { width * 0.5 } else { 0.0 },
                cell_size + extent.height.saturating_sub(1) as f32 * cell_size * 0.75,
            )
        }
    }
}

fn pointer_coordinate(
    mode: GridMode,
    canvas: Rect,
    pointer: Pos2,
    cell_size: f32,
    extent: Extent2,
) -> Option<Coord2> {
    match mode {
        GridMode::Square => {
            let local = pointer - canvas.min;
            let coord = Coord2::new(
                (local.x / cell_size).floor() as i32,
                (local.y / cell_size).floor() as i32,
            );
            extent.contains(coord).then_some(coord)
        }
        GridMode::Hex => {
            for y in 0..extent.height {
                for x in 0..extent.width {
                    let coord = Coord2::new(x as i32, y as i32);
                    if point_in_convex_polygon(pointer, &hex_points(canvas, coord, cell_size, 0.0))
                    {
                        return Some(coord);
                    }
                }
            }
            None
        }
    }
}

fn point_in_convex_polygon(point: Pos2, polygon: &[Pos2]) -> bool {
    let mut sign = 0.0_f32;
    for index in 0..polygon.len() {
        let start = polygon[index];
        let end = polygon[(index + 1) % polygon.len()];
        let cross =
            (end.x - start.x) * (point.y - start.y) - (end.y - start.y) * (point.x - start.x);
        if cross.abs() <= f32::EPSILON {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

fn cell_center(mode: GridMode, canvas: Rect, coord: Coord2, cell_size: f32) -> Pos2 {
    match mode {
        GridMode::Square => {
            canvas.min
                + Vec2::new(
                    (coord.x as f32 + 0.5) * cell_size,
                    (coord.y as f32 + 0.5) * cell_size,
                )
        }
        GridMode::Hex => {
            let width = hex_width(cell_size);
            canvas.min
                + Vec2::new(
                    width * (coord.x as f32 + 0.5 + 0.5 * (coord.y & 1) as f32),
                    cell_size * (0.5 + coord.y as f32 * 0.75),
                )
        }
    }
}

fn hex_width(cell_size: f32) -> f32 {
    cell_size * SQRT_3 * 0.5
}

fn hex_points(canvas: Rect, coord: Coord2, cell_size: f32, inset: f32) -> [Pos2; 6] {
    let center = cell_center(GridMode::Hex, canvas, coord, cell_size);
    regular_hex_points(center, cell_size * 0.5 - inset)
}

fn regular_hex_points(center: Pos2, radius: f32) -> [Pos2; 6] {
    std::array::from_fn(|index| {
        let angle = -PI * 0.5 + index as f32 * PI / 3.0;
        center + Vec2::new(angle.cos(), angle.sin()) * radius
    })
}

fn square_rect(canvas: Rect, coord: Coord2, cell_size: f32) -> Rect {
    let min = canvas.min + Vec2::new(coord.x as f32 * cell_size, coord.y as f32 * cell_size);
    Rect::from_min_size(min, Vec2::splat(cell_size))
}

fn paint_preview_cell(
    painter: &egui::Painter,
    rect: Rect,
    mode: GridMode,
    fill: Color32,
    stroke: Stroke,
) {
    match mode {
        GridMode::Square => {
            painter.rect_filled(rect, 5.0, fill);
            painter.rect_stroke(rect, 5.0, stroke, StrokeKind::Inside);
        }
        GridMode::Hex => {
            painter.add(Shape::convex_polygon(
                regular_hex_points(rect.center(), 22.0).to_vec(),
                fill,
                stroke,
            ));
        }
    }
}

fn paint_cell(
    painter: &egui::Painter,
    mode: GridMode,
    canvas: Rect,
    coord: Coord2,
    cell_size: f32,
    fill: Color32,
    stroke: Stroke,
) {
    match mode {
        GridMode::Square => {
            let rect = square_rect(canvas, coord, cell_size).shrink(1.0);
            painter.rect_filled(rect, 3.0, fill);
            painter.rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);
        }
        GridMode::Hex => {
            painter.add(Shape::convex_polygon(
                hex_points(canvas, coord, cell_size, 1.0).to_vec(),
                fill,
                stroke,
            ));
        }
    }
}

fn paint_cell_outline(
    painter: &egui::Painter,
    mode: GridMode,
    canvas: Rect,
    coord: Coord2,
    cell_size: f32,
    inset: f32,
    stroke: Stroke,
) {
    match mode {
        GridMode::Square => {
            painter.rect_stroke(
                square_rect(canvas, coord, cell_size).shrink(inset),
                3.0,
                stroke,
                StrokeKind::Inside,
            );
        }
        GridMode::Hex => {
            painter.add(Shape::closed_line(
                hex_points(canvas, coord, cell_size, inset).to_vec(),
                stroke,
            ));
        }
    }
}

fn paint_sockets(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    mode: GridMode,
    sockets: &[bool],
) {
    for (index, socket) in sockets.iter().copied().enumerate() {
        if !socket {
            continue;
        }
        let vector = match mode {
            GridMode::Square => match index {
                0 => Vec2::new(0.0, -1.0),
                1 => Vec2::new(1.0, 0.0),
                2 => Vec2::new(0.0, 1.0),
                _ => Vec2::new(-1.0, 0.0),
            },
            GridMode::Hex => {
                let angle = -PI / 3.0 + index as f32 * PI / 3.0;
                Vec2::new(angle.cos(), angle.sin())
            }
        };
        painter.line_segment(
            [
                center + vector * (radius * 0.24),
                center + vector * (radius * 0.92),
            ],
            Stroke::new((radius * 0.2).clamp(3.0, 6.0), Color32::WHITE),
        );
    }
    painter.circle_filled(center, radius * 0.18, Color32::WHITE);
}

/// A square RGBA pixel buffer used to render a tile.
#[derive(Clone, PartialEq)]
struct Raster {
    size: usize,
    pixels: Vec<Rgba>,
}

type Rgba = [u8; 4];

impl Raster {
    fn filled(size: usize, color: Rgba) -> Self {
        Self {
            size,
            pixels: vec![color; size * size],
        }
    }

    fn get(&self, x: usize, y: usize) -> Rgba {
        self.pixels[y * self.size + x]
    }

    fn set(&mut self, x: usize, y: usize, color: Rgba) {
        self.pixels[y * self.size + x] = color;
    }
}

/// Screen-space unit vector for a square socket, matching `paint_sockets` and
/// `SquareDirection::ALL` order (North, East, South, West).
fn socket_vector(index: usize) -> (f32, f32) {
    match index {
        0 => (0.0, -1.0),
        1 => (1.0, 0.0),
        2 => (0.0, 1.0),
        _ => (-1.0, 0.0),
    }
}

/// Draws a tile's base (identity-orientation) picture from its local sockets:
/// a dim background of `color` with a bright hub and an arm toward each active
/// edge. Orientation is applied later by [`transform_raster`].
fn rasterize_tile(sockets: &[bool], color: [u8; 3]) -> Raster {
    let dim = |c: u8| (c as f32 * 0.30) as u8;
    let bright = |c: u8| (c as f32 * 0.5 + 128.0).min(255.0) as u8;
    let background = [dim(color[0]), dim(color[1]), dim(color[2]), 255];
    let road = [bright(color[0]), bright(color[1]), bright(color[2]), 255];

    let size = RASTER_SIZE;
    let mut raster = Raster::filled(size, background);
    let center = (size as f32 - 1.0) / 2.0;
    let reach = center;
    let arm_half = size as f32 * 0.16;
    let hub_half = size as f32 * 0.18;

    for y in 0..size {
        for x in 0..size {
            let rx = x as f32 - center;
            let ry = y as f32 - center;
            let mut on_road = rx.abs() <= hub_half && ry.abs() <= hub_half;
            if !on_road {
                for (index, active) in sockets.iter().enumerate() {
                    if !active {
                        continue;
                    }
                    let (dx, dy) = socket_vector(index);
                    let along = rx * dx + ry * dy;
                    let perp = rx * -dy + ry * dx;
                    if along >= 0.0 && along <= reach && perp.abs() <= arm_half {
                        on_road = true;
                        break;
                    }
                }
            }
            if on_road {
                raster.set(x, y, road);
            }
        }
    }
    raster
}

/// Reorients a raster through a `D4` symmetry, reusing [`D4::checked_apply`] so
/// pixels rotate and reflect with the exact convention used for sockets. Pixel
/// centers are mapped through doubled, center-relative coordinates so the
/// halving back to indices is always exact, and the transform is a bijection
/// over the lattice so every destination pixel is written once.
fn transform_raster(base: &Raster, transform: D4) -> Raster {
    let size = base.size;
    let offset = size as i32 - 1;
    let mut out = Raster::filled(size, [0, 0, 0, 0]);
    for y in 0..size {
        for x in 0..size {
            let vector = Coord2::new(2 * x as i32 - offset, 2 * y as i32 - offset);
            let mapped = transform
                .checked_apply(vector)
                .expect("raster coordinates are small enough to never overflow");
            let nx = ((mapped.x + offset) / 2) as usize;
            let ny = ((mapped.y + offset) / 2) as usize;
            out.set(nx, ny, base.get(x, y));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use seamless_tiler::D6;

    #[test]
    fn square_pointer_coordinates_respect_canvas_bounds() {
        let canvas = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(100.0, 50.0));
        let extent = Extent2::new(2, 1);
        assert_eq!(
            pointer_coordinate(
                GridMode::Square,
                canvas,
                Pos2::new(75.0, 30.0),
                50.0,
                extent
            ),
            Some(Coord2::new(1, 0))
        );
        assert_eq!(
            pointer_coordinate(
                GridMode::Square,
                canvas,
                Pos2::new(110.0, 30.0),
                50.0,
                extent
            ),
            None
        );
    }

    #[test]
    fn hex_pointer_coordinates_find_staggered_cells_and_reject_corner_gaps() {
        let extent = Extent2::new(2, 2);
        let size = 60.0;
        let canvas = Rect::from_min_size(
            Pos2::new(10.0, 20.0),
            canvas_size(GridMode::Hex, extent, size),
        );
        for coord in [Coord2::new(0, 0), Coord2::new(1, 0), Coord2::new(0, 1)] {
            assert_eq!(
                pointer_coordinate(
                    GridMode::Hex,
                    canvas,
                    cell_center(GridMode::Hex, canvas, coord, size),
                    size,
                    extent
                ),
                Some(coord)
            );
        }
        assert_eq!(
            pointer_coordinate(GridMode::Hex, canvas, canvas.left_top(), size, extent),
            None
        );
    }

    #[test]
    fn uncertainty_colors_distinguish_small_and_large_domains() {
        assert_ne!(uncertainty_color(2, 13), uncertainty_color(13, 13));
    }

    #[test]
    fn orientation_labels_cover_square_and_hex_rotations() {
        assert_eq!(orientation_label(Orientation::Square(D4::IDENTITY)), "0°");
        assert_eq!(
            orientation_label(Orientation::Square(D4::new(QuarterTurns::One, true))),
            "90° reflected"
        );
        assert_eq!(
            orientation_label(Orientation::Hex(D6::new(SixthTurns::Five, false))),
            "300°"
        );
    }

    const TEST_COLOR: [u8; 3] = [200, 160, 120];

    fn north_arm() -> Raster {
        rasterize_tile(&[true, false, false, false], TEST_COLOR)
    }

    #[test]
    fn identity_transform_leaves_a_raster_unchanged() {
        let base = north_arm();
        assert_eq!(transform_raster(&base, D4::IDENTITY).pixels, base.pixels);
    }

    #[test]
    fn four_quarter_turns_return_the_original_raster() {
        let base = north_arm();
        let quarter = D4::new(QuarterTurns::One, false);
        let mut rotated = base.clone();
        for _ in 0..4 {
            rotated = transform_raster(&rotated, quarter);
        }
        assert_eq!(rotated.pixels, base.pixels);
    }

    #[test]
    fn transform_then_inverse_restores_the_raster() {
        let base = rasterize_tile(&[true, true, false, false], TEST_COLOR);
        for transform in D4::ALL {
            let there = transform_raster(&base, transform);
            let back = transform_raster(&there, transform.inverse());
            assert_eq!(
                back.pixels, base.pixels,
                "transform {transform:?} round-trip"
            );
        }
    }

    #[test]
    fn rotating_a_north_arm_raster_points_it_east() {
        let base = north_arm();
        let rotated = transform_raster(&base, D4::new(QuarterTurns::One, false));
        let mid = RASTER_SIZE / 2;
        // The north arm's bright road moves to the east edge; the north edge clears,
        // mirroring apply_direction(North) == East.
        assert_eq!(rotated.get(RASTER_SIZE - 2, mid), base.get(mid, 1));
        assert_eq!(rotated.get(mid, 1), base.get(1, mid));
    }

    #[test]
    fn reflection_mirrors_an_east_arm_to_the_west() {
        let base = rasterize_tile(&[false, true, false, false], TEST_COLOR);
        let reflected = transform_raster(&base, D4::new(QuarterTurns::Zero, true));
        let mid = RASTER_SIZE / 2;
        let road = base.get(RASTER_SIZE - 2, mid);
        let background = base.get(1, mid);
        assert_ne!(road, background);
        assert_eq!(reflected.get(1, mid), road);
        assert_eq!(reflected.get(RASTER_SIZE - 2, mid), background);
    }

    #[test]
    fn rasterize_marks_only_active_socket_edges() {
        let base = north_arm();
        let mid = RASTER_SIZE / 2;
        // North edge is road; west and south edges stay background.
        assert_ne!(base.get(mid, 1), base.get(1, mid));
        assert_eq!(base.get(mid, RASTER_SIZE - 2), base.get(1, mid));
    }
}
