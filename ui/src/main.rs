mod app;
mod model;
mod raster;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1_100.0, 720.0])
            .with_min_inner_size([720.0, 480.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Seamless Tiler",
        options,
        Box::new(|_creation_context| Ok(Box::new(app::TilerApp::default()))),
    )
}
