mod app;
mod state;
mod updater;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 800.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("Steam Press"),
        ..Default::default()
    };

    eframe::run_native(
        "Steam Press",
        options,
        Box::new(|context| Ok(Box::new(app::SteamPressApp::new(context)))),
    )
}
