//! The look of the interface: fonts, icons and colors.
//!
//! Inter (SIL Open Font License) and Lucide icons (ISC license) are embedded, cut down
//! to the characters the app uses. Their licenses are in `assets/fonts`.

use imgui::{FontConfig, FontGlyphRanges, FontId, FontSource, StyleColor};

const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const ICONS: &[u8] = include_bytes!("../assets/fonts/lucide-subset.ttf");

/// Lucide icon characters. Each must also be in `ICON_RANGES` and in the font subset,
/// which `scripts/make-fonts.sh` remakes.
pub mod icon {
    pub const PART: char = '\u{e4b3}';
    pub const VERTEX: char = '\u{e345}';
    pub const EDGE: char = '\u{e51d}';
    pub const FACE: char = '\u{e167}';
    pub const CLICK: char = '\u{e1c3}';
    pub const BOX_SELECT: char = '\u{e509}';
    pub const CIRCLE_SELECT: char = '\u{e4b0}';
    pub const LASSO: char = '\u{e1ce}';
    pub const UNDO: char = '\u{e2a1}';
    pub const REDO: char = '\u{e2a0}';
    pub const MOVE: char = '\u{e2e5}';
    pub const ROTATE: char = '\u{e2ea}';
    pub const SCALE: char = '\u{e2eb}';
    pub const EXTRUDE: char = '\u{e45a}';
    pub const INSET: char = '\u{e60e}';
    pub const DUPLICATE: char = '\u{e09e}';
    pub const MIRROR: char = '\u{e35e}';
    pub const MERGE: char = '\u{e43f}';
    pub const FILL: char = '\u{e2e6}';
    pub const SUBDIVIDE: char = '\u{e4ff}';
    pub const LOOP_CUT: char = '\u{e439}';
    pub const SLIDE: char = '\u{e43d}';
    pub const BRIDGE: char = '\u{e74f}';
    pub const BEVEL: char = '\u{e648}';
    pub const DELETE: char = '\u{e18e}';
    pub const EXPORT: char = '\u{e0b2}';
    pub const HOME: char = '\u{e0f5}';
    pub const ORTHO: char = '\u{e257}';
    pub const CUBE: char = '\u{e061}';
    pub const CONE: char = '\u{e523}';
    pub const CYLINDER: char = '\u{e525}';
    pub const UV_SPHERE: char = '\u{e0e8}';
    pub const ICOSPHERE: char = '\u{e0f3}';
    pub const GRID: char = '\u{e0e9}';
    pub const CIRCLE: char = '\u{e076}';
    pub const MERGE_CLOSE: char = '\u{e44c}';
    pub const WARNING: char = '\u{e193}';
    pub const INFO: char = '\u{e0f9}';
    pub const CHECK: char = '\u{e06c}';
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const FOLDER: char = '\u{e247}';
}

/// Every icon, as start and end pairs, closed with a zero. Keeping the list short keeps
/// the font texture small.
static ICON_RANGES: [u32; 73] = [
    0xe061, 0xe061, 0xe06c, 0xe06c, 0xe076, 0xe076, 0xe09e, 0xe09e, 0xe0b2, 0xe0b2, 0xe0e8, 0xe0e9,
    0xe0f3, 0xe0f3, 0xe0f5, 0xe0f5, 0xe0f9, 0xe0f9, 0xe167, 0xe167, 0xe18e, 0xe18e, 0xe193, 0xe193,
    0xe1c3, 0xe1c3, 0xe1ce, 0xe1ce, 0xe247, 0xe247, 0xe257, 0xe257, 0xe2a0, 0xe2a1, 0xe2e5, 0xe2e6,
    0xe2ea, 0xe2eb, 0xe345, 0xe345, 0xe35e, 0xe35e, 0xe439, 0xe439, 0xe43d, 0xe43d, 0xe43f, 0xe43f,
    0xe44c, 0xe44c, 0xe45a, 0xe45a, 0xe4b0, 0xe4b0, 0xe4b3, 0xe4b3, 0xe4ff, 0xe4ff, 0xe509, 0xe509,
    0xe51d, 0xe51d, 0xe523, 0xe523, 0xe525, 0xe525, 0xe60e, 0xe60e, 0xe648, 0xe648, 0xe74f, 0xe74f,
    0,
];

/// Latin-1 plus a few typographic marks used in the interface.
static TEXT_RANGES: [u32; 13] = [
    0x0020, 0x00ff, 0x2013, 0x2014, 0x2018, 0x201d, 0x2022, 0x2022, 0x2026, 0x2026, 0x2190, 0x2193,
    0,
];

/// Colors, as red, green, blue and opacity from 0 to 1.
pub mod color {
    pub const WINDOW: [f32; 4] = [0.118, 0.125, 0.137, 0.97];
    pub const BAR: [f32; 4] = [0.090, 0.094, 0.106, 1.0];
    pub const BORDER: [f32; 4] = [0.20, 0.21, 0.23, 1.0];
    pub const FRAME: [f32; 4] = [0.165, 0.173, 0.192, 1.0];
    pub const FRAME_HOVER: [f32; 4] = [0.212, 0.224, 0.247, 1.0];
    pub const FRAME_ACTIVE: [f32; 4] = [0.255, 0.267, 0.294, 1.0];
    pub const TEXT: [f32; 4] = [0.91, 0.92, 0.93, 1.0];
    pub const MUTED: [f32; 4] = [0.58, 0.61, 0.65, 1.0];
    pub const FAINT: [f32; 4] = [0.40, 0.42, 0.46, 1.0];
    /// Matches the orange used for selected parts in the viewport.
    pub const ACCENT: [f32; 4] = [0.957, 0.553, 0.157, 1.0];
    pub const ACCENT_HOVER: [f32; 4] = [1.0, 0.63, 0.27, 1.0];
    pub const ACCENT_SOFT: [f32; 4] = [0.957, 0.553, 0.157, 0.20];
    pub const ON_ACCENT: [f32; 4] = [0.10, 0.07, 0.04, 1.0];
    pub const WARNING: [f32; 4] = [0.98, 0.76, 0.30, 1.0];
    pub const DANGER: [f32; 4] = [0.95, 0.40, 0.40, 1.0];
    pub const AXIS: [[f32; 4]; 3] = [
        [0.93, 0.33, 0.38, 1.0],
        [0.55, 0.80, 0.25, 1.0],
        [0.30, 0.56, 0.96, 1.0],
    ];
    pub const OVERLAY: [f32; 4] = [0.08, 0.085, 0.095, 0.88];
}

/// ImGui allows one context at a time, so tests that make one take turns.
#[cfg(test)]
pub static IMGUI_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The fonts in use.
#[derive(Debug, Clone, Copy)]
pub struct Fonts {
    /// Small headings.
    pub heading: FontId,
    /// Large icons for tool buttons, with semibold text.
    pub tool: FontId,
    /// Large text and icons for the empty-scene hint.
    pub title: FontId,
}

fn text(data: &'static [u8], size: f32) -> FontSource<'static> {
    FontSource::TtfData {
        data,
        size_pixels: size,
        config: Some(FontConfig {
            glyph_ranges: FontGlyphRanges::from_slice(&TEXT_RANGES),
            oversample_h: 2,
            oversample_v: 1,
            pixel_snap_h: true,
            ..FontConfig::default()
        }),
    }
}

fn icons(size: f32) -> FontSource<'static> {
    FontSource::TtfData {
        data: ICONS,
        size_pixels: size,
        config: Some(FontConfig {
            glyph_ranges: FontGlyphRanges::from_slice(&ICON_RANGES),
            // Lucide draws on a grid that sits a little high next to Inter's letters.
            glyph_offset: [0.0, size * 0.12],
            glyph_min_advance_x: size,
            oversample_h: 1,
            oversample_v: 1,
            pixel_snap_h: true,
            ..FontConfig::default()
        }),
    }
}

/// Adds the fonts and sets the colors and shapes. Call once, before the font texture is
/// built.
pub fn install(context: &mut imgui::Context) -> Fonts {
    let atlas = context.fonts();
    atlas.tex_glyph_padding = 1;
    // The first font added is the default, used for body text with small icons.
    atlas.add_font(&[text(INTER_REGULAR, 15.0), icons(15.0)]);
    let heading = atlas.add_font(&[text(INTER_SEMIBOLD, 11.5)]);
    let tool = atlas.add_font(&[text(INTER_SEMIBOLD, 13.0), icons(19.0)]);
    let title = atlas.add_font(&[text(INTER_SEMIBOLD, 22.0), icons(34.0)]);

    let style = context.style_mut();
    style.window_rounding = 10.0;
    style.child_rounding = 8.0;
    style.frame_rounding = 6.0;
    style.popup_rounding = 8.0;
    style.grab_rounding = 6.0;
    style.scrollbar_rounding = 6.0;
    style.tab_rounding = 6.0;
    style.window_border_size = 1.0;
    style.frame_border_size = 0.0;
    style.popup_border_size = 1.0;
    style.window_padding = [12.0, 12.0];
    style.frame_padding = [9.0, 6.0];
    style.item_spacing = [8.0, 8.0];
    style.item_inner_spacing = [8.0, 6.0];
    style.scrollbar_size = 10.0;
    style.grab_min_size = 10.0;
    style.window_title_align = [0.0, 0.5];
    style.button_text_align = [0.5, 0.5];
    style.anti_aliased_lines = true;
    style.anti_aliased_fill = true;

    use color::*;
    let set = |style: &mut imgui::Style, which: StyleColor, value: [f32; 4]| {
        style.colors[which as usize] = value;
    };
    set(style, StyleColor::Text, TEXT);
    set(style, StyleColor::TextDisabled, FAINT);
    set(style, StyleColor::WindowBg, WINDOW);
    set(style, StyleColor::ChildBg, [0.0; 4]);
    set(style, StyleColor::PopupBg, [0.13, 0.137, 0.15, 0.99]);
    set(style, StyleColor::Border, BORDER);
    set(style, StyleColor::BorderShadow, [0.0; 4]);
    set(style, StyleColor::FrameBg, FRAME);
    set(style, StyleColor::FrameBgHovered, FRAME_HOVER);
    set(style, StyleColor::FrameBgActive, FRAME_ACTIVE);
    set(style, StyleColor::TitleBg, BAR);
    set(style, StyleColor::TitleBgActive, BAR);
    set(style, StyleColor::TitleBgCollapsed, BAR);
    set(style, StyleColor::MenuBarBg, BAR);
    set(style, StyleColor::ScrollbarBg, [0.0; 4]);
    set(style, StyleColor::ScrollbarGrab, [0.26, 0.27, 0.30, 1.0]);
    set(
        style,
        StyleColor::ScrollbarGrabHovered,
        [0.33, 0.34, 0.38, 1.0],
    );
    set(
        style,
        StyleColor::ScrollbarGrabActive,
        [0.40, 0.42, 0.46, 1.0],
    );
    set(style, StyleColor::CheckMark, ACCENT);
    set(style, StyleColor::SliderGrab, ACCENT);
    set(style, StyleColor::SliderGrabActive, ACCENT_HOVER);
    set(style, StyleColor::Button, FRAME);
    set(style, StyleColor::ButtonHovered, FRAME_HOVER);
    set(style, StyleColor::ButtonActive, FRAME_ACTIVE);
    set(style, StyleColor::Header, FRAME);
    set(style, StyleColor::HeaderHovered, FRAME_HOVER);
    set(style, StyleColor::HeaderActive, FRAME_ACTIVE);
    set(style, StyleColor::Separator, BORDER);
    set(style, StyleColor::SeparatorHovered, BORDER);
    set(style, StyleColor::SeparatorActive, BORDER);
    set(style, StyleColor::ResizeGrip, [0.0; 4]);
    set(style, StyleColor::ResizeGripHovered, [0.0; 4]);
    set(style, StyleColor::ResizeGripActive, [0.0; 4]);
    set(style, StyleColor::TextSelectedBg, ACCENT_SOFT);
    set(style, StyleColor::NavHighlight, ACCENT);
    set(style, StyleColor::ModalWindowDimBg, [0.0, 0.0, 0.0, 0.45]);

    Fonts {
        heading,
        tool,
        title,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_pairs_ending_in_zero() {
        for ranges in [&ICON_RANGES[..], &TEXT_RANGES[..]] {
            assert_eq!(ranges.len() % 2, 1);
            assert_eq!(*ranges.last().unwrap(), 0);
            let pairs: Vec<&[u32]> = ranges[..ranges.len() - 1].chunks(2).collect();
            for pair in &pairs {
                assert!(pair[0] <= pair[1] && pair[0] != 0, "{pair:?}");
            }
            // Dear ImGui panics on overlapping ranges.
            for two in pairs.windows(2) {
                assert!(two[0][1] < two[1][0], "{two:?} overlap or are out of order");
            }
        }
    }

    #[test]
    fn every_icon_is_listed_and_the_font_texture_is_small() {
        use icon::*;
        let all = [
            PART,
            VERTEX,
            EDGE,
            FACE,
            CLICK,
            BOX_SELECT,
            CIRCLE_SELECT,
            LASSO,
            UNDO,
            REDO,
            MOVE,
            ROTATE,
            SCALE,
            EXTRUDE,
            INSET,
            DUPLICATE,
            MIRROR,
            MERGE,
            FILL,
            SUBDIVIDE,
            LOOP_CUT,
            SLIDE,
            BRIDGE,
            BEVEL,
            DELETE,
            EXPORT,
            HOME,
            ORTHO,
            CUBE,
            CONE,
            CYLINDER,
            UV_SPHERE,
            ICOSPHERE,
            GRID,
            CIRCLE,
            MERGE_CLOSE,
            WARNING,
            INFO,
            CHECK,
            FOLDER,
        ];
        let _turn = IMGUI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut context = imgui::Context::create();
        let fonts = install(&mut context);
        context.fonts().build_alpha8_texture();
        for c in all {
            let code = c as u32;
            let listed = ICON_RANGES[..ICON_RANGES.len() - 1]
                .chunks(2)
                .any(|p| (p[0]..=p[1]).contains(&code));
            assert!(listed, "{code:#x} is missing from ICON_RANGES");
        }
        // The texture must stay small for a laptop with 128 MB of video memory.
        let texture = context.fonts().build_alpha8_texture();
        assert!(texture.width * texture.height <= 1024 * 1024);
        let _ = fonts;
    }
}
