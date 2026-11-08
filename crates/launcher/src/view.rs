//! What the launcher window shows, as a list of shapes and clickable areas, without
//! any Windows code. `gui.rs` draws the list with GDI+ and sends clicks back; tests
//! check the layout, and can draw it as an SVG picture for review.
//!
//! Sizes are in pixels at 100% display scaling; the window multiplies them.

use crate::core::{self, Found, SDL2};
use crate::settings::{Engines, LauncherSettings, Renderer, Support};
use crate::update::{self, Release, Version};

/// The window's inside, before scaling.
pub const WIDTH: f32 = 640.0;
pub const HEIGHT: f32 = 500.0;
const MARGIN: f32 = 28.0;
const TOP: f32 = 24.0;
const BOTTOM: f32 = 24.0;

/// Colors as 0xAARRGGBB, Mixer's theme.
pub mod color {
    pub const WINDOW: u32 = 0xFF1E_2023;
    pub const FRAME: u32 = 0xFF2A_2C31;
    pub const FRAME_HOVER: u32 = 0xFF36_393F;
    pub const BORDER: u32 = 0xFF33_363B;
    pub const LINE: u32 = 0x10FF_FFFF;
    pub const TEXT: u32 = 0xFFE8_EAED;
    pub const MUTED: u32 = 0xFF94_9BA6;
    pub const FAINT: u32 = 0xFF66_6B75;
    pub const ACCENT: u32 = 0xFFF4_8D28;
    pub const ACCENT_HOVER: u32 = 0xFFFF_A145;
    pub const ACCENT_SOFT: u32 = 0x1FF4_8D28;
    pub const ACCENT_EDGE: u32 = 0x66F4_8D28;
    pub const ON_ACCENT: u32 = 0xFF1A_1209;
    pub const OK: u32 = 0xFF7F_C84A;
    pub const OK_SOFT: u32 = 0x2E7F_C84A;
    pub const WARNING: u32 = 0xFFFA_C24D;
    pub const WARNING_SOFT: u32 = 0x1FFA_C24D;
    pub const DANGER: u32 = 0xFFF2_6666;
    pub const DANGER_SOFT: u32 = 0x24F2_6666;
    pub const LOG: u32 = 0xFF12_1315;
    pub const LOG_TEXT: u32 = 0xFF9A_A1AB;
}

/// The text styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Font {
    Body,
    Small,
    Strong,
    Heading,
    Title,
    Button,
    BigButton,
    Mono,
}

impl Font {
    /// Size in pixels.
    pub fn size(self) -> f32 {
        match self {
            Font::Body | Font::Button => 13.0,
            Font::Small => 12.0,
            Font::Strong => 13.5,
            Font::Heading => 16.0,
            Font::Title => 20.0,
            Font::BigButton => 15.0,
            Font::Mono => 11.5,
        }
    }

    pub fn semibold(self) -> bool {
        matches!(
            self,
            Font::Strong | Font::Heading | Font::Title | Font::BigButton
        )
    }

    /// Distance between lines.
    pub fn line(self) -> f32 {
        (self.size() * 1.5).round()
    }
}

/// Something to draw. Rectangles are [x, y, width, height].
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Fill {
        rect: [f32; 4],
        radius: f32,
        color: u32,
    },
    Outline {
        rect: [f32; 4],
        radius: f32,
        color: u32,
        width: f32,
    },
    /// One line of text; `y` is the top of the line box.
    Text {
        x: f32,
        y: f32,
        text: String,
        font: Font,
        color: u32,
    },
    Line {
        from: [f32; 2],
        to: [f32; 2],
        color: u32,
        width: f32,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        fill: Option<u32>,
        ring: Option<(u32, f32)>,
    },
    /// A check mark inside the rectangle.
    Tick { rect: [f32; 4], color: u32 },
    /// An X inside the rectangle.
    Cross { rect: [f32; 4], color: u32 },
    /// A downward arrow inside the rectangle.
    Down { rect: [f32; 4], color: u32 },
    /// A filled triangle pointing right, as on a play button.
    Play { rect: [f32; 4], color: u32 },
    /// An exclamation mark inside the rectangle.
    Alert { rect: [f32; 4], color: u32 },
}

/// What a click can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    StartMixer,
    UpdateNow,
    NotNow,
    Advanced,
    Back,
    Save,
    Repair,
    Pick(Renderer),
    ToggleUpdates,
    InstallAndStart,
    Close,
    TryAgain,
    ToggleLog,
    Cancel,
}

/// A clickable area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub rect: [f32; 4],
    pub action: Action,
}

/// One frame of the window.
#[derive(Debug, Default)]
pub struct Frame {
    pub shapes: Vec<Shape>,
    pub hits: Vec<Hit>,
    /// What Enter does.
    pub enter: Option<Action>,
    /// What Esc does.
    pub escape: Option<Action>,
}

impl Frame {
    /// The action under a point, if any.
    pub fn hit(&self, point: [f32; 2]) -> Option<Action> {
        self.hits.iter().rev().find_map(|hit| {
            let [x, y, w, h] = hit.rect;
            (point[0] >= x && point[0] < x + w && point[1] >= y && point[1] < y + h)
                .then_some(hit.action)
        })
    }
}

/// The pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Mixer is ready: start it, or update it.
    Home,
    /// Something Mixer needs is missing.
    Welcome,
    Advanced,
    /// A job is running or has just ended.
    Working,
}

/// What is known about updates.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateState {
    Off,
    Checking,
    UpToDate,
    Failed(String),
    Available(Release),
    /// Available, but "Not now" was pressed.
    Dismissed,
}

/// The graphics check.
#[derive(Debug, Clone, PartialEq)]
pub struct Graphics {
    pub ok: bool,
    /// Such as "Intel HD Graphics 3000 · OpenGL 3.1", or what went wrong.
    pub text: String,
}

/// A job's progress, for the Working page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JobView {
    pub title: String,
    pub steps: Vec<String>,
    pub current: usize,
    /// The current step's progress and a line such as "1.9 MB of 3.1 MB".
    pub progress: Option<(f32, String)>,
    pub log: Vec<String>,
    pub failed: Option<String>,
    pub finished: bool,
    /// Whether Cancel is offered.
    pub cancellable: bool,
    /// After finishing: whether the window offers Start Mixer (when the job did not).
    pub offer_start: bool,
}

/// Everything the window shows.
#[derive(Debug, Clone)]
pub struct Model {
    pub page: Page,
    pub folder: String,
    pub installed: Version,
    pub mixer_present: bool,
    pub sdl: Found,
    pub graphics: Graphics,
    pub update: UpdateState,
    pub settings: LauncherSettings,
    /// The Advanced page's choices before Save.
    pub draft: LauncherSettings,
    /// Which engines this PC offers, as Mixer found out.
    pub engines: Engines,
    pub job: JobView,
    pub show_log: bool,
    pub hover: Option<Action>,
    pub pressed: Option<Action>,
    /// A problem to show on the Home or Welcome page.
    pub message: Option<String>,
}

impl Model {
    /// The page to show when nothing is running.
    pub fn resting_page(&self) -> Page {
        if self.sdl == Found::Installed && self.mixer_present {
            Page::Home
        } else {
            Page::Welcome
        }
    }
}

/// Measures a line of text's width.
pub type Measure<'a> = &'a dyn Fn(&str, Font) -> f32;

/// Breaks text into lines no wider than `width`, at spaces where possible.
pub fn wrap(text: &str, font: Font, width: f32, measure: Measure) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if measure(&candidate, font) <= width || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
            // A single word wider than the line is cut where it fits.
            while measure(&line, font) > width && line.chars().count() > 1 {
                let mut cut = line.len();
                while cut > 0 && measure(&line[..cut], font) > width {
                    cut = line[..cut].char_indices().next_back().map_or(0, |(i, _)| i);
                }
                let cut = cut.max(line.chars().next().map_or(1, char::len_utf8));
                lines.push(line[..cut].to_string());
                line = line[cut..].to_string();
            }
        }
        lines.push(line);
    }
    lines
}

/// Shortens text with "…" to fit `width`.
fn fit(text: &str, font: Font, width: f32, measure: Measure) -> String {
    if measure(text, font) <= width {
        return text.to_string();
    }
    let mut end = text.len();
    while end > 0 {
        end = text[..end].char_indices().next_back().map_or(0, |(i, _)| i);
        let candidate = format!("{}\u{2026}", &text[..end]);
        if measure(&candidate, font) <= width {
            return candidate;
        }
    }
    "\u{2026}".to_string()
}

/// Builds a frame, keeping track of where the next thing goes.
struct Builder<'a> {
    frame: Frame,
    model: &'a Model,
    measure: Measure<'a>,
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Primary,
    Secondary,
}

impl Builder<'_> {
    fn add(&mut self, shape: Shape) {
        self.frame.shapes.push(shape);
    }

    fn fill(&mut self, rect: [f32; 4], radius: f32, color: u32) {
        self.add(Shape::Fill {
            rect,
            radius,
            color,
        });
    }

    fn text(&mut self, x: f32, y: f32, text: impl Into<String>, font: Font, color: u32) {
        self.add(Shape::Text {
            x,
            y,
            text: text.into(),
            font,
            color,
        });
    }

    /// Wrapped text; returns its height.
    fn paragraph(&mut self, x: f32, y: f32, width: f32, text: &str, font: Font, color: u32) -> f32 {
        let lines = wrap(text, font, width, self.measure);
        for (i, line) in lines.iter().enumerate() {
            self.text(x, y + i as f32 * font.line(), line.clone(), font, color);
        }
        lines.len() as f32 * font.line()
    }

    fn hovered(&self, action: Action) -> bool {
        self.model.hover == Some(action)
    }

    fn width_of(&self, text: &str, font: Font) -> f32 {
        (self.measure)(text, font)
    }

    /// A button's width for its label.
    fn button_width(&self, label: &str, font: Font, icon: bool) -> f32 {
        let padding = if font == Font::BigButton { 28.0 } else { 16.0 };
        (self.width_of(label, font) + 2.0 * padding + if icon { 22.0 } else { 0.0 }).ceil()
    }

    /// A button with its top-left corner at `x`, `y`.
    #[allow(clippy::too_many_arguments)]
    fn button(
        &mut self,
        x: f32,
        y: f32,
        height: f32,
        label: &str,
        font: Font,
        style: Style,
        play: bool,
        action: Action,
    ) -> f32 {
        let width = self.button_width(label, font, play);
        let rect = [x, y, width, height];
        let hover = self.hovered(action);
        let pressed = self.model.pressed == Some(action) && hover;
        let (fill, text) = match style {
            Style::Primary => (
                if hover {
                    color::ACCENT_HOVER
                } else {
                    color::ACCENT
                },
                color::ON_ACCENT,
            ),
            Style::Secondary => (
                if hover {
                    color::FRAME_HOVER
                } else {
                    color::FRAME
                },
                color::TEXT,
            ),
        };
        let shift = if pressed { 1.0 } else { 0.0 };
        self.fill(rect, 8.0, fill);
        if style == Style::Secondary {
            self.add(Shape::Outline {
                rect,
                radius: 8.0,
                color: color::BORDER,
                width: 1.0,
            });
        }
        let text_width = self.width_of(label, font);
        let mut left = x + (width - text_width - if play { 22.0 } else { 0.0 }) / 2.0;
        if play {
            let size = 12.0;
            self.add(Shape::Play {
                rect: [left, y + (height - size) / 2.0 + shift, size, size],
                color: text,
            });
            left += 22.0;
        }
        self.text(
            left,
            y + (height - font.line()) / 2.0 + shift,
            label,
            font,
            text,
        );
        self.frame.hits.push(Hit { rect, action });
        width
    }

    /// An underlined link; returns its width.
    fn link(&mut self, x: f32, y: f32, label: &str, action: Action) -> f32 {
        let font = Font::Body;
        let width = self.width_of(label, font);
        let tint = if self.hovered(action) {
            color::TEXT
        } else {
            color::MUTED
        };
        self.text(x, y, label, font, tint);
        let base = y + font.line() - 2.0;
        self.add(Shape::Line {
            from: [x, base],
            to: [x + width, base],
            color: tint,
            width: 1.0,
        });
        self.frame.hits.push(Hit {
            rect: [x - 4.0, y - 4.0, width + 8.0, font.line() + 8.0],
            action,
        });
        width
    }

    fn tag(&mut self, x: f32, y: f32, label: &str, fill: u32, text: u32) -> f32 {
        let width = self.width_of(label, Font::Small) + 14.0;
        self.fill([x, y, width, 20.0], 5.0, fill);
        self.text(x + 7.0, y + 1.0, label, Font::Small, text);
        width
    }

    fn logo(&mut self, x: f32, y: f32) {
        self.fill([x, y, 40.0, 40.0], 11.0, color::ACCENT);
    }

    fn head(&mut self, title: &str, line: &str) -> f32 {
        self.logo(MARGIN, TOP);
        self.text(MARGIN + 54.0, TOP - 1.0, title, Font::Title, color::TEXT);
        self.text(MARGIN + 54.0, TOP + 24.0, line, Font::Small, color::MUTED);
        TOP + 40.0 + 18.0
    }

    /// The update card or line. Returns the height used.
    fn update_status(&mut self, y: f32) -> f32 {
        let width = WIDTH - 2.0 * MARGIN;
        match &self.model.update {
            UpdateState::Available(release) => {
                let inner = width - 32.0;
                // Measured first, drawn after the card behind it.
                let title = format!(
                    "Mixer {} is ready to download ({})",
                    release.version,
                    core::size_text(release.zip.size)
                );
                let notes = update::notes_summary(&release.notes);
                let note_lines: Vec<Vec<String>> = notes
                    .iter()
                    .map(|n| wrap(n, Font::Small, inner - 18.0, self.measure))
                    .collect();
                let lines: usize = note_lines.iter().map(Vec::len).sum();
                let height = 16.0
                    + Font::Strong.line()
                    + if lines > 0 {
                        6.0 + lines as f32 * 19.0
                    } else {
                        0.0
                    }
                    + 12.0
                    + 28.0
                    + 16.0;
                let rect = [MARGIN, y, width, height];
                self.fill(rect, 10.0, color::FRAME);
                self.fill(rect, 10.0, color::ACCENT_SOFT);
                self.add(Shape::Outline {
                    rect,
                    radius: 10.0,
                    color: color::ACCENT_EDGE,
                    width: 1.0,
                });
                let x = MARGIN + 16.0;
                let mut at = y + 16.0;
                self.text(x, at, title, Font::Strong, color::TEXT);
                at += Font::Strong.line() + if lines > 0 { 6.0 } else { 0.0 };
                for note in note_lines {
                    for (i, line) in note.into_iter().enumerate() {
                        if i == 0 {
                            self.add(Shape::Circle {
                                center: [x + 5.0, at + 9.5],
                                radius: 2.0,
                                fill: Some(color::MUTED),
                                ring: None,
                            });
                        }
                        self.text(x + 18.0, at, line, Font::Small, color::MUTED);
                        at += 19.0;
                    }
                }
                at += 12.0;
                let used = self.button(
                    x,
                    at,
                    28.0,
                    "Update now",
                    Font::Button,
                    Style::Primary,
                    false,
                    Action::UpdateNow,
                );
                self.button(
                    x + used + 8.0,
                    at,
                    28.0,
                    "Not now",
                    Font::Button,
                    Style::Secondary,
                    false,
                    Action::NotNow,
                );
                height + 12.0
            }
            UpdateState::Checking => {
                self.text(
                    MARGIN,
                    y,
                    "Checking GitHub for a newer version\u{2026}",
                    Font::Small,
                    color::FAINT,
                );
                Font::Small.line() + 10.0
            }
            UpdateState::UpToDate => {
                self.add(Shape::Tick {
                    rect: [MARGIN, y + 3.0, 12.0, 12.0],
                    color: color::OK,
                });
                self.text(
                    MARGIN + 18.0,
                    y,
                    "You have the newest version.",
                    Font::Small,
                    color::FAINT,
                );
                Font::Small.line() + 10.0
            }
            UpdateState::Failed(reason) => {
                let text = format!("Could not check for updates. {reason}");
                let lines = wrap(&text, Font::Small, width, self.measure);
                let shown = lines.len().min(2);
                for (i, line) in lines.into_iter().take(2).enumerate() {
                    self.text(
                        MARGIN,
                        y + i as f32 * Font::Small.line(),
                        line,
                        Font::Small,
                        color::FAINT,
                    );
                }
                shown as f32 * Font::Small.line() + 10.0
            }
            UpdateState::Off => {
                self.text(
                    MARGIN,
                    y,
                    "Update checks are off. Turn them on under Advanced.",
                    Font::Small,
                    color::FAINT,
                );
                Font::Small.line() + 10.0
            }
            UpdateState::Dismissed => 0.0,
        }
    }

    /// A card of rows: an icon, a name and a value. Returns the height used.
    fn checks(&mut self, y: f32, rows: &[Row]) -> f32 {
        let width = WIDTH - 2.0 * MARGIN;
        let key_x = MARGIN + 44.0;
        let value_x = MARGIN + 184.0;
        let value_width = MARGIN + width - 16.0 - value_x;
        let mut heights = Vec::new();
        for (_, _, value, tag) in rows {
            let tag_width = tag.map_or(0.0, |(t, _, _)| self.width_of(t, Font::Small) + 22.0);
            let lines = wrap(value, Font::Body, value_width - tag_width, self.measure).len();
            heights.push((lines as f32 * Font::Body.line()).max(Font::Body.line()) + 16.0);
        }
        let height = heights.iter().sum::<f32>() + 12.0;
        self.fill([MARGIN, y, width, height], 10.0, color::FRAME);
        let mut at = y + 6.0;
        for (i, ((icon, key, value, tag), row)) in rows.iter().zip(&heights).enumerate() {
            if i > 0 {
                self.add(Shape::Line {
                    from: [MARGIN + 16.0, at],
                    to: [MARGIN + width - 16.0, at],
                    color: color::LINE,
                    width: 1.0,
                });
            }
            let text_y = at + 8.0;
            let icon_rect = [MARGIN + 18.0, text_y + 4.0, 12.0, 12.0];
            match icon {
                Icon::Ok => self.add(Shape::Tick {
                    rect: icon_rect,
                    color: color::OK,
                }),
                Icon::Get => self.add(Shape::Down {
                    rect: icon_rect,
                    color: color::ACCENT,
                }),
                Icon::Warn => self.add(Shape::Alert {
                    rect: icon_rect,
                    color: color::WARNING,
                }),
            }
            self.text(key_x, text_y, *key, Font::Body, color::MUTED);
            let tag_width = tag.map_or(0.0, |(t, _, _)| self.width_of(t, Font::Small) + 22.0);
            let lines = wrap(value, Font::Body, value_width - tag_width, self.measure);
            let last_width = lines.last().map_or(0.0, |l| self.width_of(l, Font::Body));
            let count = lines.len();
            for (j, line) in lines.into_iter().enumerate() {
                self.text(
                    value_x,
                    text_y + j as f32 * Font::Body.line(),
                    line,
                    Font::Body,
                    color::TEXT,
                );
            }
            if let Some((label, fill, text)) = tag {
                let tag_y = text_y + (count as f32 - 1.0) * Font::Body.line();
                self.tag(value_x + last_width + 8.0, tag_y, label, *fill, *text);
            }
            at += row;
        }
        height + 12.0
    }

    fn sdl_row(&self) -> Row {
        let name = "SDL2";
        match self.model.sdl {
            Found::Installed => (Icon::Ok, name, format!("{} installed", SDL2.version), None),
            Found::Missing => (
                Icon::Get,
                name,
                format!(
                    "Needed for the window, mouse and keyboard. {} from SDL's official GitHub.",
                    core::size_text(SDL2.size)
                ),
                None,
            ),
            Found::Different => (
                Icon::Get,
                name,
                format!(
                    "A different SDL2.dll is here; it will be replaced. {} from SDL's official \
                     GitHub.",
                    core::size_text(SDL2.size)
                ),
                None,
            ),
        }
    }

    fn graphics_row(&self) -> Row {
        let graphics = &self.model.graphics;
        (
            if graphics.ok { Icon::Ok } else { Icon::Warn },
            "Graphics",
            graphics.text.clone(),
            None,
        )
    }

    /// The bottom row's top.
    fn foot_y(height: f32) -> f32 {
        HEIGHT - BOTTOM - height
    }

    /// Buttons from the right edge, right to left; returns where the leftmost starts.
    fn right_buttons(&mut self, buttons: &[(&str, Style, bool, Action)], big: bool) -> f32 {
        let height = if big { 44.0 } else { 36.0 };
        let y = Self::foot_y(height);
        let mut right = WIDTH - MARGIN;
        for &(label, style, play, action) in buttons {
            let font = if big && style == Style::Primary {
                Font::BigButton
            } else {
                Font::Button
            };
            let width = self.button_width(label, font, play);
            right -= width;
            self.button(right, y, height, label, font, style, play, action);
            right -= 8.0;
        }
        right
    }

    fn foot_link(&mut self, label: &str, action: Action, big: bool) {
        let height = if big { 44.0 } else { 36.0 };
        let y = Self::foot_y(height) + (height - Font::Body.line()) / 2.0;
        self.link(MARGIN, y, label, action);
    }

    fn message(&mut self, y: f32) -> f32 {
        let Some(message) = self.model.message.clone() else {
            return 0.0;
        };
        self.paragraph(
            MARGIN,
            y,
            WIDTH - 2.0 * MARGIN,
            &message,
            Font::Small,
            color::DANGER,
        ) + 8.0
    }
}

/// A row of a checks card: icon, name, value, and maybe a tag (label, fill, text color).
type Row = (Icon, &'static str, String, Option<(&'static str, u32, u32)>);

#[derive(Clone, Copy)]
enum Icon {
    Ok,
    Get,
    Warn,
}

/// Lays out the window for this model.
pub fn frame(model: &Model, measure: Measure) -> Frame {
    let mut b = Builder {
        frame: Frame::default(),
        model,
        measure,
    };
    b.fill([0.0, 0.0, WIDTH, HEIGHT], 0.0, color::WINDOW);
    match model.page {
        Page::Home => home(&mut b),
        Page::Welcome => welcome(&mut b),
        Page::Advanced => advanced(&mut b),
        Page::Working => working(&mut b),
    }
    b.frame
}

/// The tag beside an engine: recommended, or a warning when it seems missing or only
/// runs in software. Unchecked newer engines get the warning too, as they may well be
/// unsupported; OpenGL 2.1 works almost everywhere.
fn engine_tag(
    engines: &Engines,
    renderer: Renderer,
    recommended: &'static str,
) -> Option<(&'static str, u32, u32)> {
    if renderer == engines.recommended() {
        return Some((recommended, color::OK_SOFT, color::OK));
    }
    match (renderer, engines.support(renderer)) {
        (Renderer::OpenGl21, _) | (_, Some(Support::Yes)) => None,
        (_, Some(Support::Software)) => {
            Some(("Software only, slow", color::WARNING_SOFT, color::WARNING))
        }
        _ => Some(("May be unsupported", color::WARNING_SOFT, color::WARNING)),
    }
}

fn home(b: &mut Builder) {
    let line = format!("Version {}", b.model.installed);
    let mut y = b.head("Mixer", &line);
    y += b.update_status(y);
    let renderer = b.model.settings.renderer;
    let rows = [b.sdl_row(), b.graphics_row(), {
        let tag = engine_tag(&b.model.engines, renderer, "Recommended");
        let warned = tag.is_some_and(|t| t.2 == color::WARNING);
        (
            if warned { Icon::Warn } else { Icon::Ok },
            "Rendering engine",
            renderer.name().to_string(),
            tag,
        )
    }];
    y += b.checks(y, &rows);
    b.message(y);
    b.foot_link("Advanced\u{2026}", Action::Advanced, true);
    b.right_buttons(
        &[("Start Mixer", Style::Primary, true, Action::StartMixer)],
        true,
    );
    b.frame.enter = Some(Action::StartMixer);
    b.frame.escape = Some(Action::Close);
}

fn welcome(b: &mut Builder) {
    let needs = if b.model.mixer_present {
        "One thing is needed before the first start."
    } else {
        "This folder is missing mixer.exe."
    };
    let mut y = b.head("Welcome to Mixer", needs);
    y += b.update_status(y);
    let rows = [b.sdl_row(), b.graphics_row()];
    y += b.checks(y, &rows);
    let width = WIDTH - 2.0 * MARGIN;
    if !b.model.mixer_present {
        y += b.paragraph(
            MARGIN,
            y,
            width,
            "Unzip the whole Mixer download into one folder, then start the launcher from \
             there. Or press Update now, if an update is offered.",
            Font::Small,
            color::WARNING,
        ) + 8.0;
    } else if !b.model.graphics.ok {
        y += b.paragraph(
            MARGIN,
            y,
            width,
            "Mixer needs OpenGL 2.1 from the graphics driver, so it may not start. Update \
             the driver through Windows Update or the maker's website; the launcher cannot \
             install drivers.",
            Font::Small,
            color::WARNING,
        ) + 8.0;
    }
    y += b.paragraph(
        MARGIN,
        y,
        width,
        "Nothing else is installed: no shortcuts, no registry entries. Mixer stays in this \
         folder.",
        Font::Small,
        color::MUTED,
    ) + 8.0;
    b.message(y);
    b.foot_link("Advanced\u{2026}", Action::Advanced, true);
    if b.model.mixer_present {
        b.right_buttons(
            &[
                (
                    "Install and start",
                    Style::Primary,
                    false,
                    Action::InstallAndStart,
                ),
                ("Close", Style::Secondary, false, Action::Close),
            ],
            true,
        );
        b.frame.enter = Some(Action::InstallAndStart);
    } else {
        b.right_buttons(&[("Close", Style::Secondary, false, Action::Close)], true);
    }
    b.frame.escape = Some(Action::Close);
}

fn advanced(b: &mut Builder) {
    let width = WIDTH - 2.0 * MARGIN;
    let mut y = TOP;
    b.text(MARGIN, y, "Rendering engine", Font::Heading, color::TEXT);
    y += Font::Heading.line() + 2.0;
    y += b.paragraph(
        MARGIN,
        y,
        width,
        "How Mixer draws. Every engine can be picked. If one does not start, Mixer falls \
         back to OpenGL 2.1 and tells you.",
        Font::Small,
        color::MUTED,
    ) + 10.0;
    for renderer in Renderer::ALL {
        let row = [MARGIN, y, width, 50.0];
        let selected = b.model.draft.renderer == renderer;
        let action = Action::Pick(renderer);
        if selected {
            b.fill(row, 8.0, color::ACCENT_SOFT);
            b.add(Shape::Outline {
                rect: row,
                radius: 8.0,
                color: color::ACCENT,
                width: 1.0,
            });
        } else if b.hovered(action) {
            b.fill(row, 8.0, color::FRAME);
        }
        let center = [MARGIN + 20.0, y + 17.0];
        if selected {
            b.add(Shape::Circle {
                center,
                radius: 8.0,
                fill: None,
                ring: Some((color::ACCENT, 2.0)),
            });
            b.add(Shape::Circle {
                center,
                radius: 4.0,
                fill: Some(color::ACCENT),
                ring: None,
            });
        } else {
            b.add(Shape::Circle {
                center,
                radius: 8.0,
                fill: None,
                ring: Some((color::FAINT, 2.0)),
            });
        }
        b.text(
            MARGIN + 40.0,
            y + 7.0,
            renderer.name(),
            Font::Strong,
            color::TEXT,
        );
        let name_width = b.width_of(renderer.name(), Font::Strong);
        if let Some((tag, fill, text)) =
            engine_tag(&b.model.engines, renderer, "Recommended for this PC")
        {
            b.tag(MARGIN + 48.0 + name_width, y + 7.0, tag, fill, text);
        }
        b.text(
            MARGIN + 40.0,
            y + 27.0,
            renderer.description(),
            Font::Small,
            color::MUTED,
        );
        b.frame.hits.push(Hit { rect: row, action });
        y += 54.0;
    }
    y += 6.0;
    let checked = b.model.draft.check_updates;
    let box_rect = [MARGIN + 2.0, y + 2.0, 16.0, 16.0];
    if checked {
        b.fill(box_rect, 4.0, color::ACCENT);
        b.add(Shape::Tick {
            rect: [box_rect[0] + 3.0, box_rect[1] + 3.0, 10.0, 10.0],
            color: color::ON_ACCENT,
        });
    } else {
        b.fill(box_rect, 4.0, color::FRAME);
        b.add(Shape::Outline {
            rect: box_rect,
            radius: 4.0,
            color: color::FAINT,
            width: 1.5,
        });
    }
    let label = "Check for updates when the launcher opens";
    b.text(MARGIN + 28.0, y, label, Font::Body, color::TEXT);
    let label_width = b.width_of(label, Font::Body);
    b.frame.hits.push(Hit {
        rect: [MARGIN, y - 4.0, label_width + 32.0, 28.0],
        action: Action::ToggleUpdates,
    });
    y += 30.0;
    let about = format!(
        "Launcher {}  \u{b7}  Mixer {}  \u{b7}  {}",
        Version::own(),
        b.model.installed,
        b.model.folder
    );
    let about = fit(&about, Font::Small, width, b.measure);
    b.text(MARGIN, y, about, Font::Small, color::FAINT);

    b.right_buttons(
        &[
            ("Save", Style::Primary, false, Action::Save),
            ("Back", Style::Secondary, false, Action::Back),
        ],
        false,
    );
    let y = Builder::foot_y(36.0);
    b.button(
        MARGIN,
        y,
        36.0,
        "Repair install",
        Font::Button,
        Style::Secondary,
        false,
        Action::Repair,
    );
    b.frame.enter = Some(Action::Save);
    b.frame.escape = Some(Action::Back);
}

fn working(b: &mut Builder) {
    let job = &b.model.job;
    let width = WIDTH - 2.0 * MARGIN;
    let mut y = TOP;
    b.text(MARGIN, y, job.title.clone(), Font::Heading, color::TEXT);
    y += Font::Heading.line() + 12.0;
    for (i, step) in job.steps.iter().enumerate() {
        let center = [MARGIN + 10.0, y + 10.0];
        let failed_here = job.failed.is_some() && i == job.current;
        let done = i < job.current || (job.finished && job.failed.is_none());
        let (text_color, font) = if done {
            b.add(Shape::Circle {
                center,
                radius: 10.0,
                fill: Some(color::OK_SOFT),
                ring: None,
            });
            b.add(Shape::Tick {
                rect: [center[0] - 5.0, center[1] - 5.0, 10.0, 10.0],
                color: color::OK,
            });
            (color::TEXT, Font::Body)
        } else if failed_here {
            b.add(Shape::Circle {
                center,
                radius: 10.0,
                fill: Some(color::DANGER_SOFT),
                ring: None,
            });
            b.add(Shape::Cross {
                rect: [center[0] - 4.0, center[1] - 4.0, 8.0, 8.0],
                color: color::DANGER,
            });
            (color::TEXT, Font::Body)
        } else if i == job.current {
            b.add(Shape::Circle {
                center,
                radius: 9.0,
                fill: None,
                ring: Some((color::ACCENT, 2.0)),
            });
            (color::TEXT, Font::Body)
        } else {
            b.add(Shape::Circle {
                center,
                radius: 9.0,
                fill: None,
                ring: Some((color::BORDER, 2.0)),
            });
            (color::FAINT, Font::Body)
        };
        b.text(MARGIN + 32.0, y + 1.0, step.clone(), font, text_color);
        y += 30.0;
        if i == job.current
            && job.failed.is_none()
            && !job.finished
            && let Some((fraction, line)) = &job.progress
        {
            let bar = [MARGIN + 32.0, y, width - 32.0, 8.0];
            b.fill(bar, 4.0, 0x12FF_FFFF);
            let filled = (bar[2] * fraction.clamp(0.0, 1.0)).max(8.0);
            b.fill([bar[0], bar[1], filled, bar[3]], 4.0, color::ACCENT);
            b.text(
                MARGIN + 32.0,
                y + 12.0,
                line.clone(),
                Font::Small,
                color::MUTED,
            );
            y += 36.0;
        }
    }
    y += 8.0;
    let foot = Builder::foot_y(36.0);
    if let Some(error) = job.failed.clone() {
        let inner = width - 32.0;
        let lines = wrap(&error, Font::Small, inner, b.measure);
        let lines: Vec<String> = lines.into_iter().take(6).collect();
        let height = 14.0 + Font::Strong.line() + 4.0 + lines.len() as f32 * 19.0 + 12.0;
        let rect = [MARGIN, y, width, height];
        b.fill(rect, 10.0, color::DANGER_SOFT);
        b.add(Shape::Outline {
            rect,
            radius: 10.0,
            color: 0x66F2_6666,
            width: 1.0,
        });
        b.text(
            MARGIN + 16.0,
            y + 14.0,
            "Something went wrong",
            Font::Strong,
            color::TEXT,
        );
        for (i, line) in lines.into_iter().enumerate() {
            b.text(
                MARGIN + 16.0,
                y + 14.0 + Font::Strong.line() + 4.0 + i as f32 * 19.0,
                line,
                Font::Small,
                color::MUTED,
            );
        }
        y += height + 12.0;
    }
    if b.model.show_log && !job.log.is_empty() {
        let bottom = foot - 16.0;
        let height = bottom - y;
        if height >= 40.0 {
            b.fill([MARGIN, y, width, height], 8.0, color::LOG);
            let fits = ((height - 20.0) / 18.0).floor().max(1.0) as usize;
            let start = job.log.len().saturating_sub(fits);
            for (i, line) in job.log[start..].iter().enumerate() {
                let text = fit(line, Font::Mono, width - 24.0, b.measure);
                let tint = if line.starts_with("ERROR") {
                    color::DANGER
                } else {
                    color::LOG_TEXT
                };
                b.text(
                    MARGIN + 12.0,
                    y + 10.0 + i as f32 * 18.0,
                    text,
                    Font::Mono,
                    tint,
                );
            }
        }
    }
    let label = if b.model.show_log {
        "Hide details"
    } else {
        "Show details"
    };
    b.foot_link(label, Action::ToggleLog, false);
    if job.failed.is_some() {
        b.right_buttons(
            &[
                ("Try again", Style::Primary, false, Action::TryAgain),
                ("Back", Style::Secondary, false, Action::Back),
            ],
            false,
        );
        b.frame.enter = Some(Action::TryAgain);
        b.frame.escape = Some(Action::Back);
    } else if job.finished {
        if job.offer_start {
            b.right_buttons(
                &[
                    ("Start Mixer", Style::Primary, true, Action::StartMixer),
                    ("Back", Style::Secondary, false, Action::Back),
                ],
                false,
            );
            b.frame.enter = Some(Action::StartMixer);
        } else {
            b.right_buttons(&[("Back", Style::Primary, false, Action::Back)], false);
            b.frame.enter = Some(Action::Back);
        }
        b.frame.escape = Some(Action::Back);
    } else if job.cancellable {
        b.right_buttons(
            &[("Cancel", Style::Secondary, false, Action::Cancel)],
            false,
        );
        b.frame.escape = Some(Action::Cancel);
    }
}

/// Draws a frame as an SVG picture, for reviewing the layout without Windows.
#[cfg(test)]
pub fn svg(frame: &Frame) -> String {
    let rgba = |c: u32| {
        format!(
            "rgba({},{},{},{:.3})",
            (c >> 16) & 255,
            (c >> 8) & 255,
            c & 255,
            f64::from(c >> 24) / 255.0
        )
    };
    let escape = |t: &str| {
        t.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut out = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{WIDTH}' height='{HEIGHT}' viewBox='0 0 {WIDTH} {HEIGHT}'>"
    );
    for shape in &frame.shapes {
        out.push_str(&match shape {
            Shape::Fill { rect, radius, color } => format!(
                "<rect x='{}' y='{}' width='{}' height='{}' rx='{radius}' fill='{}'/>",
                rect[0], rect[1], rect[2], rect[3], rgba(*color)
            ),
            Shape::Outline { rect, radius, color, width } => format!(
                "<rect x='{}' y='{}' width='{}' height='{}' rx='{radius}' fill='none' stroke='{}' stroke-width='{width}'/>",
                rect[0] + 0.5, rect[1] + 0.5, rect[2] - 1.0, rect[3] - 1.0, rgba(*color)
            ),
            Shape::Text { x, y, text, font, color } => format!(
                "<text x='{x}' y='{}' font-family='{}' font-weight='{}' font-size='{}' fill='{}' xml:space='preserve'>{}</text>",
                y + font.line() / 2.0 + font.size() * 0.36,
                if *font == Font::Mono { "DejaVu Sans Mono" } else { "Inter" },
                if font.semibold() { 600 } else { 400 },
                font.size(),
                rgba(*color),
                escape(text)
            ),
            Shape::Line { from, to, color, width } => format!(
                "<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{}' stroke-width='{width}'/>",
                from[0], from[1], to[0], to[1], rgba(*color)
            ),
            Shape::Circle { center, radius, fill, ring } => format!(
                "<circle cx='{}' cy='{}' r='{radius}' fill='{}' {}/>",
                center[0], center[1],
                fill.map_or("none".to_string(), rgba),
                ring.map_or(String::new(), |(c, w)| format!("stroke='{}' stroke-width='{w}'", rgba(c)))
            ),
            Shape::Tick { rect, color } => {
                let [x, y, w, h] = *rect;
                format!(
                    "<polyline points='{},{} {},{} {},{}' fill='none' stroke='{}' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'/>",
                    x, y + h * 0.55, x + w * 0.38, y + h * 0.9, x + w, y + h * 0.1, rgba(*color)
                )
            }
            Shape::Cross { rect, color } => {
                let [x, y, w, h] = *rect;
                format!(
                    "<path d='M{x},{y} L{},{} M{},{y} L{x},{}' stroke='{}' stroke-width='2' stroke-linecap='round'/>",
                    x + w, y + h, x + w, y + h, rgba(*color)
                )
            }
            Shape::Down { rect, color } => {
                let [x, y, w, h] = *rect;
                format!(
                    "<path d='M{},{y} L{},{} M{x},{} L{},{} L{},{}' fill='none' stroke='{}' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'/>",
                    x + w / 2.0, x + w / 2.0, y + h, y + h * 0.5, x + w / 2.0, y + h, x + w, y + h * 0.5, rgba(*color)
                )
            }
            Shape::Play { rect, color } => {
                let [x, y, w, h] = *rect;
                format!("<polygon points='{x},{y} {},{} {x},{}' fill='{}'/>", x + w, y + h / 2.0, y + h, rgba(*color))
            }
            Shape::Alert { rect, color } => {
                let [x, y, w, h] = *rect;
                format!(
                    "<path d='M{},{y} L{},{}' stroke='{}' stroke-width='2' stroke-linecap='round'/><circle cx='{}' cy='{}' r='1.2' fill='{}'/>",
                    x + w / 2.0, x + w / 2.0, y + h * 0.62, rgba(*color), x + w / 2.0, y + h, rgba(*color)
                )
            }
        });
    }
    out.push_str("</svg>");
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::update::Asset;

    /// Widths from the real Inter font files Mixer ships, so the layout here matches
    /// the window closely. Monospaced text is assumed 0.55 em per character.
    pub fn measure() -> impl Fn(&str, Font) -> f32 {
        let regular = fontdue::Font::from_bytes(
            include_bytes!("../../app/assets/fonts/Inter-Regular.ttf") as &[u8],
            fontdue::FontSettings::default(),
        )
        .unwrap();
        let semibold = fontdue::Font::from_bytes(
            include_bytes!("../../app/assets/fonts/Inter-SemiBold.ttf") as &[u8],
            fontdue::FontSettings::default(),
        )
        .unwrap();
        move |text: &str, font: Font| {
            if font == Font::Mono {
                // Consolas is 0.55 em; a little more leaves room.
                return text.chars().count() as f32 * font.size() * 0.6;
            }
            let face = if font.semibold() { &semibold } else { &regular };
            text.chars()
                .map(|c| face.metrics(c, font.size()).advance_width)
                .sum()
        }
    }

    pub fn model(page: Page) -> Model {
        let release = Release {
            version: Version(0, 11, 0),
            notes: "- Settings page with touchpad mode and auto-save\n- Choose the rendering \
                    engine: Vulkan, DirectX 12 or OpenGL 3.3\n- Fixes: deleting recent projects"
                .to_string(),
            zip: Asset {
                name: "mixer-0.11.0-windows.zip".into(),
                url: "https://github.com/x".into(),
                size: 3_212_544,
            },
            fingerprint: Asset {
                name: "mixer-0.11.0-windows.zip.sha256".into(),
                url: "https://github.com/y".into(),
                size: 99,
            },
        };
        Model {
            page,
            folder: "C:\\Users\\Owner\\Documents\\mixer".into(),
            installed: Version(0, 10, 0),
            mixer_present: true,
            sdl: Found::Installed,
            graphics: Graphics {
                ok: true,
                text: "Intel(R) HD Graphics 3000 \u{b7} OpenGL 3.1".into(),
            },
            update: UpdateState::Available(release),
            settings: LauncherSettings::default(),
            draft: LauncherSettings::default(),
            // The owner's laptop: nothing newer than OpenGL 2.1 on its graphics chip.
            engines: Engines::parse(
                "version 0.13.0\nvulkan no\ndirectx12 software Microsoft Basic Render \
                 Driver\nopengl3.3 no\n",
            ),
            job: JobView {
                title: "Updating to Mixer 0.11.0".into(),
                steps: vec![
                    "Download Mixer 0.11.0 (3.1 MB)".into(),
                    "Check the download's fingerprint".into(),
                    "Replace Mixer (your projects are not touched)".into(),
                    "Start Mixer".into(),
                ],
                current: 0,
                progress: Some((0.62, "1.9 MB of 3.1 MB".into())),
                log: vec![
                    "Newest release: 0.11.0".into(),
                    "Downloading https://github.com/Smogi67/mixer/releases/download/v0.11.0/mixer-0.11.0-windows.zip".into(),
                    "   60%  1.9 MB".into(),
                ],
                failed: None,
                finished: false,
                cancellable: true,
                offer_start: false,
            },
            show_log: true,
            hover: None,
            pressed: None,
            message: None,
        }
    }

    /// Every state worth looking at, named.
    pub fn cases() -> Vec<(&'static str, Model)> {
        let mut cases = vec![
            ("home-update", model(Page::Home)),
            ("advanced", model(Page::Advanced)),
            ("working", model(Page::Working)),
        ];
        let mut welcome = model(Page::Welcome);
        welcome.sdl = Found::Missing;
        welcome.update = UpdateState::UpToDate;
        cases.push(("welcome", welcome));
        let mut checking = model(Page::Home);
        checking.update = UpdateState::Checking;
        checking.hover = Some(Action::StartMixer);
        cases.push(("home-checking", checking));
        let mut offline = model(Page::Home);
        offline.update = UpdateState::Failed(
            "Could not reach api.github.com: the server name could not be found; is the \
             internet connected? (error 12007)"
                .into(),
        );
        offline.graphics = Graphics {
            ok: false,
            text: "Too old: OpenGL 1.1.0 on GDI Generic. Update the graphics driver.".into(),
        };
        offline.message = Some("Could not start Mixer: the file is missing.".into());
        cases.push(("home-offline", offline));
        let mut failed = model(Page::Working);
        failed.job.current = 2;
        failed.job.failed = Some(
            "Could not replace mixer.exe: Access is denied. (os error 5). If Mixer is open, \
             close it and press Try again. Nothing was changed."
                .into(),
        );
        failed
            .job
            .log
            .push("ERROR: Could not replace mixer.exe".into());
        cases.push(("working-failed", failed));
        let mut done = model(Page::Working);
        done.job.title = "Repairing the install".into();
        done.job.steps = vec![
            "Find Mixer 0.10.0 on GitHub".into(),
            "Download Mixer 0.10.0".into(),
            "Check the download's fingerprint".into(),
            "Put Mixer's files back (your projects are not touched)".into(),
            "Download SDL2 2.32.10 (595 KB)".into(),
            "Check and install SDL2".into(),
            "Check that Windows can load it".into(),
        ];
        done.job.current = 7;
        done.job.finished = true;
        done.job.offer_start = true;
        done.show_log = false;
        cases.push(("working-done", done));
        let mut broken = model(Page::Welcome);
        broken.mixer_present = false;
        broken.sdl = Found::Different;
        cases.push(("welcome-broken", broken));
        cases
    }

    #[test]
    fn everything_fits_inside_the_window_and_nothing_overlaps_the_buttons() {
        let measure = measure();
        for (name, model) in cases() {
            let frame = frame(&model, &measure);
            for shape in &frame.shapes {
                let (x, y, right, bottom) = match shape {
                    Shape::Fill { rect, .. } | Shape::Outline { rect, .. } => {
                        (rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3])
                    }
                    Shape::Text {
                        x, y, text, font, ..
                    } => (*x, *y, x + measure(text, *font), y + font.line()),
                    _ => continue,
                };
                assert!(
                    x >= 0.0 && y >= 0.0 && right <= WIDTH + 0.5 && bottom <= HEIGHT + 0.5,
                    "{name}: {shape:?} leaves the window"
                );
                // Everything but the background stays inside the margins.
                if right - x < WIDTH {
                    assert!(
                        x >= MARGIN - 0.5 && right <= WIDTH - MARGIN + 0.5,
                        "{name}: {shape:?} crosses the margin"
                    );
                }
            }
            // Clickable areas do not overlap each other.
            for (i, a) in frame.hits.iter().enumerate() {
                for b in &frame.hits[i + 1..] {
                    let apart = a.rect[0] + a.rect[2] <= b.rect[0]
                        || b.rect[0] + b.rect[2] <= a.rect[0]
                        || a.rect[1] + a.rect[3] <= b.rect[1]
                        || b.rect[1] + b.rect[3] <= a.rect[1];
                    assert!(apart, "{name}: {a:?} overlaps {b:?}");
                }
            }
            // Text above the bottom row stays clear of it.
            let foot_top = frame
                .hits
                .iter()
                .filter(|h| h.rect[1] > HEIGHT - 100.0)
                .map(|h| h.rect[1])
                .fold(HEIGHT, f32::min);
            for shape in &frame.shapes {
                if let Shape::Text { y, font, text, .. } = shape
                    && *y < foot_top - 1.0
                {
                    assert!(
                        y + font.line() <= foot_top + 0.5,
                        "{name}: {text:?} runs into the bottom row"
                    );
                }
            }
        }
    }

    #[test]
    fn each_page_has_its_buttons_and_keys() {
        let measure = measure();
        let actions = |model: &Model| -> Vec<Action> {
            frame(model, &measure)
                .hits
                .iter()
                .map(|h| h.action)
                .collect()
        };
        let home = model(Page::Home);
        let found = actions(&home);
        for needed in [
            Action::StartMixer,
            Action::UpdateNow,
            Action::NotNow,
            Action::Advanced,
        ] {
            assert!(found.contains(&needed), "home lacks {needed:?}");
        }
        let advanced = actions(&model(Page::Advanced));
        // Every engine can be picked, even one that seems missing (owner's choice).
        for renderer in Renderer::ALL {
            assert!(advanced.contains(&Action::Pick(renderer)), "{renderer:?}");
        }
        for needed in [
            Action::Save,
            Action::Back,
            Action::Repair,
            Action::ToggleUpdates,
        ] {
            assert!(advanced.contains(&needed), "advanced lacks {needed:?}");
        }
        let f = frame(&model(Page::Working), &measure);
        assert_eq!(f.escape, Some(Action::Cancel));
        assert_eq!(frame(&home, &measure).enter, Some(Action::StartMixer));
        // A click lands on what is under it.
        let start = f
            .hits
            .iter()
            .find(|h| h.action == Action::Cancel)
            .unwrap()
            .rect;
        assert_eq!(
            f.hit([start[0] + 2.0, start[1] + 2.0]),
            Some(Action::Cancel)
        );
        assert_eq!(f.hit([1.0, 1.0]), None);
    }

    #[test]
    fn long_text_wraps_and_shortens() {
        let measure = measure();
        let lines = wrap(
            "a long sentence that should wrap onto more than one line here",
            Font::Body,
            150.0,
            &measure,
        );
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| measure(l, Font::Body) <= 150.0));
        let long_word = wrap(&"x".repeat(80), Font::Body, 100.0, &measure);
        assert!(long_word.len() > 1);
        assert!(long_word.iter().all(|l| measure(l, Font::Body) <= 100.0));
        let short = fit(&"y".repeat(200), Font::Small, 120.0, &measure);
        assert!(short.ends_with('\u{2026}') && measure(&short, Font::Small) <= 120.0);
    }

    /// Writes each case as an SVG for review, when LAUNCHER_PREVIEW names a folder.
    #[test]
    fn preview() {
        let Some(folder) = std::env::var_os("LAUNCHER_PREVIEW") else {
            return;
        };
        let measure = measure();
        for (name, model) in cases() {
            let picture = svg(&frame(&model, &measure));
            std::fs::write(
                std::path::Path::new(&folder).join(format!("{name}.svg")),
                picture,
            )
            .unwrap();
        }
    }
}
