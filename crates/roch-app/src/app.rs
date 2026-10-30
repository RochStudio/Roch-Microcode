//! Main window: the four pages, background jobs and the drag-and-drop targets.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use egui::{
    vec2, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense,
    Stroke, TextWrapMode, Ui, UiBuilder,
};
use roch_core::firmware::{self, recipe, FirmwareInfo, Microcode};
use roch_core::format::{decimals, n0};
use roch_core::input::FirmwareInput;
use roch_core::package::TransferPackage;
use roch_core::profile::{FlashProfile, FLASH_BACK, M_FLASH, VENDORS};
use roch_core::transfer::{self, can_replace, compatibility_detail};
use roch_core::usb::UsbDrive;
use roch_core::{paths, verification, CancelToken, Error, Result};

use crate::platform::{self, Hwnd};
use crate::smoke::Smoke;
use crate::theme::{self, bold, semibold, Palette};
use crate::{services, settings};

pub const TITLE: &str = "Roch Microcode";
pub const VERSION_TEXT: &str = concat!("Version ", env!("CARGO_PKG_VERSION"));
pub const LINKS: [(&str, &str, &str); 3] = [
    ("YouTube", "https://www.youtube.com/@MateoPcTech", "MateoPcTech on YouTube"),
    ("X", "https://x.com/MateoPCTech", "MateoPCTech on X"),
    ("Discord", "https://discord.gg/KfzExpKQHB", "Join the Discord community"),
];
pub const TABS: [&str; 4] = ["01  Transfer", "02  Verification", "03  Prepare USB", "Inspect & system"];

const DEFAULT_VERIFICATION_TITLE: &str = "Verification results will appear here";
const DEFAULT_BOARD_GUIDE: &str =
    "Confirm the selected flash method and filename in the board manual. Keep a separate recovery drive.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Old,
    New,
}

#[derive(Clone, Debug)]
pub struct CheckView {
    pub name: String,
    pub detail: String,
    pub full_detail: String,
}

/// Results of background jobs.
pub enum Outcome {
    Loaded { input: FirmwareInput, asus_name: Option<String>, side: Side },
    Inspected(FirmwareInfo),
    Built { package: TransferPackage, info: FirmwareInfo },
    Drives(Vec<UsbDrive>),
    PackageOpened(TransferPackage),
    Prepared(String),
    Smoke(Box<crate::smoke::SmokeBuild>),
}

#[allow(clippy::large_enum_variant)] // One message per job; boxing gains nothing.
pub enum Msg {
    Status(String),
    /// Progress that also updates the verification detail line.
    BuildProgress(String),
    Done(Result<Outcome>),
    System(Result<String>),
}

/// Context handed to a background job.
pub struct JobContext {
    tx: Sender<Msg>,
    ctx: egui::Context,
    pub cancel: CancelToken,
    build_progress: bool,
}

impl JobContext {
    pub fn progress(&self, text: &str) {
        let msg = if self.build_progress { Msg::BuildProgress(text.into()) } else { Msg::Status(text.into()) };
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

pub struct RochApp {
    pub pal: Palette,
    pub light: bool,
    pub hwnd: Hwnd,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub busy: bool,
    pub cancellable: bool,
    cancel: Option<CancelToken>,
    pending_warning: Option<String>,
    pub tab: usize,

    pub older: Option<Arc<FirmwareInput>>,
    pub newer: Option<Arc<FirmwareInput>>,
    newer_asus_name: Option<String>,
    pub package: Option<TransferPackage>,

    old_name: String,
    old_detail: String,
    new_name: String,
    new_detail: String,
    pub old_codes: Vec<Microcode>,
    pub old_sel: Option<usize>,
    pub new_codes: Vec<Microcode>,
    pub new_sel: Option<usize>,
    pub vendor: String,
    pub methods: Vec<&'static str>,
    pub method: String,
    pub board: String,
    pub flash_name: String,
    pub flash_read_only: bool,
    profile_hint: String,
    plan_text: String,
    coverage_text: String,

    verification_title: String,
    verification_detail: String,
    checks: Vec<CheckView>,
    output_hash: String,

    pub drives: Vec<UsbDrive>,
    drive_sel: Option<usize>,
    drive_status: String,
    recovery_choice: bool,
    usb_result: String,
    package_text: String,
    package_tooltip: String,
    board_guide: String,

    system_text: String,
    image_identity: String,
    image_hash: String,
    grid: Vec<Microcode>,

    pub status: String,

    old_card: Rect,
    new_card: Rect,
    drop_target: Option<Side>,

    pub smoke: Option<Smoke>,
    /// Bounds of key controls on the current page (layout checks).
    pub probes: HashMap<&'static str, Rect>,
    pub page_rect: Rect,
    heights: HashMap<&'static str, f32>,
    title_bar_applied: Option<bool>,
}

fn rich(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text).size(size).color(color)
}

fn file_stem(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

impl RochApp {
    pub fn new(cc: &eframe::CreationContext<'_>, smoke: Option<Smoke>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let light = settings::load_light_theme();
        let pal = theme::palette(light);
        theme::apply(&cc.egui_ctx, &pal, light);
        let (tx, rx) = channel();
        let hwnd = platform::hwnd_of_creation(cc);
        let mut app = Self {
            pal,
            light,
            hwnd,
            tx,
            rx,
            busy: false,
            cancellable: false,
            cancel: None,
            pending_warning: None,
            tab: 0,
            older: None,
            newer: None,
            newer_asus_name: None,
            package: None,
            old_name: "No old BIOS selected".into(),
            old_detail: "Intel CAP / BIN / ROM / vendor file / ZIP".into(),
            new_name: "No new BIOS selected".into(),
            new_detail: "Use the exact motherboard model and revision".into(),
            old_codes: Vec::new(),
            old_sel: None,
            new_codes: Vec::new(),
            new_sel: None,
            vendor: String::new(),
            methods: Vec::new(),
            method: String::new(),
            board: String::new(),
            flash_name: String::new(),
            flash_read_only: false,
            profile_hint: "Check model, flash method and filename against the board manual.".into(),
            plan_text: "Choose the old and new BIOS to start.".into(),
            coverage_text: String::new(),
            verification_title: DEFAULT_VERIFICATION_TITLE.into(),
            verification_detail: "UEFIExtract and MCExtractor are bundled. Build a package to view all eight checks."
                .into(),
            checks: Vec::new(),
            output_hash: String::new(),
            drives: Vec::new(),
            drive_sel: None,
            drive_status: "Existing FAT32 / MBR USB drive with one partition.".into(),
            recovery_choice: false,
            usb_result: String::new(),
            package_text: "Build and verify a package, or open an existing version 2 package.".into(),
            package_tooltip: String::new(),
            board_guide: DEFAULT_BOARD_GUIDE.into(),
            system_text: "Reading system information…".into(),
            image_identity: "No image selected".into(),
            image_hash: String::new(),
            grid: Vec::new(),
            status: "Ready • drop an old BIOS and a new BIOS to begin.".into(),
            old_card: Rect::NOTHING,
            new_card: Rect::NOTHING,
            drop_target: None,
            smoke,
            probes: HashMap::new(),
            page_rect: Rect::NOTHING,
            heights: HashMap::new(),
            title_bar_applied: None,
        };
        app.set_methods("");
        // System information loads in the background, like the original Loaded handler.
        let tx = app.tx.clone();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::System(services::system_info()));
            ctx.request_repaint();
        });
        app
    }

    // ---------------------------------------------------------------- state

    pub fn selected_old(&self) -> Option<&Microcode> {
        self.old_sel.and_then(|i| self.old_codes.get(i))
    }
    pub fn selected_new(&self) -> Option<&Microcode> {
        self.new_sel.and_then(|i| self.new_codes.get(i))
    }

    fn build_enabled(&self) -> bool {
        match (self.selected_old(), self.selected_new()) {
            (Some(d), Some(t)) => !self.busy && can_replace(d, t) && d.revision != t.revision && d.size <= t.size,
            _ => false,
        }
    }

    fn prepare_enabled(&self) -> bool {
        !self.busy
            && self.package.is_some()
            && self.drive_sel.and_then(|i| self.drives.get(i)).is_some_and(|d| d.rejection().is_none())
    }

    pub fn profile(&self) -> FlashProfile {
        FlashProfile {
            vendor: self.vendor.clone(),
            board: self.board.trim().to_string(),
            file_name: self.flash_name.trim().to_string(),
            method: Some(if self.method.is_empty() { FLASH_BACK.to_string() } else { self.method.clone() }),
            original_file_name: self.newer.as_ref().map(|n| n.file_name.clone()),
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
    }

    pub fn invalidate(&mut self) {
        self.package = None;
        self.checks.clear();
        self.output_hash.clear();
        self.verification_title = DEFAULT_VERIFICATION_TITLE.into();
        self.verification_detail = "The current selection needs a new build and verification.".into();
        self.package_text = "Build and verify the current selection, or open an existing version 2 package.".into();
        self.package_tooltip.clear();
        self.board_guide = DEFAULT_BOARD_GUIDE.into();
        self.usb_result.clear();
    }

    pub fn set_input(&mut self, input: FirmwareInput, asus_name: Option<String>, side: Side) {
        let input = Arc::new(input);
        let detail = format!(
            "{} MB • {} microcodes found",
            decimals(input.bytes.len() as f64 / 1_048_576.0, 2),
            input.info.microcodes.len()
        );
        match side {
            Side::Old => {
                self.old_name = input.file_name.clone();
                self.old_detail = detail;
                let valid: Vec<Microcode> =
                    input.info.microcodes.iter().filter(|m| m.checksum_valid).cloned().collect();
                let cpu = platform::cpuid_signature();
                let preferred = valid.iter().position(|m| m.cpuid == cpu && m.revision == 0x11f).or_else(|| {
                    let mut best: Option<usize> = None;
                    for (i, m) in valid.iter().enumerate() {
                        if m.cpuid == cpu && best.is_none_or(|b| m.revision > valid[b].revision) {
                            best = Some(i);
                        }
                    }
                    best
                });
                self.old_sel = preferred.or(if valid.is_empty() { None } else { Some(0) });
                self.old_codes = valid;
                self.older = Some(input.clone());
            }
            Side::New => {
                self.new_name = input.file_name.clone();
                self.new_detail = detail;
                let vendor = input.suggested_vendor();
                self.vendor = vendor.to_string();
                self.board = if input.info.is_supported_base() {
                    recipe::BOARD.to_string()
                } else {
                    file_stem(&input.file_name).replace('_', " ")
                };
                self.newer = Some(input.clone());
                self.newer_asus_name = asus_name;
                self.set_methods(vendor);
            }
        }
        self.invalidate();
        self.update_targets();
        self.show_inspection(&input.info);
    }

    pub fn update_targets(&mut self) {
        let current = self.selected_new().map(|m| m.offset);
        let mut choices: Vec<Microcode> = match (self.selected_old(), &self.newer) {
            (Some(donor), Some(newer)) => {
                newer.info.microcodes.iter().filter(|t| can_replace(donor, t)).cloned().collect()
            }
            _ => Vec::new(),
        };
        choices.sort_by_key(|m| std::cmp::Reverse(m.revision));
        self.new_sel =
            choices.iter().position(|t| Some(t.offset) == current).or(if choices.is_empty() { None } else { Some(0) });
        self.new_codes = choices;
        self.update_plan();
    }

    pub fn update_plan(&mut self) {
        self.plan_text = match (self.selected_old(), self.selected_new()) {
            (Some(d), Some(t)) => {
                if d.revision == t.revision {
                    "These revisions already match. Choose a different microcode.".into()
                } else if d.size > t.size {
                    format!(
                        "The old microcode is {} bytes too large for this slot. Relocation is not supported.",
                        n0((d.size - t.size) as i64)
                    )
                } else {
                    format!(
                        "CPUID {:X}: replace {} with {} in the new BIOS.",
                        d.cpuid,
                        t.revision_text(),
                        d.revision_text()
                    )
                }
            }
            _ => {
                if self.older.is_some() && self.newer.is_some() {
                    "No compatible target. The donor needs the same primary CPU signature and a platform mask equal to or contained in the new patch's mask.".into()
                } else {
                    "Choose the old and new BIOS to start.".into()
                }
            }
        };
        self.coverage_text = match (self.selected_old(), self.selected_new()) {
            (Some(s), Some(d)) if s.platforms != d.platforms => compatibility_detail(s, d),
            _ => String::new(),
        };
    }

    pub fn set_methods(&mut self, vendor: &str) {
        self.methods = FlashProfile::methods(vendor).to_vec();
        self.method = self.methods[0].to_string();
        self.set_flash_name();
    }

    pub fn set_flash_name(&mut self) {
        let method = if self.method.is_empty() { FLASH_BACK } else { self.method.as_str() };
        self.flash_name = FlashProfile::suggested_name(
            &self.vendor,
            method,
            self.newer.as_ref().map(|n| n.file_name.as_str()),
            self.newer_asus_name.as_deref(),
        );
        self.flash_read_only = method == M_FLASH;
        self.profile_hint = if method == M_FLASH {
            "M-FLASH keeps the NEW BIOS filename. MSI.ROM is only for the Flash BIOS Button. Modified firmware may be rejected."
        } else {
            "Check button support and filename against the board manual. ASUS filenames are model-specific."
        }
        .into();
    }

    pub fn set_method(&mut self, method: &str) {
        self.method = method.to_string();
        self.set_flash_name();
        self.invalidate();
    }

    pub fn show_package(&mut self, p: &TransferPackage) {
        self.checks = p
            .manifest
            .checks
            .iter()
            .map(|c| CheckView {
                name: c.name.clone(),
                detail: if c.name == "CPU compatibility" {
                    format!("{}.", c.detail.split(". ").next().unwrap_or(""))
                } else {
                    c.detail.clone()
                },
                full_detail: c.detail.clone(),
            })
            .collect();
        let m = &p.manifest;
        self.verification_title = format!("Verified • 0x{:X} → 0x{:X}", m.from_revision, m.to_revision);
        self.verification_detail = format!(
            "{} • {}\nOriginal firmware preserved; donor microcode transferred and independently checked.",
            m.profile.vendor, m.profile.board
        );
        self.output_hash = format!("SHA-256  {}\n{}", m.candidate_hash, p.directory.display());
        let scope =
            m.checks.iter().find(|c| c.name == "CPU compatibility").map(|c| c.detail.clone()).unwrap_or_default();
        self.package_text = format!(
            "{} • {}\n{}\nUSB filename: {}\n\n{scope}",
            m.profile.vendor,
            m.profile.board,
            m.profile.method_str(),
            m.profile.file_name
        );
        self.board_guide = m.profile.board_instructions().into();
        self.package_tooltip = format!("{}\n\n{scope}", p.directory.display());
    }

    pub fn show_inspection(&mut self, info: &FirmwareInfo) {
        self.image_identity = info.file_name.clone();
        self.image_hash = format!("{} bytes\nSHA-256  {}", n0(info.size as i64), info.sha256);
        self.grid = info.microcodes.clone();
    }

    pub fn set_drives(&mut self, drives: Vec<UsbDrive>) {
        if self.drive_sel.is_some() {
            self.usb_result.clear();
        }
        self.drive_status = if drives.is_empty() {
            "No mounted USB drives found. Connect one and refresh.".into()
        } else {
            "Select the USB destination.".into()
        };
        self.status = format!("Found {} USB volume(s).", drives.len());
        self.drives = drives;
        self.drive_sel = None;
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, light: bool, persist: bool) -> bool {
        self.light = light;
        self.pal = theme::palette(light);
        theme::apply(ctx, &self.pal, light);
        platform::apply_title_bar(self.hwnd, light, self.pal.canvas, self.pal.text, self.pal.outline);
        self.title_bar_applied = Some(light);
        if persist {
            settings::save_light_theme(light)
        } else {
            true
        }
    }

    // ---------------------------------------------------------------- jobs

    /// Equivalent of the original `Run(action, cancellable)`.
    pub fn run(
        &mut self,
        ctx: &egui::Context,
        cancellable: bool,
        build_progress: bool,
        job: impl FnOnce(&JobContext) -> Result<Outcome> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.cancellable = cancellable;
        let cancel = CancelToken::new();
        self.cancel = Some(cancel.clone());
        let jc = JobContext { tx: self.tx.clone(), ctx: ctx.clone(), cancel, build_progress };
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&jc)))
                .unwrap_or_else(|_| Err(Error::io("An internal error stopped the operation.")));
            let _ = jc.tx.send(Msg::Done(result));
            jc.ctx.request_repaint();
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Status(text) => self.status = text,
                Msg::BuildProgress(text) => {
                    self.verification_detail = text.clone();
                    self.status = text;
                }
                Msg::System(Ok(text)) => self.system_text = text,
                Msg::System(Err(_)) => {
                    self.system_text =
                        "System information unavailable. Use the target board's exact model and revision.".into()
                }
                Msg::Done(result) => {
                    self.busy = false;
                    self.cancellable = false;
                    self.cancel = None;
                    match result {
                        Ok(outcome) => self.complete(ctx, outcome),
                        Err(Error::Cancelled) => {
                            self.status = "Cancelled. An incomplete build is not accepted for USB preparation.".into();
                            if self.tab == 1 && self.package.is_none() {
                                self.verification_title = "Verification cancelled".into();
                            }
                            if let Some(s) = self.smoke.as_mut() {
                                s.fail("Smoke test cancelled");
                            }
                        }
                        Err(e) => {
                            self.status = format!("Stopped • {e}");
                            if self.tab == 1 && self.package.is_none() {
                                self.verification_title = "Verification did not complete".into();
                                self.verification_detail = e.to_string();
                            }
                            if let Some(s) = self.smoke.as_mut() {
                                s.fail(&e.to_string());
                            } else {
                                self.pending_warning = Some(e.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    fn complete(&mut self, ctx: &egui::Context, outcome: Outcome) {
        match outcome {
            Outcome::Loaded { input, asus_name, side } => {
                let empty = input.info.microcodes.is_empty();
                self.set_input(input, asus_name, side);
                self.status = if empty {
                    "No uncompressed Intel microcodes found. AMD or compressed images cannot be edited by this version."
                        .into()
                } else {
                    "Image loaded. Choose the microcode to transfer.".into()
                };
            }
            Outcome::Inspected(info) => {
                self.show_inspection(&info);
                self.status = "Inspection complete.".into();
            }
            Outcome::Built { package, info } => {
                self.show_package(&package);
                self.show_inspection(&info);
                self.package = Some(package);
                self.status = "All 8 verification checks passed. Modified and recovery images are ready.".into();
            }
            Outcome::Drives(drives) => self.set_drives(drives),
            Outcome::PackageOpened(package) => {
                self.show_package(&package);
                self.package = Some(package);
                self.status = "Package independently reverified from its actual firmware files.".into();
            }
            Outcome::Prepared(text) => {
                self.usb_result = text;
                self.status = "USB prepared and read-back hash verified.".into();
            }
            Outcome::Smoke(build) => crate::smoke::built(self, ctx, *build),
        }
    }

    fn load(&mut self, ctx: &egui::Context, path: PathBuf, side: Side) {
        self.status = "Reading BIOS and finding Intel microcodes…".into();
        self.run(ctx, false, false, move |_| {
            let input = FirmwareInput::load(&path)?;
            let asus_name = if side == Side::New { input.suggested_asus_name() } else { None };
            Ok(Outcome::Loaded { input, asus_name, side })
        });
    }

    fn pick_image(&self, title: &str) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(title).add_filter("BIOS files and ZIP packages", &["*"]);
        if let Some(h) = self.parent() {
            dialog = dialog.set_parent(&h);
        }
        dialog.pick_file()
    }

    fn pick_folder(&self, title: &str) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(h) = self.parent() {
            dialog = dialog.set_parent(&h);
        }
        dialog.pick_folder()
    }

    fn parent(&self) -> Option<ParentWindow> {
        (self.hwnd != 0).then_some(ParentWindow(self.hwnd))
    }

    fn build_clicked(&mut self, ctx: &egui::Context) {
        let Some(parent) = self.pick_folder("Save the verified BIOS package in…") else { return };
        let (Some(d), Some(t)) = (self.selected_old().cloned(), self.selected_new().cloned()) else { return };
        let profile = self.profile();
        if let Err(e) = profile.validate() {
            self.status = format!("Stopped • {e}");
            self.pending_warning = Some(e.to_string());
            return;
        }
        self.package = None;
        self.checks.clear();
        self.verification_title = "Building and verifying…".into();
        self.tab = 1;
        let (newer, older) = (self.newer.clone().expect("new BIOS"), self.older.clone().expect("old BIOS"));
        self.run(ctx, true, true, move |jc| {
            let result = transfer::build(&newer.bytes, &older.bytes, d.offset, t.offset)?;
            jc.cancel.check()?;
            let progress = |text: &str| jc.progress(text);
            let package = TransferPackage::export(&result, &profile, &parent, Some(&progress), &jc.cancel)?;
            let info = firmware::inspect(&result.image, &profile.file_name);
            Ok(Outcome::Built { package, info })
        });
    }

    fn inspect_clicked(&mut self, ctx: &egui::Context) {
        let Some(file) = self.pick_image("Inspect firmware or ZIP") else { return };
        self.run(ctx, false, false, move |_| Ok(Outcome::Inspected(FirmwareInput::load(&file)?.info)));
    }

    pub fn refresh_drives(&mut self, ctx: &egui::Context) {
        self.run(ctx, false, false, |_| Ok(Outcome::Drives(services::drives()?)));
    }

    fn load_package_clicked(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.pick_folder("Open a Roch Microcode version 2 package") else { return };
        self.package = None;
        self.run(ctx, true, false, move |jc| {
            let opened = TransferPackage::open(&dir)?;
            let result = opened.revalidate()?;
            let work = paths::new_verification_job();
            let progress = |text: &str| jc.progress(text);
            let evidence = verification::verify(&result, &work, Some(&progress), &jc.cancel)?;
            let checks = result.checks.iter().chain(evidence.checks.iter()).cloned().collect();
            Ok(Outcome::PackageOpened(opened.with_checks(checks)))
        });
    }

    fn prepare_clicked(&mut self, ctx: &egui::Context) {
        let (Some(package), Some(drive)) =
            (self.package.clone(), self.drive_sel.and_then(|i| self.drives.get(i)).cloned())
        else {
            return;
        };
        let profile = &package.manifest.profile;
        let recovery = self.recovery_choice;
        let message = format!(
            "Prepare {}\n\n{} • {}\nMethod: {}\nWrite {} as {}{}?\n\n{}\n\nAn existing file at this path will be backed up locally and replaced. Other files remain. The app will not flash the motherboard.",
            drive.display(),
            profile.vendor,
            profile.board,
            profile.method_str(),
            if recovery { "UNTOUCHED NEW BIOS / RECOVERY" } else { "MODIFIED BIOS" },
            drive.root,
            profile.file_name,
            profile.board_instructions()
        );
        if !platform::confirm_box(self.hwnd, "Confirm USB destination and board", &message) {
            return;
        }
        self.run(ctx, false, false, move |jc| {
            let progress = |text: &str| jc.progress(text);
            let text =
                roch_core::usb::prepare(&drive, &package, recovery, &services::drives, Some(&progress), &jc.cancel)?;
            Ok(Outcome::Prepared(text))
        });
    }

    fn open(&mut self, target: &str) {
        if let Err(e) = platform::shell_open(target) {
            self.status = format!("Could not open the link in your default browser: {e}");
        }
    }

    // ---------------------------------------------------------------- frame

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let (hovered, dropped) = ctx.input(|i| (i.raw.hovered_files.len(), i.raw.dropped_files.clone()));
        if hovered == 0 && dropped.is_empty() {
            self.drop_target = None;
            return;
        }
        let ppp = ctx.pixels_per_point();
        let pos = platform::cursor_client_pos(self.hwnd)
            .map(|(x, y)| Pos2::new(x / ppp, y / ppp))
            .or_else(|| ctx.input(|i| i.pointer.hover_pos()));
        let side = match pos {
            Some(p) if self.tab == 0 && self.old_card.contains(p) => Some(Side::Old),
            Some(p) if self.tab == 0 && self.new_card.contains(p) => Some(Side::New),
            _ => None,
        };
        self.drop_target = if self.busy || hovered > 1 { None } else { side };
        if hovered > 0 {
            ctx.request_repaint();
        }
        if !dropped.is_empty() {
            self.drop_target = None;
            if self.busy || dropped.len() != 1 {
                return;
            }
            if let (Some(side), Some(path)) = (side, dropped[0].path.clone()) {
                self.load(ctx, path, side);
            }
        }
    }

    fn probe(&mut self, name: &'static str, rect: Rect) {
        if self.smoke.is_some() {
            self.probes.insert(name, rect);
        }
    }

    fn card(&self, margin: Margin) -> Frame {
        Frame::new()
            .fill(self.pal.surface)
            .stroke(Stroke::new(1.0_f32, self.pal.outline))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(margin)
    }

    /// A card occupying exactly `rect`.
    fn card_at<R>(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        margin: Margin,
        stroke: Option<Color32>,
        add: impl FnOnce(&mut Self, &mut Ui) -> R,
    ) -> R {
        let mut frame = self.card(margin);
        if let Some(c) = stroke {
            frame = frame.stroke(Stroke::new(1.5_f32, c));
        }
        ui.scope_builder(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)), |ui| {
            frame
                .show(ui, |ui| {
                    let inner = rect.size() - margin.sum() - vec2(2.0, 2.0);
                    ui.set_min_size(inner);
                    ui.set_max_size(inner);
                    add(self, ui)
                })
                .inner
        })
        .inner
    }

    /// A full-width card whose height follows its content.
    fn card_auto<R>(
        &mut self,
        ui: &mut Ui,
        margin: Margin,
        add: impl FnOnce(&mut Self, &mut Ui) -> R,
    ) -> egui::InnerResponse<R> {
        let width = ui.available_width();
        self.card(margin).show(ui, |ui| {
            ui.set_width(width - margin.sum().x - 2.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| add(self, ui)).inner
        })
    }

    /// A card spanning `left..right` whose bottom edge is at `bottom` and whose
    /// height follows its content (measured on the previous pass).
    fn card_bottom(
        &mut self,
        ui: &mut Ui,
        key: &'static str,
        (left, right): (f32, f32),
        bottom: f32,
        margin: Margin,
        add: impl FnOnce(&mut Self, &mut Ui),
    ) -> Rect {
        let h = *self.heights.get(key).unwrap_or(&120.0);
        let top = bottom - h;
        let max = Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, top + 4000.0));
        let frame = self.card(margin);
        let rect = ui
            .scope_builder(UiBuilder::new().max_rect(max).layout(Layout::top_down(Align::Min)), |ui| {
                frame
                    .show(ui, |ui| {
                        ui.set_width(right - left - margin.sum().x - 2.0);
                        add(self, ui)
                    })
                    .response
                    .rect
            })
            .inner;
        if (rect.height() - h).abs() > 0.5 {
            self.heights.insert(key, rect.height());
            ui.ctx().request_discard("card height changed");
            ui.ctx().request_repaint();
        }
        rect
    }

    fn primary_button(&self, ui: &mut Ui, enabled: bool, text: &str) -> egui::Response {
        let button = egui::Button::new(rich(text, 13.0, Color32::WHITE).family(semibold())).fill(self.pal.accent_fill);
        ui.add_enabled(enabled, button)
    }

    fn button(&self, ui: &mut Ui, enabled: bool, text: &str) -> egui::Response {
        let button = egui::Button::new(rich(text, 13.0, self.pal.text)).fill(self.pal.raised).stroke(Stroke::NONE);
        ui.add_enabled(enabled, button)
    }

    fn header(&mut self, ui: &mut Ui, ctx: &egui::Context, rect: Rect) {
        let p = self.pal;
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.horizontal_centered(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut job = egui::text::LayoutJob::default();
                    let font = FontId::new(23.0, bold());
                    job.append("ROCH", 0.0, egui::TextFormat::simple(font.clone(), p.accent));
                    job.append(" MICROCODE", 0.0, egui::TextFormat::simple(font, p.text));
                    ui.label(job);
                    ui.label(rich("Intel microcode transfer", 12.0, p.muted));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if self.light { "Dark theme" } else { "Light theme" };
                    let theme_button =
                        egui::Button::new(rich(label, 13.0, p.text)).fill(p.raised).min_size(vec2(108.0, 30.0));
                    if ui.add_enabled(true, theme_button).on_hover_text("Switch light or dark theme").clicked() {
                        let saved = self.set_theme(ctx, !self.light, true);
                        if !saved {
                            self.status = "Theme changed for this session; the preference could not be saved.".into();
                        }
                    }
                    ui.add_space(14.0);
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        ui.add_space(4.0);
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.label(rich("ASUS  /  MSI  /  ASRock  /  Gigabyte", 12.0, p.muted));
                        ui.label(rich(VERSION_TEXT, 12.0, p.muted));
                    });
                });
            });
        });
    }

    fn tabs(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.pal;
        let mut x = rect.left();
        for (i, name) in TABS.iter().enumerate() {
            let galley = ui.painter().layout_no_wrap(name.to_string(), FontId::new(13.0, semibold()), p.text);
            let size = vec2(galley.size().x + 36.0, galley.size().y + 18.0);
            let tab_rect = Rect::from_min_size(Pos2::new(x, rect.top()), size);
            let response = ui.interact(tab_rect, ui.id().with(("tab", i)), Sense::click());
            let selected = self.tab == i;
            let color = if selected {
                p.accent
            } else if response.hovered() {
                p.text
            } else {
                p.muted
            };
            let painter = ui.painter();
            painter.rect_filled(tab_rect, CornerRadius::same(3), if selected { p.surface } else { p.canvas });
            if selected {
                painter.line_segment(
                    [tab_rect.left_bottom() + vec2(1.0, -1.0), tab_rect.right_bottom() + vec2(-1.0, -1.0)],
                    Stroke::new(2.0_f32, p.accent),
                );
            }
            painter.text(tab_rect.center(), Align2::CENTER_CENTER, *name, FontId::new(13.0, semibold()), color);
            if response.clicked() {
                self.tab = i;
            }
            x += size.x + 8.0;
        }
    }

    fn footer(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.pal;
        let painter = ui.painter_at(rect);
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 3.0));
        if self.busy {
            painter.rect_filled(bar, CornerRadius::ZERO, p.outline);
            let t = ui.input(|i| i.time) as f32;
            let w = bar.width() * 0.25;
            let x = bar.left() + ((t * 0.6).fract() * (bar.width() + w)) - w;
            let seg = Rect::from_min_max(
                Pos2::new(x.max(bar.left()), bar.top()),
                Pos2::new((x + w).min(bar.right()), bar.bottom()),
            );
            painter.rect_filled(seg, CornerRadius::ZERO, p.accent);
            ui.ctx().request_repaint();
        }
        let status_rect = Rect::from_min_max(
            Pos2::new(rect.left(), bar.bottom() + 5.0),
            Pos2::new(rect.right(), bar.bottom() + 24.0),
        );
        let mut text_right = status_rect.right();
        if self.busy && self.cancellable {
            let cancel_rect = Rect::from_min_max(
                Pos2::new(status_rect.right() - 76.0, status_rect.top() - 4.0),
                status_rect.right_bottom(),
            );
            let response = ui.put(cancel_rect, egui::Button::new(rich("Cancel", 13.0, p.text)).fill(p.raised));
            if response.clicked() {
                if let Some(c) = &self.cancel {
                    c.cancel();
                }
            }
            text_right = cancel_rect.left() - 12.0;
        }
        let status_area = Rect::from_min_max(status_rect.min, Pos2::new(text_right, status_rect.bottom()));
        ui.scope_builder(UiBuilder::new().max_rect(status_area), |ui| {
            let r = ui.add(egui::Label::new(rich(&self.status, 12.0, p.muted)).wrap_mode(TextWrapMode::Truncate));
            r.on_hover_text(&self.status);
        });
        let links_rect = Rect::from_min_max(Pos2::new(rect.left(), status_rect.bottom() + 2.0), rect.right_bottom());
        let mut open: Option<&str> = None;
        ui.scope_builder(UiBuilder::new().max_rect(links_rect), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (i, (label, url, tip)) in LINKS.iter().enumerate() {
                    if i > 0 {
                        ui.label(rich("  |  ", 11.5, p.muted).family(semibold()));
                    }
                    let r =
                        ui.add(egui::Label::new(rich(*label, 11.5, p.accent).family(semibold())).sense(Sense::click()));
                    let r = if r.hovered() {
                        ui.painter().line_segment(
                            [r.rect.left_bottom() + vec2(0.0, -1.0), r.rect.right_bottom() + vec2(0.0, -1.0)],
                            Stroke::new(1.0_f32, p.accent),
                        );
                        r.on_hover_cursor(egui::CursorIcon::PointingHand)
                    } else {
                        r
                    };
                    if r.on_hover_text(*tip).clicked() {
                        open = Some(url);
                    }
                }
            });
        });
        if let Some(url) = open {
            self.open(url);
        }
    }

    // ---------------------------------------------------------------- pages

    fn microcode_combo(&mut self, ui: &mut Ui, id: &str, side: Side, width: f32) -> egui::Response {
        let (codes, sel) = match side {
            Side::Old => (&self.old_codes, self.old_sel),
            Side::New => (&self.new_codes, self.new_sel),
        };
        let text = sel.and_then(|i| codes.get(i)).map(|m| m.display()).unwrap_or_default();
        let mut chosen: Option<usize> = None;
        let font = FontId::monospace(12.0);
        let response = ui
            .add_enabled_ui(!self.busy, |ui| {
                egui::ComboBox::from_id_salt(id)
                    .width(width)
                    .height(250.0)
                    .wrap_mode(TextWrapMode::Truncate)
                    .selected_text(RichText::new(text).font(font.clone()).color(self.pal.text))
                    .show_ui(ui, |ui| {
                        for (i, m) in codes.iter().enumerate() {
                            if ui
                                .selectable_label(sel == Some(i), RichText::new(m.display()).font(font.clone()))
                                .clicked()
                            {
                                chosen = Some(i);
                            }
                        }
                    })
                    .response
            })
            .inner;
        if let Some(i) = chosen {
            if Some(i) != sel {
                match side {
                    Side::Old => {
                        self.old_sel = Some(i);
                        self.invalidate();
                        self.update_targets();
                    }
                    Side::New => {
                        self.new_sel = Some(i);
                        self.invalidate();
                        self.update_plan();
                    }
                }
            }
        }
        response
    }

    fn file_card(&mut self, ui: &mut Ui, ctx: &egui::Context, rect: Rect, side: Side) {
        let p = self.pal;
        let highlight = (self.drop_target == Some(side)).then_some(p.accent);
        let (tag, heading, hint, label, combo_id, combo_probe) = match side {
            Side::Old => (
                "01  DONOR BIOS",
                "Drop donor BIOS",
                "Copy a CPU microcode from a BIOS file or vendor ZIP.",
                "Microcode to copy",
                "old-codes",
                "OldCodes",
            ),
            Side::New => (
                "02  TARGET BIOS",
                "Drop target BIOS",
                "Use your target board's BIOS. The original is preserved.",
                "Microcode to replace",
                "new-codes",
                "NewCodes",
            ),
        };
        self.card_at(ui, rect, Margin::same(16), highlight, |app, ui| {
            let inner = ui.max_rect();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(rich(tag, 11.0, p.accent).family(semibold()));
                    ui.label(rich(heading, 17.0, p.text).family(semibold()));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if app.button(ui, !app.busy, "Browse…").clicked() {
                        let title = match side {
                            Side::Old => "Choose OLD donor BIOS or ZIP",
                            Side::New => "Choose NEW BIOS to modify or ZIP",
                        };
                        if let Some(path) = app.pick_image(title) {
                            app.load(ctx, path, side);
                        }
                    }
                });
            });
            ui.add_space(4.0);
            let hint_rect = ui.add(egui::Label::new(rich(hint, 12.0, p.muted)).wrap_mode(TextWrapMode::Wrap)).rect;
            let top = hint_rect.bottom();
            // Bottom: label + combo.
            let combo_h = 30.0;
            let combo_rect =
                Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - combo_h), inner.right_bottom());
            let label_rect = Rect::from_min_max(
                Pos2::new(inner.left(), combo_rect.top() - 22.0),
                Pos2::new(inner.right(), combo_rect.top() - 5.0),
            );
            ui.scope_builder(UiBuilder::new().max_rect(label_rect), |ui| {
                ui.label(rich(label, 12.0, p.muted));
            });
            let response = ui
                .scope_builder(UiBuilder::new().max_rect(combo_rect), |ui| {
                    app.microcode_combo(ui, combo_id, side, inner.width())
                })
                .inner;
            app.probe(combo_probe, response.rect);
            // Middle: file name and details, vertically centred.
            let (name, detail) = match side {
                Side::Old => (app.old_name.clone(), app.old_detail.clone()),
                Side::New => (app.new_name.clone(), app.new_detail.clone()),
            };
            let middle = Rect::from_min_max(
                Pos2::new(inner.left(), top + 10.0),
                Pos2::new(inner.right(), label_rect.top() - 10.0),
            );
            let block = Rect::from_center_size(middle.center(), vec2(middle.width(), 40.0));
            ui.scope_builder(UiBuilder::new().max_rect(block), |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                let r = ui.add(
                    egui::Label::new(rich(&name, 13.0, p.text).family(semibold())).wrap_mode(TextWrapMode::Truncate),
                );
                r.on_hover_text(&name);
                ui.add(egui::Label::new(rich(&detail, 12.0, p.muted)).wrap_mode(TextWrapMode::Truncate));
            });
        });
    }

    fn transfer_page(&mut self, ui: &mut Ui, ctx: &egui::Context, page: Rect) {
        let p = self.pal;
        let span = (page.left(), page.right());
        // Plan card at the bottom.
        let plan = self.card_bottom(ui, "plan", span, page.bottom(), Margin::symmetric(14, 11), |app, ui| {
                let r = ui.add(egui::Label::new(rich(&app.plan_text, 15.0, p.text).family(semibold())).wrap_mode(TextWrapMode::Wrap));
                app.probe("PlanText", r.rect);
                if !app.coverage_text.is_empty() {
                    let r = ui.add(egui::Label::new(rich(&app.coverage_text, 12.0, p.warning)).wrap_mode(TextWrapMode::Wrap));
                    app.probe("CoverageText", r.rect);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let enabled = app.build_enabled();
                    let r = app.primary_button(ui, enabled, "Build & verify…");
                    app.probe("BuildButton", r.rect);
                    if r.clicked() {
                        app.build_clicked(ctx);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(rich("Native checks + UEFIExtract + MCExtractor", 12.0, p.muted));
                    });
                });
                ui.add_space(3.0);
                ui.add(
                    egui::Label::new(rich(
                        "Older microcode can omit fixes. File checks do not prove bootability. Supported: Intel FIT with uncompressed slots; equal-size or smaller donor.",
                        11.0,
                        p.muted,
                    ))
                    .wrap_mode(TextWrapMode::Wrap),
                );
            });
        // Profile card above it.
        let profile = self.card_bottom(ui, "profile", span, plan.top() - 10.0, Margin::symmetric(14, 11), |app, ui| {
            app.profile_row(ui)
        });
        let bottom_top = (profile.top() - 10.0).max(page.top() + 60.0);
        let top = Rect::from_min_max(page.min, Pos2::new(page.right(), bottom_top));
        let half = (top.width() - 12.0) / 2.0;
        let left = Rect::from_min_size(top.min, vec2(half, top.height()));
        let right = Rect::from_min_size(Pos2::new(left.right() + 12.0, top.top()), vec2(half, top.height()));
        self.old_card = left;
        self.new_card = right;
        self.file_card(ui, ctx, left, Side::Old);
        self.file_card(ui, ctx, right, Side::New);
    }

    fn profile_row(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let width = ui.available_width();
        let fixed = 110.0 + 190.0 + 165.0 + 36.0;
        let board_w = (width - fixed).max(120.0);
        let enabled = !self.busy;
        let mut vendor_changed: Option<String> = None;
        let mut method_changed: Option<String> = None;
        let mut profile_changed = false;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.allocate_ui_with_layout(vec2(110.0, 52.0), Layout::top_down(Align::Min), |ui| {
                ui.label(rich("Manufacturer", 12.0, p.muted));
                ui.add_enabled_ui(enabled, |ui| {
                    egui::ComboBox::from_id_salt("vendor")
                        .width(110.0)
                        .selected_text(rich(&self.vendor, 13.0, p.text))
                        .show_ui(ui, |ui| {
                            for v in VENDORS {
                                if ui.selectable_label(self.vendor == v, v).clicked() && self.vendor != v {
                                    vendor_changed = Some(v.to_string());
                                }
                            }
                        })
                });
            });
            let r = ui
                .allocate_ui_with_layout(vec2(190.0, 52.0), Layout::top_down(Align::Min), |ui| {
                    ui.label(rich("Flash method", 12.0, p.muted));
                    ui.add_enabled_ui(enabled, |ui| {
                        egui::ComboBox::from_id_salt("method")
                            .width(190.0)
                            .selected_text(rich(&self.method, 12.0, p.text))
                            .show_ui(ui, |ui| {
                                for m in self.methods.clone() {
                                    if ui.selectable_label(self.method == m, rich(m, 12.0, p.text)).clicked()
                                        && self.method != m
                                    {
                                        method_changed = Some(m.to_string());
                                    }
                                }
                            })
                            .response
                    })
                    .inner
                })
                .inner;
            self.probe("MethodBox", r.rect);
            let r = ui
                .allocate_ui_with_layout(vec2(board_w, 52.0), Layout::top_down(Align::Min), |ui| {
                    ui.label(rich("Target motherboard / revision", 12.0, p.muted));
                    let edit = egui::TextEdit::singleline(&mut self.board)
                        .desired_width(board_w)
                        .min_size(vec2(board_w, 30.0))
                        .margin(Margin::symmetric(9, 6))
                        .font(FontId::proportional(13.0));
                    let r = ui.add_enabled(enabled, edit);
                    if r.changed() {
                        profile_changed = true;
                    }
                    r
                })
                .inner;
            self.probe("BoardBox", r.rect);
            let r = ui
                .allocate_ui_with_layout(vec2(165.0, 52.0), Layout::top_down(Align::Min), |ui| {
                    ui.label(rich("USB root filename", 12.0, p.muted));
                    let edit = egui::TextEdit::singleline(&mut self.flash_name)
                        .desired_width(165.0)
                        .min_size(vec2(165.0, 30.0))
                        .margin(Margin::symmetric(9, 6))
                        .font(FontId::monospace(13.0))
                        .interactive(!self.flash_read_only);
                    let r = ui.add_enabled(enabled, edit);
                    if r.changed() {
                        profile_changed = true;
                    }
                    r
                })
                .inner;
            self.probe("FlashNameBox", r.rect);
        });
        let r = ui.add(egui::Label::new(rich(&self.profile_hint, 11.0, p.muted)).wrap_mode(TextWrapMode::Wrap));
        self.probe("ProfileHint", r.rect);
        if let Some(v) = vendor_changed {
            self.vendor = v.clone();
            self.set_methods(&v);
            self.invalidate();
        }
        if let Some(m) = method_changed {
            self.set_method(&m);
        }
        if profile_changed {
            self.invalidate();
        }
    }

    fn verification_page(&mut self, ui: &mut Ui, ctx: &egui::Context, page: Rect) {
        let p = self.pal;
        // Header.
        let header = ui
            .scope_builder(UiBuilder::new().max_rect(page.shrink2(vec2(2.0, 0.0))), |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let r = ui.add(
                    egui::Label::new(rich(&self.verification_title, 21.0, p.text).family(semibold()))
                        .wrap_mode(TextWrapMode::Truncate),
                );
                self.probe("VerificationTitle", r.rect);
                let r = ui.add(
                    egui::Label::new(rich(&self.verification_detail, 12.0, p.muted)).wrap_mode(TextWrapMode::Wrap),
                );
                self.probe("VerificationDetail", r.rect);
            })
            .response
            .rect;
        // Bottom card.
        let span = (page.left(), page.right());
        let card = self.card_bottom(ui, "verification", span, page.bottom(), Margin::symmetric(14, 10), |app, ui| {
            let lines: Vec<String> = if app.output_hash.is_empty() {
                vec![String::new()]
            } else {
                app.output_hash.lines().map(str::to_string).collect()
            };
            let hash_rect = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for line in &lines {
                        let r = ui.add(
                            egui::Label::new(RichText::new(line).font(FontId::monospace(11.0)).color(p.text))
                                .wrap_mode(TextWrapMode::Truncate)
                                .selectable(true),
                        );
                        if !app.output_hash.is_empty() {
                            r.on_hover_text(&app.output_hash);
                        }
                    }
                })
                .response
                .rect;
            app.probe("OutputHash", hash_rect);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let has = !app.busy && app.package.is_some();
                let r = app.button(ui, has, "Open package & reports");
                app.probe("OpenPackageButton", r.rect);
                if r.clicked() {
                    if let Some(dir) = app.package.as_ref().map(|p| p.directory.display().to_string()) {
                        app.open(&dir);
                    }
                }
                let r = app.primary_button(ui, has, "Prepare USB →");
                app.probe("GoUsbButton", r.rect);
                if r.clicked() {
                    app.tab = 2;
                    app.refresh_drives(ctx);
                }
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(rich(
                        "File integrity verified; vendor acceptance, hardware booting and stability remain untested.",
                        11.0,
                        p.warning,
                    ))
                    .wrap_mode(TextWrapMode::Wrap),
                );
            });
        });
        let bottom_top = card.top() - 6.0;
        // Checks: two columns by four rows.
        let area =
            Rect::from_min_max(Pos2::new(page.left(), header.bottom() + 10.0), Pos2::new(page.right(), bottom_top));
        self.probe("Checks", area);
        let cell_w = area.width() / 2.0;
        let cell_h = area.height() / 4.0;
        let checks = self.checks.clone();
        for (i, c) in checks.iter().enumerate().take(8) {
            let col = (i % 2) as f32;
            let row = (i / 2) as f32;
            let cell = Rect::from_min_size(
                Pos2::new(area.left() + col * cell_w, area.top() + row * cell_h),
                vec2(cell_w, cell_h),
            );
            let rect = Rect::from_min_max(cell.min + vec2(4.0, 0.0), cell.max - vec2(4.0, 8.0));
            self.card_at(ui, rect, Margin::symmetric(12, 8), None, |_, ui| {
                let inner = ui.max_rect();
                let block_h: f32 = 40.0;
                let block = Rect::from_center_size(inner.center(), vec2(inner.width(), block_h.min(inner.height())));
                ui.scope_builder(UiBuilder::new().max_rect(block), |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add(
                        egui::Label::new(rich(&c.name, 13.0, p.success).family(semibold()))
                            .wrap_mode(TextWrapMode::Truncate),
                    );
                    ui.add(egui::Label::new(rich(&c.detail, 12.0, p.muted)).wrap_mode(TextWrapMode::Truncate));
                });
            });
            let hover = ui.interact(rect, ui.id().with(("check", i)), Sense::hover());
            hover.on_hover_text(&c.full_detail);
        }
    }

    fn usb_page(&mut self, ui: &mut Ui, ctx: &egui::Context, page: Rect) {
        let p = self.pal;
        let left_w = (page.width() - 12.0) * 1.15 / 2.15;
        let left = Rect::from_min_size(page.min, vec2(left_w, page.height()));
        let right = Rect::from_min_max(Pos2::new(left.right() + 12.0, page.top()), page.max);
        self.card_at(ui, left, Margin::same(16), None, |app, ui| {
            let inner = ui.max_rect();
            ui.label(rich("Prepare a BIOS USB drive", 21.0, p.text).family(semibold()));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if app.button(ui, !app.busy, "Refresh USB drives").clicked() {
                    app.refresh_drives(ctx);
                }
                if app.button(ui, !app.busy, "Open package…").clicked() {
                    app.load_package_clicked(ctx);
                }
            });
            ui.add_space(10.0);
            ui.label(rich("USB destination", 12.0, p.muted));
            let text = app.drive_sel.and_then(|i| app.drives.get(i)).map(|d| d.display()).unwrap_or_default();
            let mut chosen: Option<usize> = None;
            let response = ui
                .add_enabled_ui(!app.busy, |ui| {
                    egui::ComboBox::from_id_salt("drives")
                        .width(inner.width())
                        .wrap_mode(TextWrapMode::Truncate)
                        .selected_text(rich(text, 13.0, p.text))
                        .show_ui(ui, |ui| {
                            for (i, d) in app.drives.iter().enumerate() {
                                if ui.selectable_label(app.drive_sel == Some(i), d.display()).clicked() {
                                    chosen = Some(i);
                                }
                            }
                        })
                        .response
                })
                .inner;
            app.probe("Drives", response.rect);
            if let Some(i) = chosen {
                app.drive_sel = Some(i);
                app.drive_status = app.drives[i]
                    .rejection()
                    .unwrap_or("Compatible layout. The destination will be confirmed before copying.")
                    .into();
                app.usb_result.clear();
            }
            ui.add_space(4.0);
            let r = ui.add(egui::Label::new(rich(&app.drive_status, 12.0, p.warning)).wrap_mode(TextWrapMode::Wrap));
            app.probe("DriveStatus", r.rect);
            ui.add_space(12.0);
            let radios = ui.add_enabled_ui(!app.busy, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                let r = ui.radio_value(&mut app.recovery_choice, false, rich("Modified BIOS with the selected microcode", 13.0, p.text));
                app.probe("ModifiedChoice", r.rect);
                let r = ui.radio_value(&mut app.recovery_choice, true, rich("Untouched new BIOS for recovery", 13.0, p.text));
                app.probe("RecoveryChoice", r.rect);
            });

            let after_radios = radios.response.rect.bottom() + 8.0;
            // Bottom-aligned explanation and button.
            let bottom = Rect::from_min_max(Pos2::new(inner.left(), after_radios), inner.max);
            ui.scope_builder(UiBuilder::new().max_rect(bottom).layout(Layout::bottom_up(Align::Min)), |ui| {
                let enabled = app.prepare_enabled();
                let r = app.primary_button(ui, enabled, "Verify again & prepare USB…");
                app.probe("PrepareButton", r.rect);
                if r.clicked() {
                    app.prepare_clicked(ctx);
                }
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(rich(
                        "Rechecks the files and drive identity, backs up an existing BIOS file, then copies and verifies the result.",
                        12.0,
                        p.muted,
                    ))
                    .wrap_mode(TextWrapMode::Wrap),
                );
                ui.add_space(6.0);
                // Remaining space (top-down) for the result text.
                let rest = Rect::from_min_max(bottom.min, Pos2::new(bottom.right(), ui.min_rect().top() - 4.0));
                if rest.height() > 0.0 {
                    ui.scope_builder(UiBuilder::new().max_rect(rest).layout(Layout::top_down(Align::Min)), |ui| {
                        egui::ScrollArea::vertical().id_salt("usb-result").max_height(rest.height()).show(ui, |ui| {
                            ui.add(egui::Label::new(rich(&app.usb_result, 12.0, p.success)).wrap_mode(TextWrapMode::Wrap).selectable(true));
                        });
                    });
                }
            });
        });
        // Right column: package (fills) above the board guide (content height).
        let guide = self.card_bottom(
            ui,
            "guide",
            (right.left(), right.right()),
            right.bottom(),
            Margin::same(16),
            |app, ui| {
                ui.label(rich("At the motherboard", 17.0, p.text).family(semibold()));
                ui.add_space(3.0);
                ui.add(egui::Label::new(rich(&app.board_guide, 12.0, p.text)).wrap_mode(TextWrapMode::Wrap));
                ui.add_space(5.0);
                if app.button(ui, true, "Board vendor guide ↗").clicked() {
                    let url = app
                        .package
                        .as_ref()
                        .map(|p| p.manifest.profile.guide_url())
                        .unwrap_or_else(|| app.profile().guide_url());
                    app.open(url);
                }
                ui.add_space(5.0);
                ui.add(
                    egui::Label::new(rich(
                        "The app does not format the drive or flash the motherboard.",
                        11.0,
                        p.muted,
                    ))
                    .wrap_mode(TextWrapMode::Wrap),
                );
            },
        );
        let guide_top = guide.top() - 12.0;
        let package_rect = Rect::from_min_max(right.min, Pos2::new(right.right(), guide_top));
        self.card_at(ui, package_rect, Margin::same(16), None, |app, ui| {
            ui.label(rich("Selected package", 17.0, p.text).family(semibold()));
            ui.add_space(6.0);
            let height = ui.available_height();
            let r = egui::ScrollArea::vertical()
                .id_salt("package-text")
                .max_height(height)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(rich(&app.package_text, 12.0, p.muted)).wrap_mode(TextWrapMode::Wrap))
                })
                .inner;
            app.probe("PackageText", r.rect);
            if !app.package_tooltip.is_empty() {
                r.on_hover_text(&app.package_tooltip);
            }
        });
    }

    fn inspect_page(&mut self, ui: &mut Ui, ctx: &egui::Context, page: Rect) {
        let p = self.pal;
        let mut system_bottom = page.top();
        ui.scope_builder(UiBuilder::new().max_rect(page), |ui| {
            let r = self.card_auto(ui, Margin::symmetric(14, 10), |app, ui| {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(vec2(145.0, 20.0), Layout::left_to_right(Align::Center), |ui| {
                        ui.label(rich("This computer", 13.0, p.text).family(semibold()));
                    });
                    let r =
                        ui.add(egui::Label::new(rich(&app.system_text, 12.0, p.muted)).wrap_mode(TextWrapMode::Wrap));
                    app.probe("SystemText", r.rect);
                });
            });
            system_bottom = r.response.rect.bottom() + 10.0;
        });
        let credits_h = 22.0;
        let credits = Rect::from_min_max(Pos2::new(page.left(), page.bottom() - credits_h + 8.0), page.max);
        ui.scope_builder(UiBuilder::new().max_rect(credits), |ui| {
            ui.add(
                egui::Label::new(rich(
                    concat!(
                        "Roch Microcode ",
                        env!("CARGO_PKG_VERSION"),
                        " • MateoPcTech. UEFIExtract by LongSoft; MCExtractor by Plato Mavropoulos. Licenses included."
                    ),
                    11.0,
                    p.muted,
                ))
                .wrap_mode(TextWrapMode::Truncate),
            );
        });
        let main = Rect::from_min_max(
            Pos2::new(page.left(), system_bottom),
            Pos2::new(page.right(), page.bottom() - credits_h),
        );
        self.card_at(ui, main, Margin::same(16), None, |app, ui| {
            let inner = ui.max_rect();
            ui.horizontal(|ui| {
                ui.label(rich("Firmware microcodes", 17.0, p.text).family(semibold()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = app.button(ui, !app.busy, "Open firmware…");
                    app.probe("InspectButton", r.rect);
                    if r.clicked() {
                        app.inspect_clicked(ctx);
                    }
                });
            });
            ui.add_space(3.0);
            let r = ui.add(egui::Label::new(rich(&app.image_identity, 13.0, p.text).family(semibold())).wrap_mode(TextWrapMode::Truncate));
            app.probe("ImageIdentity", r.rect);
            let hash_rect = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for line in app.image_hash.lines() {
                        ui.add(
                            egui::Label::new(RichText::new(line).font(FontId::monospace(11.0)).color(p.text))
                                .wrap_mode(TextWrapMode::Truncate)
                                .selectable(true),
                        );
                    }
                })
                .response
                .rect;
            app.probe("ImageHash", hash_rect);
            if !app.image_hash.is_empty() {
                ui.interact(hash_rect, ui.id().with("image-hash-tip"), Sense::hover()).on_hover_text(&app.image_hash);
            }
            ui.add_space(8.0);
            let foot_h = 18.0;
            let table = Rect::from_min_max(Pos2::new(inner.left(), hash_rect.bottom() + 8.0), Pos2::new(inner.right(), inner.bottom() - foot_h - 6.0));
            app.probe("MicrocodeGrid", table);
            app.microcode_table(ui, table);
            let foot = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - foot_h), inner.max);
            ui.scope_builder(UiBuilder::new().max_rect(foot), |ui| {
                ui.add(
                    egui::Label::new(rich(
                        "Uncompressed Intel microcodes. Checksums do not establish CPU compatibility or Intel signature authenticity.",
                        11.0,
                        p.muted,
                    ))
                    .wrap_mode(TextWrapMode::Truncate),
                );
            });
        });
    }

    fn microcode_table(&mut self, ui: &mut Ui, rect: Rect) {
        let p = self.pal;
        if rect.height() < 30.0 {
            return;
        }
        let weights = [1.0f32, 1.0, 1.0, 1.2, 1.0, 1.1, 1.7];
        let headers = ["CPU", "Platform", "Revision", "Date", "Size", "Offset", "Checksum"];
        let total: f32 = weights.iter().sum();
        let mut xs = vec![rect.left()];
        for w in weights {
            xs.push(xs.last().unwrap() + rect.width() * w / total);
        }
        let row_h = 27.0;
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 30.0));
        let painter = ui.painter_at(rect);
        painter.rect_filled(header, CornerRadius::ZERO, p.input);
        for (i, h) in headers.iter().enumerate() {
            painter.text(
                Pos2::new(xs[i] + 9.0, header.center().y),
                Align2::LEFT_CENTER,
                *h,
                FontId::new(13.0, semibold()),
                p.muted,
            );
        }
        let body = Rect::from_min_max(Pos2::new(rect.left(), header.bottom()), rect.max);
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("microcodes")
                .max_height(body.height())
                .auto_shrink([false, false])
                .show_rows(ui, row_h, self.grid.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for i in range {
                        let m = &self.grid[i];
                        let (row, _) = ui.allocate_exact_size(vec2(body.width(), row_h), Sense::hover());
                        let painter = ui.painter_at(row);
                        painter.rect_filled(row, CornerRadius::ZERO, if i % 2 == 0 { p.surface } else { p.raised });
                        let cells = [
                            m.cpu_text(),
                            m.platform_text(),
                            m.revision_text(),
                            m.date.clone(),
                            m.size_text(),
                            m.offset_text(),
                            m.status().to_string(),
                        ];
                        for (c, text) in cells.iter().enumerate() {
                            let cell = Rect::from_min_max(
                                Pos2::new(xs[c] + 9.0, row.top()),
                                Pos2::new(xs[c + 1] - 4.0, row.bottom()),
                            );
                            ui.painter_at(cell).text(
                                Pos2::new(cell.left(), cell.center().y),
                                Align2::LEFT_CENTER,
                                text,
                                FontId::new(13.0, FontFamily::Proportional),
                                p.text,
                            );
                        }
                    }
                });
        });
    }
}

/// Parent window for native file dialogs.
pub struct ParentWindow(Hwnd);

impl raw_window_handle::HasWindowHandle for ParentWindow {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let hwnd = std::num::NonZeroIsize::new(self.0).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let raw = raw_window_handle::RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(hwnd));
        // SAFETY: the handle belongs to the live main window for the dialog's duration.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

impl raw_window_handle::HasDisplayHandle for ParentWindow {
    fn display_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::windows())
    }
}

impl eframe::App for RochApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.hwnd == 0 {
            self.hwnd = platform::hwnd_of(frame);
        }
        if self.title_bar_applied != Some(self.light) && self.hwnd != 0 {
            platform::apply_title_bar(self.hwnd, self.light, self.pal.canvas, self.pal.text, self.pal.outline);
            self.title_bar_applied = Some(self.light);
        }
        self.poll(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && self.busy {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Wait for the operation to finish or cancel verification before closing.".into();
        }
        self.handle_drops(ctx);
        self.probes.clear();
        let p = self.pal;
        egui::CentralPanel::default().frame(Frame::new().fill(p.canvas).inner_margin(Margin::symmetric(20, 14))).show(
            ctx,
            |ui| {
                let full = ui.max_rect();
                let header = Rect::from_min_size(full.min, vec2(full.width(), 46.0));
                let tabs =
                    Rect::from_min_size(Pos2::new(full.left(), header.bottom() + 10.0), vec2(full.width(), 36.0));
                let footer = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - 46.0), full.max);
                let page = Rect::from_min_max(
                    Pos2::new(full.left(), tabs.bottom() + 12.0),
                    Pos2::new(full.right(), footer.top() - 10.0),
                );
                self.page_rect = page;
                self.header(ui, ctx, header);
                self.tabs(ui, tabs);
                match self.tab {
                    0 => self.transfer_page(ui, ctx, page),
                    1 => self.verification_page(ui, ctx, page),
                    2 => self.usb_page(ui, ctx, page),
                    _ => self.inspect_page(ui, ctx, page),
                }
                self.footer(ui, footer);
            },
        );
        if let Some(text) = self.pending_warning.take() {
            platform::warning_box(self.hwnd, TITLE, &text);
        }
        crate::smoke::step(self, ctx);
    }
}

/// Load a BIOS synchronously (smoke test).
pub fn load_now(path: &Path, side: Side) -> Result<(FirmwareInput, Option<String>)> {
    let input = FirmwareInput::load(path)?;
    let asus = if side == Side::New { input.suggested_asus_name() } else { None };
    Ok((input, asus))
}
