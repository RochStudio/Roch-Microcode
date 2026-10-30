//! Light and dark palettes (from the original Themes/*.xaml) and egui styling.

use std::path::PathBuf;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub canvas: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub input: Color32,
    pub outline: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_fill: Color32,
    pub selection: Color32,
    pub warning: Color32,
    pub success: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const DARK: Palette = Palette {
    canvas: hex(0x080808),
    surface: hex(0x111111),
    raised: hex(0x1C1C1C),
    input: hex(0x090909),
    outline: hex(0x303030),
    text: hex(0xEEEEEE),
    muted: hex(0xACACAC),
    accent: hex(0xFF626A),
    accent_fill: hex(0xA32934),
    selection: hex(0x32181C),
    warning: hex(0xF0BE81),
    success: hex(0x82DDB3),
};

pub const LIGHT: Palette = Palette {
    canvas: hex(0xF1F3F6),
    surface: hex(0xFFFFFF),
    raised: hex(0xE8ECF1),
    input: hex(0xF7F8FA),
    outline: hex(0xC5CDD7),
    text: hex(0x202733),
    muted: hex(0x535F70),
    accent: hex(0xAF2331),
    accent_fill: hex(0xB52E38),
    selection: hex(0xF5DADD),
    warning: hex(0x825013),
    success: hex(0x146C47),
};

pub fn palette(light: bool) -> Palette {
    if light {
        LIGHT
    } else {
        DARK
    }
}

/// Font families with weights (egui has no font weights of its own).
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Segoe UI / Consolas from the Windows font folder, with egui's bundled
/// fonts as fallback (for arrows and any missing glyphs).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var_os("WINDIR")
        .map(|w| PathBuf::from(w).join("Fonts"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows\Fonts"));
    let mut load = |key: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                fonts.font_data.insert(key.to_string(), std::sync::Arc::new(FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoe", "segoeui.ttf");
    let semi = load("segoe-semibold", "seguisb.ttf");
    let heavy = load("segoe-bold", "segoeuib.ttf");
    let mono = load("consolas", "consola.ttf");
    let symbols = load("segoe-symbol", "seguisym.ttf");
    let fallback: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with_fallback = |first: Option<&str>| {
        let mut v: Vec<String> = first.map(|f| vec![f.to_string()]).unwrap_or_default();
        if symbols {
            v.push("segoe-symbol".into());
        }
        v.extend(fallback.iter().cloned());
        v
    };
    let base = if regular { Some("segoe") } else { None };
    fonts.families.insert(FontFamily::Proportional, with_fallback(base));
    fonts.families.insert(semibold(), with_fallback(if semi { Some("segoe-semibold") } else { base }));
    fonts.families.insert(bold(), with_fallback(if heavy { Some("segoe-bold") } else { base }));
    if mono {
        let mut v = vec!["consolas".to_string()];
        v.extend(fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default());
        if symbols {
            v.push("segoe-symbol".into());
        }
        fonts.families.insert(FontFamily::Monospace, v);
    }
    ctx.set_fonts(fonts);
}

/// Apply the palette to egui's visuals.
pub fn apply(ctx: &egui::Context, p: &Palette, light: bool) {
    let mut v = if light { Visuals::light() } else { Visuals::dark() };
    v.override_text_color = Some(p.text);
    v.panel_fill = p.canvas;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0_f32, p.outline);
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.raised;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.accent;
    v.selection.bg_fill = p.accent_fill;
    v.selection.stroke = Stroke::new(1.0_f32, p.text);
    v.window_corner_radius = CornerRadius::same(5);
    v.menu_corner_radius = CornerRadius::same(4);
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.window_shadow = egui::epaint::Shadow::NONE;
    let radius = CornerRadius::same(5);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.outline);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.noninteractive.corner_radius = radius;
    for (state, fill) in
        [(&mut w.inactive, p.raised), (&mut w.hovered, p.raised), (&mut w.active, p.raised), (&mut w.open, p.input)]
    {
        state.bg_fill = p.input;
        state.weak_bg_fill = fill;
        state.fg_stroke = Stroke::new(1.0_f32, p.text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.outline);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.open.bg_stroke = Stroke::new(1.0_f32, p.outline);
    w.hovered.weak_bg_fill = p.selection;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 5.0);
        s.spacing.button_padding = egui::vec2(16.0, 7.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.combo_height = 250.0;
        s.interaction.tooltip_delay = 0.35;
        s.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    });
}
