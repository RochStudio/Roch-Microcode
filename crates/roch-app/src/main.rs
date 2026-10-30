//! Roch Microcode: Intel BIOS microcode transfer, independent verification and USB preparation.
//!
//! Copyright (C) 2026 Roch Studio. Licensed GPL-3.0-or-later.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod platform;
mod services;
mod settings;
mod smoke;
mod theme;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

static ICON: &[u8] = include_bytes!("../../../assets/RochMicrocode.ico");

/// The largest PNG image in the .ico file.
fn icon() -> Option<egui::IconData> {
    let count = u16::from_le_bytes([*ICON.get(4)?, *ICON.get(5)?]) as usize;
    let mut best: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let e = 6 + i * 16;
        let entry = ICON.get(e..e + 16)?;
        let width = if entry[0] == 0 { 256 } else { entry[0] as u32 };
        let len = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        let data = ICON.get(offset..offset + len)?;
        if data.starts_with(b"\x89PNG") && best.is_none_or(|(w, _)| width > w) {
            best = Some((width, data));
        }
    }
    eframe::icon_data::from_png_bytes(best?.1).ok()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let smoke = if args.len() == 5 && args[1] == "--smoke-test" {
        Some(smoke::Smoke::new(PathBuf::from(&args[2]), PathBuf::from(&args[3]), PathBuf::from(&args[4])))
    } else {
        None
    };
    // Fit the initial window to the available desktop (title bar and borders excluded).
    let (mut width, mut height) = (smoke::SIZES[0].1, smoke::SIZES[0].2);
    if let Some((w, h)) = platform::work_area_points() {
        width = width.min(w - 16.0);
        height = height.min(h - 39.0);
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(app::TITLE)
        .with_inner_size([width, height])
        .with_min_inner_size([smoke::SIZES[1].1, smoke::SIZES[1].2 - 1.0])
        .with_drag_and_drop(true);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    let result =
        eframe::run_native(app::TITLE, options, Box::new(move |cc| Ok(Box::new(app::RochApp::new(cc, smoke)))));
    if let Err(e) = result {
        #[cfg(windows)]
        platform::warning_box(0, app::TITLE, &format!("Roch Microcode could not start: {e}"));
        #[cfg(not(windows))]
        eprintln!("Roch Microcode could not start: {e}");
        std::process::exit(1);
    }
    std::process::exit(smoke::EXIT_CODE.load(Ordering::SeqCst));
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_decodes() {
        let icon = super::icon().expect("icon");
        assert_eq!(icon.width, 256);
        assert_eq!(icon.height, 256);
    }
}
