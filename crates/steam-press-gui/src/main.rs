mod app;
mod state;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([780.0, 520.0])
            .with_title("Steam Press"),
        ..Default::default()
    };

    eframe::run_native(
        "Steam Press",
        options,
        Box::new(|context| Ok(Box::new(app::SteamPressApp::new(context)))),
    )
}
