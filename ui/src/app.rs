use std::collections::HashSet;
use std::f32::consts::PI;
use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, PointerButton, Pos2, Rect, RichText, Sense, Shape,
    Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use seamless_tiler::{
    AxisBoundaries, Boundary, CellId, Coord2, Direction, Extent2, HexDirection, QuarterTurns,
    SixthTurns, SquareDirection, TileId, WfcStatus,
};

use crate::model::{
    CanvasTool, CellVisual, DEFAULT_EXTENT, EditableRaster, EditorModel, GridMode, MAX_DIMENSION,
    Orientation, TileStyle,
};
use crate::raster::{
    DEFAULT_PAINT_COLOR, EDGE_BACKGROUND, HexRaster, Rgba, SQUARE_RASTER_SIZE, SquareRaster,
    VariantImage,
};
use crate::seams::{EdgeCopyResult, EdgeRef, OrphanEdges};

#[cfg(test)]
mod contact_sheet;

const DEFAULT_CELL_SIZE: f32 = 52.0;
const DEFAULT_STEPS_PER_SECOND: f32 = 8.0;
const EDITOR_PIXEL_SIZE: f32 = 8.0;
const MAX_BRUSH_SIZE: usize = 8;
const MAX_HEX_BRUSH_SIZE: usize = 6;
const HEX_EDITOR_HEIGHT: f32 = 288.0;
const SQRT_3: f32 = 1.732_050_8;

pub struct TilerApp {
    model: EditorModel,
    dimension_inputs: [Extent2; 2],
    cell_size: f32,
    steps_per_second: f32,
    last_frame_time: f64,
    step_accumulator: f64,
    notice: Option<String>,
    texture_cache: Option<VariantTextureCache>,
    editor_texture: Option<EditorTextureCache>,
    paint_color: Rgba,
    brush_sizes: [usize; 2],
    paint_stroke: Option<PaintStroke>,
    square_edge_copy: EdgeCopyControls<SquareDirection>,
    hex_edge_copy: EdgeCopyControls<HexDirection>,
}

/// One mode's edge assistant selections, kept independent per mode so switching
/// modes never disturbs the other session's authoring state.
struct EdgeCopyControls<D> {
    source: Option<EdgeRef<D>>,
    target: D,
    reverse: bool,
}

#[derive(Clone, Copy)]
struct PaintStroke {
    mode: GridMode,
    tile: TileId,
    button: PointerButton,
    last_sample: Coord2,
}

/// The selected hex tile's editor texture, rebuilt when the mode, tile, or
/// catalog version changes.
struct EditorTextureCache {
    mode: GridMode,
    tile: TileId,
    version: u64,
    texture: TextureHandle,
}

/// Per-variant textures for the active catalog, refreshed when the mode or
/// catalog version changes.
///
/// Every pointer event of a stroke bumps the catalog version, but a stroke only
/// changes one tile's orientations, so each texture records the image it was
/// uploaded from and unchanged variants keep their existing handle.
struct VariantTextureCache {
    mode: GridMode,
    version: u64,
    images: Vec<VariantImage>,
    textures: Vec<TextureHandle>,
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
            texture_cache: None,
            editor_texture: None,
            paint_color: DEFAULT_PAINT_COLOR,
            brush_sizes: [1; 2],
            paint_stroke: None,
            square_edge_copy: EdgeCopyControls {
                source: None,
                target: SquareDirection::North,
                reverse: false,
            },
            hex_edge_copy: EdgeCopyControls {
                source: None,
                target: HexDirection::NorthEast,
                reverse: false,
            },
        }
    }
}

impl eframe::App for TilerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.advance_playback(ui);
        if ui.input(|input| input.pointer.any_released()) {
            self.paint_stroke = None;
        }
        self.ensure_textures(ui.ctx());

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

    fn ensure_textures(&mut self, ctx: &egui::Context) {
        let mode = self.model.mode();
        let version = self.model.catalog_version();
        if self
            .texture_cache
            .as_ref()
            .is_some_and(|cache| cache.mode == mode && cache.version == version)
        {
            return;
        }

        // Only the previous cache of the same mode can be reused; switching modes
        // replaces the whole catalog.
        let mut reusable: Vec<Option<(VariantImage, TextureHandle)>> = self
            .texture_cache
            .take()
            .filter(|cache| cache.mode == mode)
            .map(|cache| {
                cache
                    .images
                    .into_iter()
                    .zip(cache.textures)
                    .map(Some)
                    .collect()
            })
            .unwrap_or_default();

        let count = self.model.variant_count();
        let mut images = Vec::with_capacity(count);
        let mut textures = Vec::with_capacity(count);
        for index in 0..count {
            let image = self
                .model
                .variant_image(index)
                .expect("catalog variants have raster images")
                .clone();
            let cached = reusable
                .get_mut(index)
                .filter(|slot| slot.as_ref().is_some_and(|(cached, _)| *cached == image))
                .and_then(Option::take);
            let texture = match cached {
                Some((_, texture)) => texture,
                None => build_variant_texture(ctx, index, &image),
            };
            images.push(image);
            textures.push(texture);
        }
        self.texture_cache = Some(VariantTextureCache {
            mode,
            version,
            images,
            textures,
        });
    }

    /// The cached texture for a variant in the active catalog, if any.
    fn variant_texture(&self, index: usize) -> Option<&TextureHandle> {
        self.texture_cache
            .as_ref()
            .filter(|cache| cache.mode == self.model.mode())
            .and_then(|cache| cache.textures.get(index))
    }

    /// Draws a variant's raster into `rect`, if its texture is cached.
    fn paint_variant_texture(&self, painter: &egui::Painter, rect: Rect, index: usize) {
        let Some(texture) = self.variant_texture(index) else {
            return;
        };
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    fn show_controls(&mut self, ui: &mut egui::Ui) {
        self.show_grid_controls(ui);
        ui.separator();
        self.show_topology_controls(ui);
        ui.separator();
        self.show_solver_controls(ui);
        ui.separator();
        // Edge families cost a pass over the whole catalog, so derive them once
        // and share them between the catalog list and the editor overlay.
        let orphan_edges = self.model.orphan_edges();
        self.show_tile_catalog(ui, &orphan_edges);
        ui.separator();
        self.show_tile_editor(ui, &orphan_edges);
        ui.separator();
        // Catalog edits above may have added, removed, or repainted variants.
        self.ensure_textures(ui.ctx());
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
            self.paint_stroke = None;
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
        let reset = ui
            .horizontal_wrapped(|ui| {
                if ui.button("Apply size").clicked() {
                    self.model.resize(*dimensions);
                }
                if ui.button("Clear pins").clicked() {
                    let cleared = self.model.clear_pins();
                    self.notice = Some(format!("Cleared {cleared} pin(s)"));
                }
                ui.button("Reset mode").clicked()
            })
            .inner;
        if reset {
            self.model.reset_active();
            self.paint_stroke = None;
            self.clear_edge_source(mode);
            self.dimension_inputs[mode.index()] = DEFAULT_EXTENT;
            self.notice = Some(format!("Reset {} mode", mode.label()));
        }
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
        ui.weak("Bounded edges require background-only sample strips.");
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

    fn show_tile_catalog(&mut self, ui: &mut egui::Ui, orphan_edges: &[(TileId, OrphanEdges)]) {
        ui.heading("Tile catalog");
        ui.weak("Select a tile to edit its samples; border samples drive matching.");
        let count = orphan_edges
            .iter()
            .map(|(_, edges)| orphan_count(*edges))
            .sum::<usize>();
        if count == 0 {
            ui.colored_label(
                Color32::from_rgb(100, 220, 130),
                "All tile edges have partners",
            );
        } else {
            ui.colored_label(
                Color32::from_rgb(255, 105, 105),
                format!("{count} orphan edge(s) need a matching partner"),
            );
        }
        let mut pending_delete: Option<(TileId, String)> = None;
        for (tile_id, style) in self.model.tiles() {
            let tile_orphans = orphan_edges
                .iter()
                .find_map(|(candidate, edges)| (*candidate == tile_id).then_some(*edges));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let selected = self.model.selected_tile() == Some(tile_id);
                if ui.selectable_label(selected, "Edit").clicked() {
                    self.model.set_selected_tile(tile_id);
                    self.paint_stroke = None;
                }
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
                if let Some(edges) = tile_orphans {
                    let count = orphan_count(edges);
                    if count > 0 {
                        ui.colored_label(
                            Color32::from_rgb(255, 105, 105),
                            format!("{count} orphan"),
                        );
                    }
                }
            });
        }
        if let Some((tile_id, name)) = pending_delete {
            self.model.remove_tile(tile_id);
            self.paint_stroke = None;
            self.clear_edge_source(self.model.mode());
            self.notice = Some(format!("Deleted {name}"));
        }
        ui.add_space(4.0);
        if ui.button("Add tile").clicked() {
            self.model.add_tile();
            self.paint_stroke = None;
            self.notice = Some("Added a tile".to_owned());
        }
    }

    fn show_tile_editor(&mut self, ui: &mut egui::Ui, orphan_edges: &[(TileId, OrphanEdges)]) {
        ui.heading("Pencil editor");
        let Some(tile_id) = self.model.selected_tile() else {
            ui.weak("Add a tile to begin painting.");
            self.paint_stroke = None;
            return;
        };
        let Some(style) = self.model.tile_style(tile_id) else {
            self.paint_stroke = None;
            return;
        };
        let mode = self.model.mode();
        ui.label(RichText::new(style.name).strong());
        ui.horizontal(|ui| {
            ui.label("Paint");
            ui.color_edit_button_srgba_unmultiplied(&mut self.paint_color);
        });
        let (max_brush, brush_label) = match mode {
            GridMode::Square => (MAX_BRUSH_SIZE, "Brush pixels"),
            GridMode::Hex => (MAX_HEX_BRUSH_SIZE, "Brush samples"),
        };
        ui.add(
            egui::Slider::new(&mut self.brush_sizes[mode.index()], 1..=max_brush).text(brush_label),
        );
        ui.weak("Left-drag paints · Right-drag erases to the closed-edge background");

        let tile_orphans = tile_orphan_edges(orphan_edges, tile_id);
        match mode {
            GridMode::Square => self.show_square_editor(ui, tile_id, tile_orphans),
            GridMode::Hex => self.show_hex_editor(ui, tile_id, tile_orphans),
        }
    }

    fn show_square_editor(
        &mut self,
        ui: &mut egui::Ui,
        tile_id: TileId,
        orphan_edges: OrphanEdges,
    ) {
        ui.weak("Linked border painting is active; matching and reversed edges update together.");

        self.show_square_edge_assistant(ui, orphan_edges);

        let editor_size = Vec2::splat(SQUARE_RASTER_SIZE as f32 * EDITOR_PIXEL_SIZE);
        let (response, painter) = ui.allocate_painter(editor_size, Sense::drag());
        let hovered = self.drive_paint_stroke(ui, &response, tile_id, editor_pixel);

        if let Some(EditableRaster::Square(raster)) = self.model.selected_raster() {
            paint_raster_editor(&painter, response.rect, raster);
            paint_orphan_edges(&painter, response.rect, orphan_edges);
        }
        if let Some(pixel) = hovered {
            paint_brush_preview(
                &painter,
                response.rect,
                pixel,
                self.brush_sizes[GridMode::Square.index()],
            );
        }
    }

    fn show_hex_editor(&mut self, ui: &mut egui::Ui, tile_id: TileId, orphan_edges: OrphanEdges) {
        ui.weak("Linked border painting is active; matching and reversed edges update together.");

        self.show_hex_edge_assistant(ui, orphan_edges);

        let (response, painter) = ui.allocate_painter(hex_editor_size(), Sense::drag());
        let hovered = self.drive_paint_stroke(ui, &response, tile_id, hex_editor_sample);

        self.ensure_editor_texture(ui.ctx(), tile_id);
        if let Some(cache) = self
            .editor_texture
            .as_ref()
            .filter(|cache| cache.mode == GridMode::Hex && cache.tile == tile_id)
        {
            painter.image(
                cache.texture.id(),
                response.rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let bounds = hex_cell_bounds(response.rect);
        let polygon = regular_hex_points(bounds.center(), bounds.height() * 0.5);
        painter.add(Shape::closed_line(
            polygon.to_vec(),
            Stroke::new(1.0, Color32::from_gray(130)),
        ));
        paint_hex_orphan_edges(&painter, polygon, orphan_edges);
        if let Some(sample) = hovered {
            paint_hex_brush_preview(
                &painter,
                response.rect,
                sample,
                self.brush_sizes[GridMode::Hex.index()],
            );
        }
    }

    /// Applies pointer drags to the selected tile and returns the hovered
    /// sample, if the pointer is over one.
    ///
    /// `locate` maps a pointer position to an authoritative sample, so both
    /// modes share stroke continuation, button handling, and erasure.
    fn drive_paint_stroke(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        tile: TileId,
        locate: impl Fn(Rect, Pos2) -> Option<Coord2>,
    ) -> Option<Coord2> {
        let mode = self.model.mode();
        let hovered = response
            .hover_pos()
            .or_else(|| response.interact_pointer_pos())
            .and_then(|pointer| locate(response.rect, pointer));
        let (primary_down, secondary_down, released) = ui.input(|input| {
            (
                input.pointer.button_down(PointerButton::Primary),
                input.pointer.button_down(PointerButton::Secondary),
                input.pointer.any_released(),
            )
        });

        if released {
            self.paint_stroke = None;
        }
        if !response.is_pointer_button_down_on() {
            return hovered;
        }
        let Some(sample) = hovered else {
            self.paint_stroke = None;
            return hovered;
        };
        let button = if secondary_down {
            PointerButton::Secondary
        } else if primary_down {
            PointerButton::Primary
        } else {
            return hovered;
        };

        let from = self
            .paint_stroke
            .filter(|stroke| stroke.mode == mode && stroke.tile == tile && stroke.button == button)
            .map_or(sample, |stroke| stroke.last_sample);
        let color = if button == PointerButton::Secondary {
            EDGE_BACKGROUND
        } else {
            self.paint_color
        };
        if self
            .model
            .paint_selected_tile(from, sample, self.brush_sizes[mode.index()], color)
        {
            ui.ctx().request_repaint();
        }
        self.paint_stroke = Some(PaintStroke {
            mode,
            tile,
            button,
            last_sample: sample,
        });
        hovered
    }

    /// Uploads the selected hex raster as a nearest-filtered editor texture.
    fn ensure_editor_texture(&mut self, ctx: &egui::Context, tile: TileId) {
        let mode = self.model.mode();
        let version = self.model.catalog_version();
        if self.editor_texture.as_ref().is_some_and(|cache| {
            cache.mode == mode && cache.tile == tile && cache.version == version
        }) {
            return;
        }
        let Some(EditableRaster::Hex(raster)) = self.model.selected_raster() else {
            return;
        };
        let image = raster.to_variant_image();
        let texture = ctx.load_texture(
            "hex-editor",
            ColorImage::from_rgba_unmultiplied(image.size(), image.rgba()),
            TextureOptions::NEAREST,
        );
        self.editor_texture = Some(EditorTextureCache {
            mode,
            tile,
            version,
            texture,
        });
    }

    /// Drops one mode's source selection after its catalog changed underneath it.
    fn clear_edge_source(&mut self, mode: GridMode) {
        match mode {
            GridMode::Square => self.square_edge_copy.source = None,
            GridMode::Hex => self.hex_edge_copy.source = None,
        }
    }

    fn show_square_edge_assistant(&mut self, ui: &mut egui::Ui, orphan_edges: OrphanEdges) {
        let tiles = self.model.tiles();
        let request = edge_assistant_controls(
            ui,
            &tiles,
            orphan_edges,
            &mut self.square_edge_copy,
            square_direction_label,
        );
        if let Some((source, target, reverse)) = request {
            let result = self
                .model
                .copy_selected_square_edge(source, target, reverse);
            self.report_edge_copy(ui, result);
        }
    }

    fn show_hex_edge_assistant(&mut self, ui: &mut egui::Ui, orphan_edges: OrphanEdges) {
        let tiles = self.model.tiles();
        let request = edge_assistant_controls(
            ui,
            &tiles,
            orphan_edges,
            &mut self.hex_edge_copy,
            hex_direction_label,
        );
        if let Some((source, target, reverse)) = request {
            let result = self.model.copy_selected_hex_edge(source, target, reverse);
            self.report_edge_copy(ui, result);
        }
    }

    fn report_edge_copy(&mut self, ui: &egui::Ui, result: EdgeCopyResult) {
        self.notice = Some(match result {
            EdgeCopyResult::Applied => "Copied edge and updated its linked family".to_owned(),
            EdgeCopyResult::NoChange => "Target edge already matches the source".to_owned(),
            EdgeCopyResult::Conflict => {
                "Copy would assign conflicting linked corner colors".to_owned()
            }
            EdgeCopyResult::Invalid => "Choose a valid source and target edge".to_owned(),
        });
        if result == EdgeCopyResult::Applied {
            ui.ctx().request_repaint();
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
            .expect("variant refers to a catalog tile");
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
            self.paint_variant_texture(
                ui.painter(),
                preview_texture_rect(self.model.mode(), rect, 4.0),
                variant_index,
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
                .expect("pins refer to catalog tiles");
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
                    .expect("variant refers to a catalog tile");
                ui.label(format!(
                    "Collapsed: {} · {}",
                    style.name,
                    orientation_label(view.orientation)
                ));
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                self.paint_variant_texture(
                    ui.painter(),
                    preview_texture_rect(self.model.mode(), rect, 0.0),
                    variant,
                );
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
                            .expect("variant refers to a catalog tile");
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
                            .expect("variant refers to a catalog tile")
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
                    self.paint_variant_texture(
                        painter,
                        cell_texture_rect(mode, canvas, coord, self.cell_size),
                        variant,
                    );
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

fn build_variant_texture(ctx: &egui::Context, index: usize, image: &VariantImage) -> TextureHandle {
    ctx.load_texture(
        format!("variant-{index}"),
        ColorImage::from_rgba_unmultiplied(image.size(), image.rgba()),
        TextureOptions::NEAREST,
    )
}

fn editor_pixel(canvas: Rect, pointer: Pos2) -> Option<Coord2> {
    let local = pointer - canvas.min;
    let extent = SQUARE_RASTER_SIZE as f32 * EDITOR_PIXEL_SIZE;
    if local.x < 0.0 || local.y < 0.0 || local.x >= extent || local.y >= extent {
        return None;
    }
    Some(Coord2::new(
        (local.x / EDITOR_PIXEL_SIZE).floor() as i32,
        (local.y / EDITOR_PIXEL_SIZE).floor() as i32,
    ))
}

/// The rect an exported hex image must fill to depict a cell of `bounds`.
///
/// Hex samples are points on a lattice whose outermost ring lies exactly on the
/// cell boundary, but a renderer maps each texel to a *block* of its destination
/// rect. Inflating the bounds by half a texel on every side puts sample centers
/// back on the cell's true geometry, so neighboring cells' opaque borders meet
/// instead of leaving transparent notches along the slanted sides.
///
/// The resulting one-texel overlap is invisible: facing strips are byte-identical
/// by the matching contract, and only boundary samples of either cell reach into
/// the overlap band.
fn hex_image_rect(bounds: Rect) -> Rect {
    let [width, height] = HexRaster::IMAGE_SIZE;
    let [span_x, span_y] = HexRaster::SAMPLE_SPAN;
    Rect::from_center_size(
        bounds.center(),
        Vec2::new(
            bounds.width() * width as f32 / span_x as f32,
            bounds.height() * height as f32 / span_y as f32,
        ),
    )
}

/// The cell bounds an exported hex image depicts; the inverse of
/// [`hex_image_rect`].
fn hex_cell_bounds(image: Rect) -> Rect {
    let [width, height] = HexRaster::IMAGE_SIZE;
    let [span_x, span_y] = HexRaster::SAMPLE_SPAN;
    Rect::from_center_size(
        image.center(),
        Vec2::new(
            image.width() * span_x as f32 / width as f32,
            image.height() * span_y as f32 / height as f32,
        ),
    )
}

/// The footprint of the hex sample editor, which allocates the image rect for a
/// pointy-top cell of [`HEX_EDITOR_HEIGHT`].
fn hex_editor_size() -> Vec2 {
    hex_image_rect(Rect::from_min_size(
        Pos2::ZERO,
        Vec2::new(hex_width(HEX_EDITOR_HEIGHT), HEX_EDITOR_HEIGHT),
    ))
    .size()
}

/// Maps a pointer position to the hex sample drawn under it.
///
/// The canvas holds the exported image stretched to the cell's pointy-top
/// bounds, so mapping through texels reuses `HexRaster::sample_at_texel` and is
/// mask-aware: positions in the rect's corners fall outside the hexagon.
fn hex_editor_sample(canvas: Rect, pointer: Pos2) -> Option<Coord2> {
    let [width, height] = HexRaster::IMAGE_SIZE;
    let local = pointer - canvas.min;
    if local.x < 0.0 || local.y < 0.0 || local.x >= canvas.width() || local.y >= canvas.height() {
        return None;
    }
    HexRaster::sample_at_texel(
        (local.x / canvas.width() * width as f32).floor() as i32,
        (local.y / canvas.height() * height as f32).floor() as i32,
    )
}

fn hex_texel_rect(canvas: Rect, x: i32, y: i32) -> Rect {
    let [width, height] = HexRaster::IMAGE_SIZE;
    let size = Vec2::new(
        canvas.width() / width as f32,
        canvas.height() / height as f32,
    );
    Rect::from_min_size(
        canvas.min + Vec2::new(x as f32 * size.x, y as f32 * size.y),
        size,
    )
}

/// Tints the texels owned by the samples a brush impression would cover.
fn paint_hex_brush_preview(
    painter: &egui::Painter,
    canvas: Rect,
    center: Coord2,
    brush_size: usize,
) {
    let samples: HashSet<Coord2> = HexRaster::brush_samples(center, brush_size)
        .into_iter()
        .collect();
    let Some(bounds) = samples
        .iter()
        .map(|coord| HexRaster::sample_texel(*coord))
        .fold(None, |bounds: Option<[i32; 4]>, [x, y]| {
            Some(match bounds {
                // Samples own their center texel and the column to its right.
                None => [x, x + 1, y, y],
                Some([min_x, max_x, min_y, max_y]) => {
                    [min_x.min(x), max_x.max(x + 1), min_y.min(y), max_y.max(y)]
                }
            })
        })
    else {
        return;
    };
    for y in bounds[2]..=bounds[3] {
        for x in bounds[0]..=bounds[1] {
            if HexRaster::sample_at_texel(x, y).is_some_and(|coord| samples.contains(&coord)) {
                painter.rect_filled(
                    hex_texel_rect(canvas, x, y),
                    0.0,
                    Color32::from_white_alpha(70),
                );
            }
        }
    }
}

fn paint_raster_editor(painter: &egui::Painter, canvas: Rect, raster: &SquareRaster) {
    for y in 0..SQUARE_RASTER_SIZE {
        for x in 0..SQUARE_RASTER_SIZE {
            let min =
                canvas.min + Vec2::new(x as f32 * EDITOR_PIXEL_SIZE, y as f32 * EDITOR_PIXEL_SIZE);
            let rect = Rect::from_min_size(min, Vec2::splat(EDITOR_PIXEL_SIZE));
            let checker = if (x + y) % 2 == 0 {
                Color32::from_gray(56)
            } else {
                Color32::from_gray(42)
            };
            painter.rect_filled(rect, 0.0, checker);
            let [red, green, blue, alpha] = raster.get(x, y);
            painter.rect_filled(
                rect,
                0.0,
                Color32::from_rgba_unmultiplied(red, green, blue, alpha),
            );
        }
    }

    let grid_stroke = Stroke::new(0.5, Color32::from_black_alpha(80));
    for index in 0..=SQUARE_RASTER_SIZE {
        let offset = index as f32 * EDITOR_PIXEL_SIZE;
        painter.line_segment(
            [
                canvas.min + Vec2::new(offset, 0.0),
                canvas.left_bottom() + Vec2::new(offset, 0.0),
            ],
            grid_stroke,
        );
        painter.line_segment(
            [
                canvas.min + Vec2::new(0.0, offset),
                canvas.right_top() + Vec2::new(0.0, offset),
            ],
            grid_stroke,
        );
    }
    painter.rect_stroke(
        canvas,
        0.0,
        Stroke::new(1.0, Color32::from_gray(130)),
        StrokeKind::Inside,
    );
}

/// Draws the shared edge assistant: per-side health, a catalog-wide source
/// picker, and the target and reversal controls.
///
/// Returns the requested copy so the caller can route it to the matching mode,
/// which keeps the model borrow separate from the control state borrow.
fn edge_assistant_controls<D: Direction>(
    ui: &mut egui::Ui,
    tiles: &[(TileId, TileStyle)],
    orphan_edges: OrphanEdges,
    controls: &mut EdgeCopyControls<D>,
    label: fn(D) -> &'static str,
) -> Option<(EdgeRef<D>, D, bool)> {
    ui.add_space(4.0);
    ui.label(RichText::new("Edge assistant").strong());
    ui.horizontal_wrapped(|ui| {
        ui.label("Selected edge status:");
        for direction in D::ALL.iter().copied() {
            let orphan = orphan_edges[direction.index()];
            ui.colored_label(
                if orphan {
                    Color32::from_rgb(255, 105, 105)
                } else {
                    Color32::from_rgb(100, 220, 130)
                },
                format!(
                    "{} {}",
                    label(direction),
                    if orphan { "orphan" } else { "linked" }
                ),
            );
        }
    });

    if controls
        .source
        .is_some_and(|source| !tiles.iter().any(|(tile, _)| *tile == source.tile))
    {
        controls.source = None;
    }
    let selected_source = controls
        .source
        .and_then(|source| {
            tiles
                .iter()
                .find(|(tile, _)| *tile == source.tile)
                .map(|(_, style)| edge_source_label(&style.name, label(source.direction)))
        })
        .unwrap_or_else(|| "Choose an edge".to_owned());

    ui.horizontal(|ui| {
        ui.label("Source");
        egui::ComboBox::from_id_salt("edge-copy-source")
            .selected_text(selected_source)
            .show_ui(ui, |ui| {
                for (tile, style) in tiles {
                    for direction in D::ALL.iter().copied() {
                        let source = EdgeRef {
                            tile: *tile,
                            direction,
                        };
                        ui.selectable_value(
                            &mut controls.source,
                            Some(source),
                            edge_source_label(&style.name, label(direction)),
                        );
                    }
                }
            });
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Target");
        for direction in D::ALL.iter().copied() {
            ui.selectable_value(&mut controls.target, direction, label(direction));
        }
        ui.checkbox(&mut controls.reverse, "Reverse");
        let copy = ui.add_enabled(controls.source.is_some(), egui::Button::new("Copy"));
        copy.clicked()
            .then(|| {
                controls
                    .source
                    .map(|source| (source, controls.target, controls.reverse))
            })
            .flatten()
    })
    .inner
}

/// Counts the sides of one tile that have no partner to match.
fn orphan_count(orphan_edges: OrphanEdges) -> usize {
    orphan_edges.iter().filter(|orphan| **orphan).count()
}

/// Picks one tile's per-side orphan flags out of the catalog-wide report.
fn tile_orphan_edges(orphan_edges: &[(TileId, OrphanEdges)], tile: TileId) -> OrphanEdges {
    orphan_edges
        .iter()
        .find_map(|(candidate, edges)| (*candidate == tile).then_some(*edges))
        .unwrap_or_default()
}

/// Outlines the orphaned sides of the hex editor polygon.
///
/// Polygon vertex `index` starts the side facing `HexDirection::ALL[index]`,
/// because `regular_hex_points` walks clockwise from the top vertex.
fn paint_hex_orphan_edges(painter: &egui::Painter, polygon: [Pos2; 6], orphan_edges: OrphanEdges) {
    let stroke = Stroke::new(4.0, Color32::from_rgb(255, 75, 75));
    for direction in HexDirection::ALL.iter().copied() {
        if !orphan_edges[direction.index()] {
            continue;
        }
        let index = direction.index();
        painter.line_segment([polygon[index], polygon[(index + 1) % 6]], stroke);
    }
}

fn paint_orphan_edges(painter: &egui::Painter, canvas: Rect, orphan_edges: OrphanEdges) {
    let stroke = Stroke::new(4.0, Color32::from_rgb(255, 75, 75));
    let inset = 2.0;
    for direction in SquareDirection::ALL.iter().copied() {
        if !orphan_edges[direction.index()] {
            continue;
        }
        let points = match direction {
            SquareDirection::North => [
                canvas.left_top() + Vec2::new(0.0, inset),
                canvas.right_top() + Vec2::new(0.0, inset),
            ],
            SquareDirection::East => [
                canvas.right_top() + Vec2::new(-inset, 0.0),
                canvas.right_bottom() + Vec2::new(-inset, 0.0),
            ],
            SquareDirection::South => [
                canvas.left_bottom() + Vec2::new(0.0, -inset),
                canvas.right_bottom() + Vec2::new(0.0, -inset),
            ],
            SquareDirection::West => [
                canvas.left_top() + Vec2::new(inset, 0.0),
                canvas.left_bottom() + Vec2::new(inset, 0.0),
            ],
        };
        painter.line_segment(points, stroke);
    }
}

fn paint_brush_preview(painter: &egui::Painter, canvas: Rect, center: Coord2, brush_size: usize) {
    let size = brush_size as i32;
    let offset = (size - 1) / 2;
    let start_x = (center.x - offset).clamp(0, SQUARE_RASTER_SIZE as i32);
    let start_y = (center.y - offset).clamp(0, SQUARE_RASTER_SIZE as i32);
    let end_x = (center.x - offset + size).clamp(0, SQUARE_RASTER_SIZE as i32);
    let end_y = (center.y - offset + size).clamp(0, SQUARE_RASTER_SIZE as i32);
    let min = canvas.min
        + Vec2::new(
            start_x as f32 * EDITOR_PIXEL_SIZE,
            start_y as f32 * EDITOR_PIXEL_SIZE,
        );
    let max = canvas.min
        + Vec2::new(
            end_x as f32 * EDITOR_PIXEL_SIZE,
            end_y as f32 * EDITOR_PIXEL_SIZE,
        );
    painter.rect_stroke(
        Rect::from_min_max(min, max),
        0.0,
        Stroke::new(2.0, Color32::WHITE),
        StrokeKind::Inside,
    );
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

fn square_direction_label(direction: SquareDirection) -> &'static str {
    match direction {
        SquareDirection::North => "N",
        SquareDirection::East => "E",
        SquareDirection::South => "S",
        SquareDirection::West => "W",
    }
}

fn hex_direction_label(direction: HexDirection) -> &'static str {
    match direction {
        HexDirection::NorthEast => "NE",
        HexDirection::East => "E",
        HexDirection::SouthEast => "SE",
        HexDirection::SouthWest => "SW",
        HexDirection::West => "W",
        HexDirection::NorthWest => "NW",
    }
}

fn edge_source_label(name: &str, direction: &str) -> String {
    format!("{name} · {direction}")
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

fn cell_texture_rect(mode: GridMode, canvas: Rect, coord: Coord2, cell_size: f32) -> Rect {
    match mode {
        GridMode::Square => square_rect(canvas, coord, cell_size),
        GridMode::Hex => hex_image_rect(Rect::from_center_size(
            cell_center(mode, canvas, coord, cell_size),
            Vec2::new(hex_width(cell_size), cell_size),
        )),
    }
}

fn preview_texture_rect(mode: GridMode, rect: Rect, margin: f32) -> Rect {
    let height = (rect.height() - 2.0 * margin).max(0.0);
    match mode {
        GridMode::Square => Rect::from_center_size(rect.center(), Vec2::splat(height)),
        GridMode::Hex => hex_image_rect(Rect::from_center_size(
            rect.center(),
            Vec2::new(hex_width(height), height),
        )),
    }
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
                regular_hex_points(rect.center(), (rect.height() * 0.5 - 2.0).max(0.0)).to_vec(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::TileSurface;
    use seamless_tiler::{D4, D6};

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

    /// The orphan overlay assumes polygon side `index` faces
    /// `HexDirection::ALL[index]`; check that against the editor's own pointer
    /// mapping so a change to either geometry cannot silently mislabel a side.
    ///
    /// The drawn polygon touches the canvas bounds while the exported image's
    /// outermost sample centers sit half a texel inside, so a side's midpoint
    /// probe lands next to that side rather than exactly on it. Nearness is what
    /// the overlay needs, so the assertion compares distances instead.
    #[test]
    fn hex_polygon_sides_face_their_direction() {
        let canvas = Rect::from_min_size(Pos2::new(10.0, 20.0), hex_editor_size());
        let polygon = regular_hex_points(canvas.center(), canvas.height() * 0.5);
        let distance = |left: Coord2, right: Coord2| {
            let dq = left.x - right.x;
            let dr = left.y - right.y;
            (dq.abs() + dr.abs() + (dq + dr).abs()) / 2
        };
        let distance_to_side = |sample: Coord2, side: HexDirection| {
            HexRaster::edge_coordinates(side)
                .into_iter()
                .map(|coord| distance(sample, coord))
                .min()
                .expect("every side has samples")
        };

        for (index, direction) in HexDirection::ALL.iter().copied().enumerate() {
            let midpoint = polygon[index] + (polygon[(index + 1) % 6] - polygon[index]) * 0.5;
            let probe = midpoint + (canvas.center() - midpoint).normalized() * 4.0;
            let sample = hex_editor_sample(canvas, probe)
                .unwrap_or_else(|| panic!("side {index} probe fell outside the hex"));

            let nearest = distance_to_side(sample, direction);
            for other in HexDirection::ALL
                .iter()
                .copied()
                .filter(|d| *d != direction)
            {
                assert!(
                    distance_to_side(sample, other) > nearest,
                    "polygon side {index} is not closest to {direction:?}",
                );
            }
        }
    }

    #[test]
    fn editor_pointer_coordinates_reject_the_far_edges() {
        let canvas = Rect::from_min_size(
            Pos2::new(10.0, 20.0),
            Vec2::splat(SQUARE_RASTER_SIZE as f32 * EDITOR_PIXEL_SIZE),
        );
        assert_eq!(
            editor_pixel(canvas, Pos2::new(10.0, 20.0)),
            Some(Coord2::ZERO)
        );
        assert_eq!(
            editor_pixel(canvas, Pos2::new(265.9, 275.9)),
            Some(Coord2::new(31, 31))
        );
        assert_eq!(editor_pixel(canvas, Pos2::new(266.0, 20.0)), None);
        assert_eq!(editor_pixel(canvas, Pos2::new(10.0, 276.0)), None);
    }

    #[test]
    fn hex_editor_samples_track_the_texel_mapping_and_reject_the_corners() {
        let canvas = Rect::from_min_size(Pos2::new(10.0, 20.0), hex_editor_size());
        assert_eq!(
            hex_editor_sample(canvas, canvas.center()),
            Some(Coord2::ZERO)
        );

        let raster = HexRaster::filled(EDGE_BACKGROUND);
        for coord in raster.coordinates() {
            let [x, y] = HexRaster::sample_texel(coord);
            assert_eq!(
                hex_editor_sample(canvas, hex_texel_rect(canvas, x, y).center()),
                Some(coord),
                "{coord:?}",
            );
        }

        // The rect's corners lie outside the pointy-top hexagon it contains.
        assert_eq!(hex_editor_sample(canvas, canvas.left_top()), None);
        assert_eq!(
            hex_editor_sample(canvas, canvas.right_bottom() - Vec2::splat(0.5)),
            None
        );
        assert_eq!(
            hex_editor_sample(canvas, canvas.left_top() - Vec2::splat(1.0)),
            None
        );
        assert_eq!(hex_editor_sample(canvas, canvas.right_bottom()), None);
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

    /// The world position of one hex sample as the renderer draws it.
    fn sample_position(canvas: Rect, coord: Coord2, cell_size: f32, sample: Coord2) -> Pos2 {
        let image = cell_texture_rect(GridMode::Hex, canvas, coord, cell_size);
        let [width, height] = HexRaster::IMAGE_SIZE;
        let [x, y] = HexRaster::sample_texel(sample);
        image.min
            + Vec2::new(
                (x as f32 + 0.5) / width as f32 * image.width(),
                (y as f32 + 0.5) / height as f32 * image.height(),
            )
    }

    /// Facing sides carry equal strips, so their samples must also land on the
    /// same points; otherwise matching art would still be drawn discontinuously.
    #[test]
    fn hex_edge_samples_meet_their_neighbour_index_for_index() {
        let extent = Extent2::new(4, 4);
        let cell_size = 64.0;
        let canvas = Rect::from_min_size(
            Pos2::new(10.0, 20.0),
            canvas_size(GridMode::Hex, extent, cell_size),
        );
        let coord = Coord2::new(1, 1);

        for direction in HexDirection::ALL.iter().copied() {
            let offset = direction.offset(coord.y);
            let neighbor = Coord2::new(coord.x + offset.x, coord.y + offset.y);
            for index in 0..HexRaster::EDGE_SAMPLES {
                let near = sample_position(
                    canvas,
                    coord,
                    cell_size,
                    HexRaster::edge_sample(direction, index),
                );
                let far = sample_position(
                    canvas,
                    neighbor,
                    cell_size,
                    HexRaster::edge_sample(direction.opposite(), index),
                );
                assert!(
                    (near - far).length() < 0.001,
                    "{direction:?} sample {index} lands at {near:?} but its partner at {far:?}",
                );
            }
        }
    }

    /// Probes a cell's interior against the opaque texels of the cell itself and
    /// every neighbour, which is exactly what a viewer sees along the seams.
    #[test]
    fn hex_cell_textures_cover_their_cell_without_gaps() {
        let extent = Extent2::new(5, 5);
        let cell_size = 64.0;
        let canvas = Rect::from_min_size(
            Pos2::new(10.0, 20.0),
            canvas_size(GridMode::Hex, extent, cell_size),
        );
        let coord = Coord2::new(2, 2);
        let center = cell_center(GridMode::Hex, canvas, coord, cell_size);
        let polygon = hex_points(canvas, coord, cell_size, 0.0);

        // The cell itself plus its six neighbours: nothing else can reach it.
        let mut cells = vec![coord];
        cells.extend(HexDirection::ALL.iter().copied().map(|direction| {
            let offset = direction.offset(coord.y);
            Coord2::new(coord.x + offset.x, coord.y + offset.y)
        }));

        let opaque_at = |cell: Coord2, point: Pos2| -> Option<Coord2> {
            let image = cell_texture_rect(GridMode::Hex, canvas, cell, cell_size);
            let [width, height] = HexRaster::IMAGE_SIZE;
            let local = point - image.min;
            if local.x < 0.0
                || local.y < 0.0
                || local.x >= image.width()
                || local.y >= image.height()
            {
                return None;
            }
            HexRaster::sample_at_texel(
                (local.x / image.width() * width as f32).floor() as i32,
                (local.y / image.height() * height as f32).floor() as i32,
            )
        };

        // Probe like real pixel centers: offset by an irrational-ish fraction so
        // no probe lands exactly on a texel boundary, where which side wins is a
        // measure-zero tie rather than a gap a viewer could see.
        let steps = 140;
        let (mut probes, mut gaps) = (0, 0);
        for row in 0..steps {
            for column in 0..steps {
                let point = center
                    + Vec2::new(
                        (column as f32 + 0.437) / steps as f32 - 0.5,
                        (row as f32 + 0.319) / steps as f32 - 0.5,
                    ) * Vec2::new(hex_width(cell_size), cell_size);
                if !point_in_convex_polygon(point, &polygon) {
                    continue;
                }
                probes += 1;
                if !cells.iter().any(|cell| opaque_at(*cell, point).is_some()) {
                    gaps += 1;
                }
            }
        }
        assert!(
            probes > 10_000,
            "the probe grid must actually cover the cell"
        );
        assert_eq!(
            gaps, 0,
            "{gaps} of {probes} probes fell in a transparent seam",
        );
    }

    #[test]
    fn uncertainty_colors_distinguish_small_and_large_domains() {
        assert_ne!(uncertainty_color(2, 13), uncertainty_color(13, 13));
    }

    /// The drawn rect is the *image* rect, so the cell it depicts is what must
    /// match the pointy-top bounds.
    #[test]
    fn hex_texture_rect_matches_the_pointy_top_cell_bounds() {
        let canvas = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(500.0, 500.0));
        let coord = Coord2::new(2, 1);
        let rect = cell_texture_rect(GridMode::Hex, canvas, coord, 64.0);
        assert_eq!(
            rect.center(),
            cell_center(GridMode::Hex, canvas, coord, 64.0)
        );

        let bounds = hex_cell_bounds(rect);
        assert!((bounds.width() - hex_width(64.0)).abs() < 0.0001);
        assert!((bounds.height() - 64.0).abs() < 0.0001);
        // The image overhangs the cell by exactly half a texel per side.
        let [width, height] = HexRaster::IMAGE_SIZE;
        assert!(
            (rect.width() - bounds.width() - bounds.width() / (width - 1) as f32).abs() < 0.001
        );
        assert!(
            (rect.height() - bounds.height() - bounds.height() / (height - 1) as f32).abs() < 0.001
        );
    }

    #[test]
    fn hex_image_rect_round_trips_cell_bounds() {
        let bounds =
            Rect::from_center_size(Pos2::new(31.0, 47.0), Vec2::new(hex_width(64.0), 64.0));
        let restored = hex_cell_bounds(hex_image_rect(bounds));
        assert!((restored.width() - bounds.width()).abs() < 0.001);
        assert!((restored.height() - bounds.height()).abs() < 0.001);
        assert_eq!(restored.center(), bounds.center());
        assert!(hex_image_rect(bounds).contains_rect(bounds));
    }

    #[test]
    fn preview_texture_rect_preserves_each_mode_aspect_ratio() {
        let rect = Rect::from_min_size(Pos2::new(4.0, 8.0), Vec2::splat(48.0));
        let square = preview_texture_rect(GridMode::Square, rect, 4.0);
        let hex = hex_cell_bounds(preview_texture_rect(GridMode::Hex, rect, 4.0));
        assert_eq!(square.size(), Vec2::splat(40.0));
        assert!((hex.height() - 40.0).abs() < 0.001);
        assert!((hex.width() - hex_width(40.0)).abs() < 0.001);
        assert_eq!(hex.center(), rect.center());
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
}
