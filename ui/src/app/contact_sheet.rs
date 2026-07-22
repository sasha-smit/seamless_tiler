//! Offscreen renders of the canvas and palette, used to check visual parity.
//!
//! The native window cannot be inspected from a test, so this module composites
//! variant images with the same nearest sampling a renderer applies, driven by
//! the *real* geometry helpers in [`super`]. That makes seams, orientations, and
//! preview framing assertable rather than a matter of eyeballing the app.
//!
//! Setting `TILER_CONTACT_SHEET` to a directory also writes the sheets as binary
//! PPM files so they can be viewed directly.

use std::collections::VecDeque;
use std::path::PathBuf;

use super::{
    Color32, GridMode, Pos2, Rect, Vec2, canvas_size, cell_texture_rect, hex_points,
    point_in_convex_polygon, preview_texture_rect, regular_hex_points, square_rect, style_color,
};
use crate::model::{CellVisual, EditorModel};
use crate::raster::{Rgba, VariantImage};
use seamless_tiler::{AxisBoundaries, CellId, Extent2, WfcStatus};

/// The sheet background, chosen so any uncovered texel is unmistakable.
const BACKGROUND: [u8; 3] = [255, 0, 255];

struct Sheet {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 3]>,
}

impl Sheet {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![BACKGROUND; width * height],
        }
    }

    fn get(&self, x: usize, y: usize) -> [u8; 3] {
        self.pixels[y * self.width + x]
    }

    /// Source-over blending, matching how a renderer composites straight alpha.
    fn blend(&mut self, x: i32, y: i32, color: Rgba) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let alpha = f32::from(color[3]) / 255.0;
        let index = y as usize * self.width + x as usize;
        let under = self.pixels[index];
        self.pixels[index] = std::array::from_fn(|channel| {
            (f32::from(color[channel]) * alpha + f32::from(under[channel]) * (1.0 - alpha)) as u8
        });
    }

    fn fill_polygon(&mut self, polygon: &[Pos2], color: Rgba) {
        let (min, max) = bounds(polygon);
        for y in min.y as i32..=max.y as i32 {
            for x in min.x as i32..=max.x as i32 {
                let point = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                if point_in_convex_polygon(point, polygon) {
                    self.blend(x, y, color);
                }
            }
        }
    }

    fn fill_rect(&mut self, rect: Rect, color: Rgba) {
        for y in rect.min.y as i32..rect.max.y as i32 {
            for x in rect.min.x as i32..rect.max.x as i32 {
                self.blend(x, y, color);
            }
        }
    }

    /// Draws `image` stretched over `rect` with nearest filtering, exactly as
    /// `egui::Painter::image` does with [`egui::TextureOptions::NEAREST`].
    fn draw_image(&mut self, rect: Rect, image: &VariantImage) {
        let [width, height] = image.size();
        for y in rect.min.y as i32..=rect.max.y as i32 {
            for x in rect.min.x as i32..=rect.max.x as i32 {
                let point = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                let local = point - rect.min;
                if local.x < 0.0
                    || local.y < 0.0
                    || local.x >= rect.width()
                    || local.y >= rect.height()
                {
                    continue;
                }
                let column = (local.x / rect.width() * width as f32) as usize;
                let row = (local.y / rect.height() * height as f32) as usize;
                let offset = (row.min(height - 1) * width + column.min(width - 1)) * 4;
                let texel: Rgba = image.rgba()[offset..offset + 4]
                    .try_into()
                    .expect("variant images hold whole rgba texels");
                self.blend(x, y, texel);
            }
        }
    }

    fn write_ppm(&self, path: &std::path::Path) {
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.extend(self.pixels.iter().flatten());
        std::fs::write(path, out).expect("contact sheet directory is writable");
    }
}

fn bounds(polygon: &[Pos2]) -> (Pos2, Pos2) {
    polygon.iter().fold(
        (Pos2::new(f32::MAX, f32::MAX), Pos2::new(f32::MIN, f32::MIN)),
        |(min, max), point| {
            (
                Pos2::new(min.x.min(point.x), min.y.min(point.y)),
                Pos2::new(max.x.max(point.x), max.y.max(point.y)),
            )
        },
    )
}

fn rgba(color: Color32) -> Rgba {
    [color.r(), color.g(), color.b(), 255]
}

/// Renders a solved grid the way [`super::TilerApp::paint_grid`] does: a filled
/// cell polygon, then the collapsed variant's image on top.
fn render_grid(model: &EditorModel, cell_size: f32) -> Sheet {
    let extent = model.extent();
    let size = canvas_size(model.mode(), extent, cell_size);
    let canvas = Rect::from_min_size(Pos2::new(4.0, 4.0), size);
    let mut sheet = Sheet::new(size.x as usize + 8, size.y as usize + 8);

    for index in 0..model.cell_count() {
        let coord = model
            .coordinate(CellId::new(index))
            .expect("topology cells have coordinates");
        let visual = model.cell_visual(coord);
        let CellVisual::Collapsed { variant, .. } = visual else {
            continue;
        };
        let view = model.variant(variant).expect("wave refers to a variant");
        let fill = rgba(style_color(
            model
                .tile_style(view.tile)
                .expect("variant refers to a tile")
                .color,
        ));
        match model.mode() {
            GridMode::Square => {
                sheet.fill_rect(square_rect(canvas, coord, cell_size).shrink(1.0), fill)
            }
            GridMode::Hex => sheet.fill_polygon(&hex_points(canvas, coord, cell_size, 1.0), fill),
        }
        sheet.draw_image(
            cell_texture_rect(model.mode(), canvas, coord, cell_size),
            model
                .variant_image(variant)
                .expect("variants have raster images"),
        );
    }
    sheet
}

/// Renders every distinct orientation of every tile, one row per tile, using the
/// palette's own preview geometry.
fn render_orientations(model: &EditorModel) -> Sheet {
    let cell = 56.0_f32;
    let tiles = model.tiles();
    let columns = tiles
        .iter()
        .map(|(tile, _)| model.variants_for_tile(*tile).len())
        .max()
        .unwrap_or(0);
    let mut sheet = Sheet::new(
        (columns as f32 * cell) as usize + 8,
        (tiles.len() as f32 * cell) as usize + 8,
    );

    for (row, (tile, style)) in tiles.iter().enumerate() {
        for (column, variant) in model.variants_for_tile(*tile).into_iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(4.0 + column as f32 * cell, 4.0 + row as f32 * cell),
                Vec2::splat(cell),
            );
            let fill = rgba(style_color(style.color));
            match model.mode() {
                GridMode::Square => sheet.fill_rect(rect.shrink(2.0), fill),
                GridMode::Hex => sheet.fill_polygon(
                    &regular_hex_points(rect.center(), rect.height() * 0.5 - 2.0),
                    fill,
                ),
            }
            sheet.draw_image(
                preview_texture_rect(model.mode(), rect, 4.0),
                model
                    .variant_image(variant)
                    .expect("variants have raster images"),
            );
        }
    }
    sheet
}

fn solved(mode: GridMode, boundaries: AxisBoundaries, extent: Extent2) -> Option<EditorModel> {
    let mut model = EditorModel::new(extent);
    model.set_mode(mode);
    model.set_boundaries(boundaries);
    for attempt in 0..40 {
        model.set_seed(attempt + 1);
        model.finish();
        if model.status() == Some(WfcStatus::Solved) {
            return Some(model);
        }
    }
    None
}

fn output_dir() -> Option<PathBuf> {
    std::env::var_os("TILER_CONTACT_SHEET").map(PathBuf::from)
}

/// Renders the sheets, asserts a solved hex grid leaves no uncovered seam, and
/// writes the images when `TILER_CONTACT_SHEET` names a directory.
#[test]
fn solved_grids_render_without_uncovered_seams() {
    let dump = output_dir();
    if let Some(dir) = &dump {
        std::fs::create_dir_all(dir).expect("contact sheet directory is creatable");
    }

    let cell_size = 48.0;
    for (mode, name) in [(GridMode::Square, "square"), (GridMode::Hex, "hex")] {
        for (boundaries, suffix) in [
            (AxisBoundaries::BOUNDED, "bounded"),
            (AxisBoundaries::TOROIDAL, "wrapped"),
        ] {
            let model = solved(mode, boundaries, Extent2::new(6, 5))
                .unwrap_or_else(|| panic!("{name} {suffix} demo catalog solves"));
            let sheet = render_grid(&model, cell_size);

            assert_no_interior_holes(&sheet, &format!("{name} {suffix} grid"));
            if let Some(dir) = &dump {
                sheet.write_ppm(&dir.join(format!("{name}-grid-{suffix}.ppm")));
            }

            let orientations = render_orientations(&model);
            if let Some(dir) = &dump {
                orientations.write_ppm(&dir.join(format!("{name}-orientations-{suffix}.ppm")));
            }
        }
    }
}

/// Asserts no background pixel is enclosed by drawn cells.
///
/// Flooding the background inwards from the image border reaches everything
/// *outside* the tiled area — the staggered notches beside hex rows and the
/// sheet margin. Anything left over is a hole the cells failed to cover, which
/// is exactly what a transparent seam looks like to a viewer.
fn assert_no_interior_holes(sheet: &Sheet, label: &str) {
    let mut reached = vec![false; sheet.width * sheet.height];
    let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
    let visit =
        |x: usize, y: usize, reached: &mut Vec<bool>, queue: &mut VecDeque<(usize, usize)>| {
            if sheet.get(x, y) == BACKGROUND && !reached[y * sheet.width + x] {
                reached[y * sheet.width + x] = true;
                queue.push_back((x, y));
            }
        };
    for x in 0..sheet.width {
        visit(x, 0, &mut reached, &mut queue);
        visit(x, sheet.height - 1, &mut reached, &mut queue);
    }
    for y in 0..sheet.height {
        visit(0, y, &mut reached, &mut queue);
        visit(sheet.width - 1, y, &mut reached, &mut queue);
    }
    while let Some((x, y)) = queue.pop_front() {
        let neighbors = [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ];
        for (nx, ny) in neighbors {
            if nx < sheet.width && ny < sheet.height {
                visit(nx, ny, &mut reached, &mut queue);
            }
        }
    }

    let holes = (0..sheet.height)
        .flat_map(|y| (0..sheet.width).map(move |x| (x, y)))
        .filter(|(x, y)| sheet.get(*x, *y) == BACKGROUND && !reached[y * sheet.width + x])
        .count();
    assert_eq!(holes, 0, "{label} left {holes} pixel(s) in uncovered seams");
}
