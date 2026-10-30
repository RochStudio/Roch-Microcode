//! `--smoke-test <new-BIOS> <old-BIOS> <output-folder>`
//!
//! Loads both images through the UI state, checks the MSI M-FLASH / button
//! filename switching, builds and exports an independently verified package
//! into the output folder, reopens it, then renders all four pages in both
//! themes at the normal and compact window sizes, checks that the key controls
//! stay inside the page and saves a screenshot of each. Writes `ui-smoke.txt`
//! (PASS or the error) and exits with status 0 or 1. The saved theme
//! preference is not changed. Nothing is written to USB drives and nothing is
//! flashed; the USB inventory is only read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};

use egui::{Event, ViewportCommand};
use roch_core::firmware::FirmwareInfo;
use roch_core::package::TransferPackage;
use roch_core::profile::{FLASH_BACK, M_FLASH};
use roch_core::transfer;
use roch_core::usb::UsbDrive;
use roch_core::{CancelToken, Error};

use crate::app::{load_now, Outcome, RochApp, Side};

pub static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

/// Client-area sizes equivalent to the original 1180 × 800 and 1000 × 720 windows.
pub const SIZES: [(&str, f32, f32); 2] = [("normal", 1164.0, 761.0), ("compact", 984.0, 681.0)];

const PROBES: [&[&str]; 4] = [
    &[
        "OldCodes",
        "NewCodes",
        "MethodBox",
        "BoardBox",
        "FlashNameBox",
        "ProfileHint",
        "PlanText",
        "CoverageText",
        "BuildButton",
    ],
    &["VerificationTitle", "VerificationDetail", "Checks", "OutputHash", "OpenPackageButton", "GoUsbButton"],
    &["Drives", "DriveStatus", "ModifiedChoice", "RecoveryChoice", "PackageText", "PrepareButton"],
    &["SystemText", "ImageIdentity", "ImageHash", "MicrocodeGrid", "InspectButton"],
];

/// Always-present probes per page (CoverageText only appears for narrower donors).
const OPTIONAL: [&str; 1] = ["CoverageText"];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Start(u32),
    Building,
    Shooting,
    Finished,
}

pub struct Smoke {
    new_path: PathBuf,
    old_path: PathBuf,
    output: PathBuf,
    stage: Stage,
    error: Option<String>,
    shot: usize,
    frames: u32,
    awaiting: bool,
}

pub struct SmokeBuild {
    package: TransferPackage,
    info: FirmwareInfo,
    drives: Vec<UsbDrive>,
}

struct Shot {
    size: (&'static str, f32, f32),
    light: bool,
    tab: usize,
}

fn shots() -> Vec<Shot> {
    let mut v = Vec::new();
    for size in SIZES {
        for light in [false, true] {
            for tab in 0..4 {
                v.push(Shot { size, light, tab });
            }
        }
    }
    v
}

impl Smoke {
    pub fn new(new_path: PathBuf, old_path: PathBuf, output: PathBuf) -> Self {
        Self { new_path, old_path, output, stage: Stage::Start(0), error: None, shot: 0, frames: 0, awaiting: false }
    }

    pub fn fail(&mut self, message: &str) {
        if self.error.is_none() {
            self.error = Some(message.to_string());
        }
    }

    fn finish(&mut self, ctx: &egui::Context) {
        let _ = std::fs::create_dir_all(&self.output);
        let path = self.output.join("ui-smoke.txt");
        match &self.error {
            None => {
                let _ = std::fs::write(
                    path,
                    "PASS: BIOS transfer, independent verification, package export; four tabs in both themes at 1180x800 and 1000x720; key controls fit without page scrolling. Theme preference was not changed. No USB writes.",
                );
            }
            Some(e) => {
                let _ = std::fs::write(path, format!("FAIL: {e}"));
                EXIT_CODE.store(1, Ordering::SeqCst);
            }
        }
        self.stage = Stage::Finished;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}

fn start(app: &mut RochApp, ctx: &egui::Context) -> Result<(), String> {
    let smoke = app.smoke.as_ref().expect("smoke");
    let (new_path, old_path, output) = (smoke.new_path.clone(), smoke.old_path.clone(), smoke.output.clone());
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let (old, _) = load_now(&old_path, Side::Old).map_err(|e| e.to_string())?;
    app.set_input(old, None, Side::Old);
    let (new, asus) = load_now(&new_path, Side::New).map_err(|e| e.to_string())?;
    app.set_input(new, asus, Side::New);
    let newer = app.newer.clone().expect("new BIOS");
    if newer.suggested_vendor() == "MSI" {
        if app.profile().method.as_deref() != Some(M_FLASH) || app.flash_name != newer.file_name {
            return Err("M-FLASH default filename failed".into());
        }
        app.set_method(FLASH_BACK);
        if app.flash_name != "MSI.ROM" {
            return Err("Button filename failed".into());
        }
        app.set_method(M_FLASH);
        if app.flash_name != newer.file_name || !app.flash_read_only {
            return Err("M-FLASH filename restoration failed".into());
        }
    }
    let donor = app.selected_old().cloned().ok_or("No donor microcode is selected")?;
    let target = app.selected_new().cloned().ok_or("No compatible target microcode is selected")?;
    let older = app.older.clone().expect("old BIOS");
    let profile = app.profile();
    app.run(ctx, false, false, move |_| {
        let result = transfer::build(&newer.bytes, &older.bytes, donor.offset, target.offset)?;
        let package = TransferPackage::export(&result, &profile, &output, None, &CancelToken::none())?;
        let reopened = TransferPackage::open(&package.directory)?;
        let same_name = |p: PathBuf| p.file_name().is_some_and(|n| n.to_string_lossy() == profile.file_name);
        if reopened.manifest.profile.method != profile.method
            || !same_name(reopened.candidate_path())
            || !same_name(reopened.recovery_path())
        {
            return Err(Error::invalid("Flash method or filenames did not survive package export/reload"));
        }
        let info = roch_core::firmware::inspect(&result.image, &profile.file_name);
        let drives = crate::services::drives()?;
        Ok(Outcome::Smoke(Box::new(SmokeBuild { package, info, drives })))
    });
    Ok(())
}

/// Called when the background build of the smoke test completes.
pub fn built(app: &mut RochApp, _ctx: &egui::Context, build: SmokeBuild) {
    app.show_package(&build.package);
    app.show_inspection(&build.info);
    app.package = Some(build.package);
    app.drives = build.drives;
    app.set_status("UI test • all verification passed. No USB or firmware writes.");
    if let Some(s) = app.smoke.as_mut() {
        s.stage = Stage::Shooting;
        s.shot = 0;
        s.frames = 0;
    }
}

fn check_layout(app: &RochApp, shot: &Shot) -> Result<(), String> {
    let page = app.page_rect;
    for name in PROBES[shot.tab] {
        let Some(rect) = app.probes.get(name) else {
            if OPTIONAL.contains(name) {
                continue;
            }
            return Err(format!("Missing control: {}, tab {}, {name}", shot.size.0, shot.tab));
        };
        if rect.left() < page.left() - 1.0
            || rect.top() < page.top() - 1.0
            || rect.right() > page.right() + 1.0
            || rect.bottom() > page.bottom() + 1.0
        {
            return Err(format!("Layout overflow: {}, tab {}, {name}", shot.size.0, shot.tab));
        }
    }
    Ok(())
}

/// Advance the smoke test by one frame.
pub fn step(app: &mut RochApp, ctx: &egui::Context) {
    let Some(stage) = app.smoke.as_ref().map(|s| s.stage) else { return };
    if stage == Stage::Finished {
        return;
    }
    ctx.request_repaint();
    if app.smoke.as_ref().is_some_and(|s| s.error.is_some()) {
        app.smoke.as_mut().expect("smoke").finish(ctx);
        return;
    }
    match stage {
        Stage::Start(n) if n < 3 => app.smoke.as_mut().expect("smoke").stage = Stage::Start(n + 1),
        Stage::Start(_) => {
            app.smoke.as_mut().expect("smoke").stage = Stage::Building;
            if let Err(e) = start(app, ctx) {
                app.smoke.as_mut().expect("smoke").fail(&e);
            }
        }
        Stage::Building => {}
        Stage::Shooting => shoot(app, ctx),
        Stage::Finished => {}
    }
}

fn shoot(app: &mut RochApp, ctx: &egui::Context) {
    let plan = shots();
    let (index, frames, awaiting) = {
        let s = app.smoke.as_ref().expect("smoke");
        (s.shot, s.frames, s.awaiting)
    };
    if index >= plan.len() {
        app.smoke.as_mut().expect("smoke").finish(ctx);
        return;
    }
    let shot = &plan[index];
    if awaiting {
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let output = app.smoke.as_ref().expect("smoke").output.clone();
            let name = format!("{}-{}-tab-{}.png", shot.size.0, if shot.light { "light" } else { "dark" }, shot.tab);
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            let saved = image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, rgba)
                .ok_or_else(|| "Screenshot buffer has an unexpected size".to_string())
                .and_then(|img| img.save(output.join(&name)).map_err(|e| e.to_string()));
            let s = app.smoke.as_mut().expect("smoke");
            if let Err(e) = saved {
                s.fail(&format!("Could not save {name}: {e}"));
                return;
            }
            s.awaiting = false;
            s.shot += 1;
            s.frames = 0;
        }
        return;
    }
    if frames == 0 {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(shot.size.1, shot.size.2)));
        if app.light != shot.light {
            app.set_theme(ctx, shot.light, false);
        }
        app.tab = shot.tab;
    }
    app.smoke.as_mut().expect("smoke").frames += 1;
    let size = ctx.input(|i| i.content_rect().size());
    let settled = (size.x - shot.size.1).abs() <= 1.5 && (size.y - shot.size.2).abs() <= 1.5;
    if frames < 6 || (!settled && frames < 120) {
        return;
    }
    if !settled {
        app.smoke.as_mut().expect("smoke").fail(&format!(
            "The window could not be resized to {}x{} (got {}x{})",
            shot.size.1, shot.size.2, size.x, size.y
        ));
        return;
    }
    if let Err(e) = check_layout(app, shot) {
        app.smoke.as_mut().expect("smoke").fail(&e);
        return;
    }
    app.smoke.as_mut().expect("smoke").awaiting = true;
    ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
}
