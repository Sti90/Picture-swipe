// Hides the terminal window if this is ever built for Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backend;
mod decisions;
mod format;
mod grid;
mod images;
mod library;
mod menu;
mod photokit;
mod scan;
mod store;
mod sweep;
mod thumbs;
mod trash;
mod widgets;

use std::sync::Arc;

use eframe::egui::{self, Color32};
use objc2_app_kit::{NSColor, NSColorSpace, NSWorkspace};

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default();
    // Without this eframe replaces the Dock icon with its own logo while the app runs.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: viewport
            .with_title("Bildesveip")
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([820.0, 560.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Bildesveip",
        options,
        Box::new(|cc| {
            install_system_font(&cc.egui_ctx);
            let menu = menu::NativeMenu::install(&cc.egui_ctx).map_err(|e| e.to_string())?;
            let settings = app::SystemSettings {
                accent: system_accent(),
                reduce_motion: reduce_motion(),
            };
            let paths = backend::Paths::standard().ok_or("Fant ikke hjemmemappen.")?;
            Ok(Box::new(Bildesveip(app::App::new(
                &cc.egui_ctx,
                paths,
                menu,
                settings,
            )?)))
        }),
    )
}

struct Bildesveip(app::App);

impl eframe::App for Bildesveip {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.0.tick(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.show(ui);
    }

    fn on_exit(&mut self) {
        self.0.finish();
    }
}

/// The accent colour the user picked in System Settings.
fn system_accent() -> Color32 {
    let fallback = Color32::from_rgb(0x0a, 0x84, 0xff);
    let Some(color) =
        NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
    else {
        return fallback;
    };
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(
        channel(color.redComponent()),
        channel(color.greenComponent()),
        channel(color.blueComponent()),
    )
}

fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Reads the font from the system at runtime, so no Apple font ships with the app.
pub fn install_system_font(ctx: &egui::Context) {
    let Some(bytes) = [
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ]
    .iter()
    .find_map(|path| std::fs::read(path).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "system".to_owned(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "system".to_owned());
    ctx.set_fonts(fonts);
}
