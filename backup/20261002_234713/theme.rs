// Theme system for XMST UI (stage 6).
// English comments per project convention; user-facing strings live in main.rs (Chinese).
//
// Design:
// - ThemeColors is a small palette struct that we lerp frame-independently (exponential
//   interpolation, same style as the settings collapse animation) so day/night/preset/custom
//   switches cross-fade smoothly. It is expanded into egui::Visuals once per frame.
// - Corner radius (rounding) is applied as a scale on top of egui's base corner values;
//   the "round corners" toggle simply multiplies the scale by 0.
// - When a background image is active, panel fills get an alpha derived from bg_alpha so the
//   image painted on the background layer shows through.

use egui::{Color32, Rounding};

#[derive(Clone, Copy, Debug)]
pub struct ThemeColors {
    pub bg: Color32,
    pub panel: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub widget_bg: Color32,
    pub widget_hover: Color32,
    pub widget_active: Color32,
    pub stroke: Color32,
}

impl ThemeColors {
    pub fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(22, 22, 26),
            panel: Color32::from_rgb(27, 27, 32),
            text: Color32::from_rgb(222, 222, 228),
            weak: Color32::from_rgb(150, 150, 158),
            accent: Color32::from_rgb(28, 150, 130), // default accent (28,150,130)
            widget_bg: Color32::from_rgb(60, 60, 64),
            widget_hover: Color32::from_rgb(74, 74, 80),
            widget_active: Color32::from_rgb(52, 52, 58),
            stroke: Color32::from_rgb(84, 84, 92),
        }
    }

    pub fn light() -> Self {
        Self {
            bg: Color32::from_rgb(244, 244, 246),
            panel: Color32::from_rgb(255, 255, 255),
            text: Color32::from_rgb(38, 38, 42),
            // weak: needs >= 4.5:1 on #f4f4f6 (WCAG AA body); gray 95 gives ~5.6:1.
            weak: Color32::from_rgb(95, 95, 103),
            accent: Color32::from_rgb(45, 75, 65), // light_adapt((28,150,130))
            widget_bg: Color32::from_rgb(222, 222, 226),
            widget_hover: Color32::from_rgb(204, 204, 210),
            widget_active: Color32::from_rgb(186, 186, 194),
            stroke: Color32::from_rgb(150, 150, 158),
        }
    }

    /// Per-channel linear interpolation between two palettes.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let l = |a: Color32, b: Color32| {
            let c = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
            Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
        };
        Self {
            bg: l(self.bg, other.bg),
            panel: l(self.panel, other.panel),
            text: l(self.text, other.text),
            weak: l(self.weak, other.weak),
            accent: l(self.accent, other.accent),
            widget_bg: l(self.widget_bg, other.widget_bg),
            widget_hover: l(self.widget_hover, other.widget_hover),
            widget_active: l(self.widget_active, other.widget_active),
            stroke: l(self.stroke, other.stroke),
        }
    }

    /// Max channel difference across every color; convergence test for the transition.
    pub fn max_diff(&self, other: &Self) -> u8 {
        let d = |a: Color32, b: Color32| {
            (a.r() as i32 - b.r() as i32)
                .abs()
                .max((a.g() as i32 - b.g() as i32).abs())
                .max((a.b() as i32 - b.b() as i32).abs()) as u8
        };
        d(self.bg, other.bg)
            .max(d(self.panel, other.panel))
            .max(d(self.text, other.text))
            .max(d(self.weak, other.weak))
            .max(d(self.accent, other.accent))
            .max(d(self.widget_bg, other.widget_bg))
            .max(d(self.widget_hover, other.widget_hover))
            .max(d(self.widget_active, other.widget_active))
            .max(d(self.stroke, other.stroke))
    }
}

/// Light-mode foreground helper: dims light colors (light gray/tint) designed
/// for dark backgrounds so they reach WCAG AA body 4.5:1 on light backgrounds
/// (#f4f4f6 / #ffffff). Dark mode returns the color unchanged.
pub fn light_adapt(c: Color32) -> Color32 {
    let f = |x: u8| ((x as f32 * 0.5).round().max(45.0) as u8);
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// Font-size scaling helper (batch 2): every explicit `RichText::size(base)` in the UI
/// keeps its hierarchy ratio; the global ui_font_scale slider (11..=20, default 14)
/// applies through `set_pixels_per_point` in tick_theme. This helper exists so the
/// scale factor is defined in one place:
///   `scaled_font(1.0, ui_font_scale)` => the ppp multiplier
///   `scaled_font(20.0, ui_font_scale)` => a title that would be 20px at default scale
/// (Do NOT apply this helper on top of ppp scaling, or sizes would be squared.)
pub fn scaled_font(base: f32, ui_font_scale: f32) -> f32 {
    base * (ui_font_scale.clamp(11.0, 20.0) / 14.0)
}

/// Build the target palette from config fields.
/// - mode: "dark" (night) / "light" (day)
/// - custom: when enabled, overrides accent + background with user colors
/// Accent is unified to RGB(28,150,130): dark uses it directly; light goes through
/// light_adapt to guarantee AA. Green is reserved for "success/running" semantics,
/// no longer used as a UI accent.
pub fn target_colors(
    mode: &str,
    custom: bool,
    custom_accent: (u8, u8, u8),
    custom_bg: (u8, u8, u8),
    custom_highlight: (u8, u8, u8),
) -> ThemeColors {
    let mut c = if mode == "light" {
        ThemeColors::light()
    } else {
        ThemeColors::dark()
    };
    if custom {
        c.accent = Color32::from_rgb(custom_accent.0, custom_accent.1, custom_accent.2);
        c.bg = Color32::from_rgb(custom_bg.0, custom_bg.1, custom_bg.2);
        c.panel = Color32::from_rgb(
            (custom_bg.0 as f32 * 1.12).round().min(255.0) as u8,
            (custom_bg.1 as f32 * 1.12).round().min(255.0) as u8,
            (custom_bg.2 as f32 * 1.12).round().min(255.0) as u8,
        );
    }
    // Auto-contrast text: derive text/weak/widget colors from the actual background
    // luminance so custom backgrounds never leave unreadable text.
    let bg = c.bg;
    auto_contrast(&mut c, bg);
    // Reserve the highlight color for future secondary-accent usage (e.g. toast emphasis).
    let _ = custom_highlight;
    c
}

/// Shift `bg` by `delta` with clamping to 0-255: used to generate contrast widget
/// colors on *any* background/material. Fixed light/dark grays no longer work — a
/// fixed gray melts into mid-luminance materials and buttons become invisible
/// (user feedback: background got translucent but buttons disappeared).
fn shade(bg: Color32, delta: i32) -> Color32 {
    let f = |v: u8| (v as i32 + delta).clamp(0, 255) as u8;
    Color32::from_rgb(f(bg.r()), f(bg.g()), f(bg.b()))
}

/// Pick readable foreground/control colors from the *actual visible* background
/// color (threshold ~140 luma).
/// When background effects (translucent/frosted/acrylic) are active, the real color
/// under the UI is "desktop texture + tint", not `ThemeColors.bg`; keeping the
/// theme's light text on a bright desktop becomes unreadable (user feedback).
/// The caller passes the composited material color; here we decide the foreground
/// colors and make control bases clearly darker/brighter than the material.
pub fn auto_contrast(c: &mut ThemeColors, bg: Color32) {
    let luma = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    // Accent must follow too: custom accents are "bright colors designed for dark
    // backgrounds" (teal/green); on bright materials they have almost no contrast
    // (user feedback: "light mode unreadable" partly comes from this). Push the
    // accent the opposite way by material luminance so it stays visible.
    let accent_luma = 0.299 * c.accent.r() as f32
        + 0.587 * c.accent.g() as f32
        + 0.114 * c.accent.b() as f32;
    if luma > 140.0 {
        // Bright background -> dark foreground; controls darker than the bg. Body
        // text near-black, secondary text also high-contrast ("blurred texture +
        // small text" is harder to read than a plain background, so go more extreme).
        c.text = Color32::from_rgb(8, 8, 10);
        c.weak = Color32::from_rgb(52, 52, 60);
        c.widget_bg = shade(bg, -58);
        c.widget_hover = shade(bg, -78);
        c.widget_active = shade(bg, -96);
        c.stroke = shade(bg, -84);
        if accent_luma > 120.0 {
            c.accent = shade(c.accent, -85);
        }
    } else {
        // Dark background -> light foreground; controls brighter than the bg.
        c.text = Color32::from_rgb(250, 250, 253);
        c.weak = Color32::from_rgb(200, 200, 208);
        c.widget_bg = shade(bg, 36);
        c.widget_hover = shade(bg, 56);
        c.widget_active = shade(bg, 20);
        c.stroke = shade(bg, 74);
        if accent_luma < 150.0 {
            c.accent = shade(c.accent, 70);
        }
    }
}

/// Apply the palette + corner radius + background translucency to the current context style.
/// Returns the window outer-corner radius (points, 0 = sharp) for the caller to apply
/// via a Win32 system region (SetWindowRgn), the only reliable way to round the
/// silhouette of a frameless opaque window.
///
/// `bg_style` / `overlay_alpha` come from the plugin background request:
/// * All non-default modes: panel fill alpha = `overlay_alpha` (slider opacity).
///   The desktop is captured by `backdrop.rs` and painted on the background layer,
///   so "see-through panels" rely on no OS transparency mechanism.
/// * `Default`: opaque (or legacy wallpaper route).
pub fn apply(
    ctx: &egui::Context,
    c: &ThemeColors,
    round_corners: bool,
    window_round_corners: bool,
    corner_scale: f32,
    window_corner_scale: f32,
    bg_alpha: f32,
    bg_enabled: bool,
    bg_style: crate::plugins::BgStyle,
    overlay_alpha: f32,
    // Composited visible color of the background material (desktop capture + tint).
    // Some = effect active: panels fully transparent (material painted on the
    // background layer) and this color drives the auto light/dark foreground pick
    // so the UI stays readable on any desktop.
    material: Option<Color32>,
    // Whether UI colors follow material luminance (user can disable in plugin page).
    // When disabled the theme palette is used; the caller then clamps material
    // intensity to 0.60 lower bound to keep readability.
    auto_contrast_material: bool,
    // Content scrim density 0.0-0.6: in material mode the content panel
    // (CentralPanel) is not fully transparent; it gets a translucent theme-color
    // backing so text has a stable base (improves small-text readability).
    content_scrim: f32,
) -> f32 {
    use crate::plugins::BgStyle;
    // When the effect is active, foreground/control colors follow the material's
    // actual color instead of the theme's own bg.
    let mut colors = *c;
    if let Some(m) = material {
        if auto_contrast_material {
            auto_contrast(&mut colors, m);
        }
    }
    let c = &colors;
    ctx.style_mut(|s| {
        let v = &mut s.visuals;
        // Material mode: content panels get a translucent theme-color backing
        // (content_scrim, default 0.5) so text has a stable base while the desktop
        // material stays visible (key to small-text readability; same idea as
        // Fluent's in-app acrylic tint).
        // Default mode: original logic (opaque / legacy wallpaper opacity).
        let panel_alpha = if material.is_some() {
            (content_scrim.clamp(0.0, 0.6) * 255.0).round() as u8
        } else {
            match bg_style {
                BgStyle::Acrylic | BgStyle::Frosted | BgStyle::Translucent => {
                    (overlay_alpha.clamp(0.0, 1.0) * 255.0).round() as u8
                }
                BgStyle::Default => {
                    if bg_enabled {
                        ((1.0 - bg_alpha.clamp(0.0, 1.0)) * 255.0) as u8
                    } else {
                        255
                    }
                }
            }
        };
        let bg_fill = Color32::from_rgba_unmultiplied(c.bg.r(), c.bg.g(), c.bg.b(), panel_alpha);
        // NOTE (egui 0.29): SidePanel / TopBottomPanel / CentralPanel default to
        // `panel_fill` — *not* `window_fill` (only Window / menu / popup use it).
        // Top bars and side bars rely on `.frame(...)` at each construction site
        // (see main.rs); in material mode `window_fill` becomes a "near-opaque
        // material" so side bars/popups share the content palette and auto-contrast
        // foreground colors hold on both sides.
        v.panel_fill = bg_fill;
        v.window_fill = match material {
            Some(m) => Color32::from_rgba_unmultiplied(m.r(), m.g(), m.b(), 236),
            None => Color32::from_rgb(c.panel.r(), c.panel.g(), c.panel.b()),
        };
        v.extreme_bg_color = match material {
            Some(m) => Color32::from_rgba_unmultiplied(m.r(), m.g(), m.b(), 240),
            None => Color32::from_rgb(c.bg.r(), c.bg.g(), c.bg.b()),
        };
        v.faint_bg_color = c.widget_hover;
        v.override_text_color = Some(c.text);
        // dark_mode tracks palette luminance: light theme sets false so egui's
        // built-in controls (scroll bars, selection) render in light mode and the
        // thumb does not melt into the light background.
        v.dark_mode = c.text.r() > 128;
        v.window_stroke.color = c.stroke;
        // Light mode uniformly darkens accent-derived text / selection / cursor to
        // avoid unreadable light accents on light backgrounds (same contrast issue).
        let accent_fg = if c.text.r() > 128 {
            c.accent
        } else {
            light_adapt(c.accent)
        };
        v.selection.bg_fill = accent_fg;
        v.selection.stroke.color = c.accent;
        v.hyperlink_color = accent_fg;
        v.text_cursor.stroke.color = accent_fg;
        // Widget state colors
        v.widgets.noninteractive.bg_fill = c.widget_bg;
        // In egui 0.29, `.weak()` text color = gray_out(body); weak tinted targets
        // use widgets.noninteractive.weak_bg_fill. In light mode the default 248
        // (near-white) makes weak text (group headers / captions) almost invisible
        // on light backgrounds; pressing to mid-gray reaches AA.
        v.widgets.noninteractive.weak_bg_fill = if c.text.r() > 128 {
            c.widget_bg
        } else {
            Color32::from_gray(120)
        };
        v.widgets.noninteractive.fg_stroke.color = c.weak;
        v.widgets.noninteractive.bg_stroke.color = c.stroke;
        v.widgets.inactive.bg_fill = c.widget_bg;
        v.widgets.inactive.weak_bg_fill = c.widget_bg;
        // Batch 2 scroll-bar enhancement: egui 0.29 scroll-bar handle color is taken from
        // the widget-state `fg_stroke` (when scroll.foreground_color=true) or `bg_fill`
        // otherwise; there is no dedicated scroll-bar color field. Text colors are already
        // pinned by override_text_color above, so we can safely drive the thumb with the
        // accent color (semi-transparency comes from ScrollStyle opacities below).
        v.widgets.inactive.fg_stroke.color = accent_fg;
        v.widgets.inactive.bg_stroke.color = c.stroke;
        v.widgets.hovered.bg_fill = c.widget_hover;
        v.widgets.hovered.weak_bg_fill = c.widget_hover;
        v.widgets.hovered.fg_stroke.color = accent_fg;
        v.widgets.hovered.bg_stroke.color = c.accent;
        v.widgets.active.bg_fill = c.widget_active;
        v.widgets.active.weak_bg_fill = c.widget_active;
        v.widgets.active.fg_stroke.color = accent_fg;
        v.widgets.active.bg_stroke.color = c.accent;
        v.widgets.open.bg_fill = c.widget_hover;
        v.widgets.open.weak_bg_fill = c.widget_hover;
        v.widgets.open.fg_stroke.color = c.text;
        v.widgets.open.bg_stroke.color = c.accent;
        // Corner radius: toggle off => scale 0 (sharp), default 1.0 => egui base values.
        // egui 0.29 uses Rounding (f32 corners) on WidgetVisuals / window / menu.
        let scale = if round_corners {
            corner_scale.clamp(0.0, 3.0)
        } else {
            0.0
        };
        let cr = |v: f32| (v * scale).round().clamp(0.0, 255.0);
        let w_r = Rounding::same(cr(4.0));
        v.widgets.noninteractive.rounding = w_r;
        v.widgets.inactive.rounding = w_r;
        v.widgets.hovered.rounding = w_r;
        v.widgets.active.rounding = w_r;
        v.widgets.open.rounding = w_r;
        let frame_r = Rounding::same(cr(6.0));
        v.window_rounding = frame_r;
        v.menu_rounding = frame_r;
        // Batch 2: scroll-bar styling. Solid 8px bars with a neutral handle (widget
        // bg_fill, not the accent color) so it reads as a traditional scroll bar
        // rather than an accent decoration line (user feedback: accent line looked
        // like a non-draggable decoration).
        let mut scroll = egui::style::ScrollStyle::solid();
        scroll.bar_width = 8.0;
        scroll.handle_min_length = 24.0;
        s.spacing.scroll = scroll;
    });
    // ---- Window outer-corner radius (feedback #2): the frameless window gets its
    // rounded silhouette from a Win32 system region (SetWindowRgn + CreateRoundRectRgn)
    // applied by the caller with the returned radius. Painting corner masks on the
    // Foreground layer cannot work on an opaque window (the mask is the same color as
    // the background), which is why the previous mask had no visible effect.
    let r = if window_round_corners {
        (12.0 * window_corner_scale.clamp(0.0, 3.0)).round().clamp(0.0, 48.0)
    } else {
        0.0
    };
    r
}
