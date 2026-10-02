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
            accent: Color32::from_rgb(34, 211, 238), // #22D3EE
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
            // weak 在 #f4f4f6 背景上需 ≥4.5:1（WCAG AA 正文）；95 灰对比度约 5.6:1。
            weak: Color32::from_rgb(95, 95, 103),
            accent: Color32::from_rgb(45, 106, 119), // light_adapt(#22D3EE)
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

/// 亮色模式前景色辅助：把为深色背景设计的浅色前景（浅灰/浅彩）压暗，
/// 使其在亮色背景（#f4f4f6 / #ffffff 附近）上达到 WCAG AA 正文 4.5:1。
/// 深色模式下原样返回。
pub fn light_adapt(c: Color32) -> Color32 {
    let f = |x: u8| ((x as f32 * 0.5).round().max(45.0) as u8);
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// Build the target palette from config fields.
/// - mode: "dark" (night) / "light" (day)
/// - custom: when enabled, overrides accent + background with user colors
/// 主强调色统一为 #22D3EE (34,211,238)：深色直接使用；亮色经 light_adapt 压暗保证 AA。
/// 绿色仅保留给"成功/运行中"等状态语义，不再作为界面强调色。
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
    // Reserve the highlight color for future secondary-accent usage (e.g. toast emphasis).
    let _ = custom_highlight;
    c
}

/// Apply the palette + corner radius + background translucency to the current context style.
pub fn apply(
    ctx: &egui::Context,
    c: &ThemeColors,
    round_corners: bool,
    corner_scale: f32,
    bg_alpha: f32,
    bg_enabled: bool,
) {
    ctx.style_mut(|s| {
        let v = &mut s.visuals;
        let panel_alpha = if bg_enabled {
            ((1.0 - bg_alpha.clamp(0.0, 1.0)) * 255.0) as u8
        } else {
            255
        };
        let bg_fill = Color32::from_rgba_unmultiplied(c.bg.r(), c.bg.g(), c.bg.b(), panel_alpha);
        let panel_fill =
            Color32::from_rgba_unmultiplied(c.panel.r(), c.panel.g(), c.panel.b(), panel_alpha);
        v.panel_fill = bg_fill;
        v.window_fill = panel_fill;
        v.extreme_bg_color = bg_fill;
        v.faint_bg_color = c.widget_hover;
        v.override_text_color = Some(c.text);
        // dark_mode 标志跟随调色板亮度：亮色主题置 false，使滚动条/选区等 egui 内建控件
        // 按浅色模式渲染（滚动条槽位/拖拽条不至于融进浅背景）。
        v.dark_mode = c.text.r() > 128;
        v.window_stroke.color = c.stroke;
        // 亮色模式统一把"强调色派生"的文字/选区/光标压暗，避免自定义浅色强调色
        // 在浅背景上不可读（图1 同源对比度问题）。
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
        // egui 0.29 中 .weak() 文字色 = gray_out(正文色)，混色目标取
        // widgets.noninteractive.weak_bg_fill。亮色模式下该值默认为 248(近白)，
        // 导致弱文字(分组标题/说明文字)在浅背景上几乎不可见；压到中灰使其达到 AA。
        v.widgets.noninteractive.weak_bg_fill = if c.text.r() > 128 {
            c.widget_bg
        } else {
            Color32::from_gray(120)
        };
        v.widgets.noninteractive.fg_stroke.color = c.weak;
        v.widgets.noninteractive.bg_stroke.color = c.stroke;
        v.widgets.inactive.bg_fill = c.widget_bg;
        v.widgets.inactive.weak_bg_fill = c.widget_bg;
        v.widgets.inactive.fg_stroke.color = c.text;
        v.widgets.inactive.bg_stroke.color = c.stroke;
        v.widgets.hovered.bg_fill = c.widget_hover;
        v.widgets.hovered.weak_bg_fill = c.widget_hover;
        v.widgets.hovered.fg_stroke.color = c.text;
        v.widgets.hovered.bg_stroke.color = c.accent;
        v.widgets.active.bg_fill = c.widget_active;
        v.widgets.active.weak_bg_fill = c.widget_active;
        v.widgets.active.fg_stroke.color = c.text;
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
    });
}
