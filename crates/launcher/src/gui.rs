//! The launcher window. It draws itself with GDI+, which is part of Windows, so the
//! launcher still needs nothing it would have to install first (spec section 6). What
//! it shows is laid out in `view.rs`; this file draws it, handles the mouse and keys,
//! and runs jobs on a second thread that wakes the window for each step.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{FreeLibrary, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EndPaint, GetDC, GetDeviceCaps, InvalidateRect, LOGPIXELSX, PAINTSTRUCT, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    DashCapFlat, FillModeAlternate, FontStyleBold, FontStyleRegular, GdipAddPathArc,
    GdipCloneStringFormat, GdipClosePathFigure, GdipCreateBitmapFromScan0, GdipCreateFont,
    GdipCreateFontFamilyFromName, GdipCreateFromHDC, GdipCreateHICONFromBitmap, GdipCreatePath,
    GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath,
    GdipDeletePen, GdipDisposeImage, GdipDrawEllipse, GdipDrawLine, GdipDrawLines, GdipDrawPath,
    GdipDrawString, GdipFillEllipse, GdipFillPath, GdipFillPolygon, GdipFillRectangle,
    GdipGetImageGraphicsContext, GdipGraphicsClear, GdipMeasureString,
    GdipNewPrivateFontCollection, GdipPrivateAddMemoryFont, GdipSetPenLineCap197819,
    GdipSetPenLineJoin, GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdipSetStringFormatFlags,
    GdipSetTextRenderingHint, GdipStringFormatGetGenericTypographic, GdiplusShutdown,
    GdiplusStartup, GdiplusStartupInput, GpFont, GpFontCollection, GpFontFamily, GpGraphics,
    GpStringFormat, LineCapRound, LineJoinRound, PixelOffsetModeHalf, PointF, RectF,
    SmoothingModeAntiAlias, StringFormatFlagsMeasureTrailingSpaces, StringFormatFlagsNoWrap,
    TextRenderingHintClearTypeGridFit, UnitPixel,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect,
    GetMessageW, GetSystemMetrics, HICON, HTCLIENT, ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_HAND,
    LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SW_SHOW, SendMessageW, SetCursor, SetProcessDPIAware, ShowWindow, TranslateMessage, WM_APP,
    WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WM_SETCURSOR, WM_SETICON, WNDCLASSW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED,
    WS_SYSMENU,
};

use crate::checks;
use crate::core::{self, Event, NEEDED_OPENGL, SDL2};
use crate::jobs::{self, Io, Plan};
use crate::net;
use crate::settings::{self, LauncherSettings};
use crate::update::{self, APP_EXE, Release, Version};
use crate::view::{self, Action, Font, Frame, Graphics, JobView, Model, Page, Shape, UpdateState};

const WAKE: u32 = WM_APP + 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;
const INTER_REGULAR: &[u8] = include_bytes!("../../app/assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../app/assets/fonts/Inter-SemiBold.ttf");

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// What the work threads have to tell the window. It reads this when woken.
#[derive(Default)]
struct Shared {
    events: Vec<Event>,
    finished: Option<Result<(), String>>,
    /// The update check: a newer release, none, or why it failed.
    check: Option<Result<Option<Release>, String>>,
}

/// Command-line options, for automated screenshots and checks.
#[derive(Default, Clone)]
struct Options {
    page: Option<String>,
    show_update: bool,
    press_continue: bool,
    update_now: bool,
    pretend_version: Option<Version>,
    /// Passed on to Mixer, such as a file to open.
    for_mixer: Vec<String>,
}

fn options() -> Options {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--page" => options.page = args.next(),
            "--show-update" => options.show_update = true,
            "--press-continue" => options.press_continue = true,
            "--update-now" => options.update_now = true,
            "--pretend-version" => {
                options.pretend_version = args.next().and_then(|v| Version::parse(&v));
            }
            "--no-window" | "--update" | "--repair" => {}
            _ => options.for_mixer.push(arg),
        }
    }
    options
}

fn own_folder() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The user's Documents folder.
fn documents() -> Option<PathBuf> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};
    let mut path: *mut u16 = null_mut();
    // SAFETY: Windows writes a pointer to a zero-ended string it allocated, which is
    // read up to the zero and then freed with the function it names for that.
    unsafe {
        let result = SHGetKnownFolderPath(&FOLDERID_Documents, 0, null_mut(), &mut path);
        if result != 0 || path.is_null() {
            if !path.is_null() {
                CoTaskMemFree(path as *const _);
            }
            return None;
        }
        let mut length = 0;
        while *path.add(length) != 0 {
            length += 1;
        }
        let folder = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(
            path, length,
        )));
        CoTaskMemFree(path as *const _);
        Some(folder)
    }
}

/// The graphics check, in words.
fn graphics() -> Graphics {
    match checks::opengl() {
        Ok(gl) => match core::opengl_version(&gl.version) {
            Some(v) if v >= NEEDED_OPENGL => Graphics {
                ok: true,
                text: format!("{} \u{b7} OpenGL {}.{}", gl.renderer.trim(), v.0, v.1),
            },
            _ => Graphics {
                ok: false,
                text: format!(
                    "Too old: OpenGL {} on {}. Update the graphics driver.",
                    gl.version.trim(),
                    gl.renderer.trim()
                ),
            },
        },
        Err(reason) => Graphics {
            ok: false,
            text: format!("{reason} Update the graphics driver."),
        },
    }
}

/// Asks GitHub for the newest release; `Some` if it is newer than `installed`.
fn check_for_update(installed: Version) -> Result<Option<Release>, String> {
    let url = format!("https://{}{}", update::API_HOST, update::LATEST_PATH);
    let answer = match net::get(&url, Some("application/vnd.github+json"), &mut |_, _| true) {
        Ok(answer) => answer,
        // No release published yet.
        Err(error) if error.contains("code 404") => return Ok(None),
        Err(error) => return Err(error),
    };
    let release = update::parse_release(&String::from_utf8_lossy(&answer))?;
    Ok((release.version > installed).then_some(release))
}

/// The system, for jobs.
struct WindowsIo {
    for_mixer: Vec<String>,
}

impl Io for WindowsIo {
    fn get(
        &mut self,
        url: &str,
        progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<Vec<u8>, String> {
        net::get(url, None, progress)
    }

    fn loads(&mut self, dll: &Path) -> bool {
        let path = wide(&dll.to_string_lossy());
        // SAFETY: the path ends in a zero; the library is freed straight away.
        unsafe {
            let module = LoadLibraryW(path.as_ptr());
            let ok = !module.is_null();
            if ok {
                FreeLibrary(module);
            }
            ok
        }
    }

    fn start_mixer(&mut self, folder: &Path) -> Result<(), String> {
        start_mixer(folder, &self.for_mixer)
    }
}

fn start_mixer(folder: &Path, args: &[String]) -> Result<(), String> {
    std::process::Command::new(folder.join(APP_EXE))
        .args(args)
        .current_dir(folder)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not start Mixer: {error}"))
}

/// Runs without a window, printing each step, for automated checks on GitHub. With
/// `--update`, updates Mixer from the newest release; with `--repair`, puts every file
/// back; otherwise installs SDL2 if it is needed. Returns the exit code.
pub fn run_without_window() -> i32 {
    let folder = own_folder();
    let options = options();
    let updating = std::env::args().any(|a| a == "--update");
    let repairing = std::env::args().any(|a| a == "--repair");
    println!("Graphics: {}", graphics().text);
    let plan = if repairing {
        Plan::repair(update::installed_version(&folder))
    } else if updating {
        let installed = options
            .pretend_version
            .unwrap_or_else(|| update::installed_version(&folder));
        println!("Installed: Mixer {installed}");
        match check_for_update(installed) {
            Ok(Some(release)) => {
                println!("Newer release: {}", release.version);
                Plan::update(release, core::find(&SDL2, &folder), false)
            }
            Ok(None) => {
                println!("Mixer is up to date.");
                return 0;
            }
            Err(error) => {
                println!("ERROR: {error}");
                return 1;
            }
        }
    } else {
        let mut plan = Plan::install();
        plan.tasks = vec![jobs::Task::EnsureSdl];
        plan
    };
    let mut io = WindowsIo {
        for_mixer: Vec::new(),
    };
    let mut print = |event: Event| match event {
        Event::Line(line) => println!("{line}"),
        Event::Step(step) => {
            if let Some(name) = plan.steps.get(step) {
                println!("== {name}");
            }
        }
        Event::Progress(..) => {}
    };
    match jobs::run(&plan, &folder, &mut io, &mut print, &AtomicBool::new(false)) {
        Ok(()) => {
            println!("Done.");
            0
        }
        Err(error) => {
            println!("ERROR: {error}");
            1
        }
    }
}

/// GDI+ and the fonts, made once.
struct Paint {
    families: [*mut GpFontFamily; 3],
    fonts: HashMap<Font, *mut GpFont>,
    format: *mut GpStringFormat,
    /// For measuring text outside painting.
    screen: *mut GpGraphics,
    scale: f32,
    /// Keeps the font data alive while GDI+ uses it.
    _collection: *mut GpFontCollection,
}

impl Paint {
    /// # Safety
    /// GDI+ must be started.
    unsafe fn new(scale: f32) -> Self {
        // SAFETY: GDI+ is started; every out-pointer is a local, and the font data is
        // static, as GDI+ needs it to stay.
        unsafe {
            let mut collection = null_mut();
            GdipNewPrivateFontCollection(&mut collection);
            for data in [INTER_REGULAR, INTER_SEMIBOLD] {
                GdipPrivateAddMemoryFont(collection, data.as_ptr().cast(), data.len() as i32);
            }
            let family = |name: &str, from: *mut GpFontCollection| {
                let mut family = null_mut();
                let name = wide(name);
                GdipCreateFontFamilyFromName(name.as_ptr(), from, &mut family);
                family
            };
            let mut regular = family("Inter", collection);
            let mut semibold = family("Inter SemiBold", collection);
            // Without the fonts, Windows' own.
            if regular.is_null() {
                regular = family("Segoe UI", null_mut());
            }
            if semibold.is_null() {
                semibold = family("Segoe UI Semibold", null_mut());
            }
            if semibold.is_null() {
                semibold = regular;
            }
            let mut mono = family("Consolas", null_mut());
            if mono.is_null() {
                mono = family("Courier New", null_mut());
            }
            let mut generic = null_mut();
            GdipStringFormatGetGenericTypographic(&mut generic);
            let mut format = null_mut();
            GdipCloneStringFormat(generic, &mut format);
            GdipSetStringFormatFlags(
                format,
                StringFormatFlagsMeasureTrailingSpaces | StringFormatFlagsNoWrap,
            );
            let screen_dc = GetDC(null_mut());
            let mut screen = null_mut();
            GdipCreateFromHDC(screen_dc, &mut screen);
            GdipSetTextRenderingHint(screen, TextRenderingHintClearTypeGridFit);
            let mut paint = Self {
                families: [regular, semibold, mono],
                fonts: HashMap::new(),
                format,
                screen,
                scale,
                _collection: collection,
            };
            for font in [
                Font::Body,
                Font::Small,
                Font::Strong,
                Font::Heading,
                Font::Title,
                Font::Button,
                Font::BigButton,
                Font::Mono,
            ] {
                let family = if font == Font::Mono {
                    paint.families[2]
                } else if font.semibold() {
                    paint.families[1]
                } else {
                    paint.families[0]
                };
                // A stand-in family without a semibold face gets bold instead.
                let style = if font.semibold() && family == paint.families[0] {
                    FontStyleBold
                } else {
                    FontStyleRegular
                };
                let mut made = null_mut();
                GdipCreateFont(family, font.size() * scale, style, UnitPixel, &mut made);
                paint.fonts.insert(font, made);
            }
            paint
        }
    }

    /// A line's width, in unscaled pixels.
    fn measure(&self, text: &str, font: Font) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let text = utf16(text);
        let layout = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 100_000.0,
            Height: 1000.0,
        };
        let mut bounds = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 0.0,
            Height: 0.0,
        };
        // SAFETY: all handles were made in `new`; the text and rectangles are locals.
        unsafe {
            GdipMeasureString(
                self.screen,
                text.as_ptr(),
                text.len() as i32,
                self.fonts[&font],
                &layout,
                self.format,
                &mut bounds,
                null_mut(),
                null_mut(),
            );
        }
        bounds.Width / self.scale
    }

    /// Draws a frame onto a GDI+ surface.
    ///
    /// # Safety
    /// `g` must be a live GDI+ surface.
    unsafe fn draw(&self, g: *mut GpGraphics, frame: &Frame) {
        let s = self.scale;
        // SAFETY: the caller's promise; every brush, pen and path made here is deleted
        // here, and the text and points are locals.
        unsafe {
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
            GdipSetTextRenderingHint(g, TextRenderingHintClearTypeGridFit);
            let brush = |color: u32| {
                let mut b = null_mut();
                GdipCreateSolidFill(color, &mut b);
                b
            };
            let pen = |color: u32, width: f32| {
                let mut p = null_mut();
                GdipCreatePen1(color, width * s, UnitPixel, &mut p);
                GdipSetPenLineCap197819(p, LineCapRound, LineCapRound, DashCapFlat);
                GdipSetPenLineJoin(p, LineJoinRound);
                p
            };
            let rounded = |rect: [f32; 4], radius: f32| {
                let [x, y, w, h] = rect.map(|v| v * s);
                let r = (radius * s).min(w / 2.0).min(h / 2.0);
                let mut path = null_mut();
                GdipCreatePath(FillModeAlternate, &mut path);
                let d = r * 2.0;
                GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
                GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
                GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
                GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
                GdipClosePathFigure(path);
                path
            };
            let lines = |points: &[[f32; 2]], color: u32| {
                let points: Vec<PointF> = points
                    .iter()
                    .map(|p| PointF {
                        X: p[0] * s,
                        Y: p[1] * s,
                    })
                    .collect();
                let p = pen(color, 2.0);
                GdipDrawLines(g, p, points.as_ptr(), points.len() as i32);
                GdipDeletePen(p);
            };
            for shape in &frame.shapes {
                match shape {
                    Shape::Fill {
                        rect,
                        radius,
                        color,
                    } => {
                        let b = brush(*color);
                        if *radius <= 0.0 {
                            GdipFillRectangle(
                                g,
                                b.cast(),
                                rect[0] * s,
                                rect[1] * s,
                                rect[2] * s,
                                rect[3] * s,
                            );
                        } else {
                            let path = rounded(*rect, *radius);
                            GdipFillPath(g, b.cast(), path);
                            GdipDeletePath(path);
                        }
                        GdipDeleteBrush(b.cast());
                    }
                    Shape::Outline {
                        rect,
                        radius,
                        color,
                        width,
                    } => {
                        let inset = width / 2.0;
                        let path = rounded(
                            [
                                rect[0] + inset,
                                rect[1] + inset,
                                rect[2] - 2.0 * inset,
                                rect[3] - 2.0 * inset,
                            ],
                            *radius,
                        );
                        let p = pen(*color, *width);
                        GdipDrawPath(g, p, path);
                        GdipDeletePen(p);
                        GdipDeletePath(path);
                    }
                    Shape::Text {
                        x,
                        y,
                        text,
                        font,
                        color,
                    } => {
                        let b = brush(*color);
                        let text = utf16(text);
                        // The glyphs sit in the middle of the line box.
                        let top = y + (font.line() - font.size() * 1.21) / 2.0;
                        let layout = RectF {
                            X: x * s,
                            Y: top * s,
                            Width: 100_000.0,
                            Height: font.line() * s * 2.0,
                        };
                        GdipDrawString(
                            g,
                            text.as_ptr(),
                            text.len() as i32,
                            self.fonts[font],
                            &layout,
                            self.format,
                            b.cast(),
                        );
                        GdipDeleteBrush(b.cast());
                    }
                    Shape::Line {
                        from,
                        to,
                        color,
                        width,
                    } => {
                        let p = pen(*color, *width);
                        GdipDrawLine(g, p, from[0] * s, from[1] * s, to[0] * s, to[1] * s);
                        GdipDeletePen(p);
                    }
                    Shape::Circle {
                        center,
                        radius,
                        fill,
                        ring,
                    } => {
                        let [cx, cy] = center.map(|v| v * s);
                        let r = radius * s;
                        if let Some(color) = fill {
                            let b = brush(*color);
                            GdipFillEllipse(g, b.cast(), cx - r, cy - r, 2.0 * r, 2.0 * r);
                            GdipDeleteBrush(b.cast());
                        }
                        if let Some((color, width)) = ring {
                            let p = pen(*color, *width);
                            let r = r - width * s / 2.0;
                            GdipDrawEllipse(g, p, cx - r, cy - r, 2.0 * r, 2.0 * r);
                            GdipDeletePen(p);
                        }
                    }
                    Shape::Tick { rect, color } => {
                        let [x, y, w, h] = *rect;
                        lines(
                            &[
                                [x, y + h * 0.55],
                                [x + w * 0.38, y + h * 0.9],
                                [x + w, y + h * 0.1],
                            ],
                            *color,
                        );
                    }
                    Shape::Cross { rect, color } => {
                        let [x, y, w, h] = *rect;
                        lines(&[[x, y], [x + w, y + h]], *color);
                        lines(&[[x + w, y], [x, y + h]], *color);
                    }
                    Shape::Down { rect, color } => {
                        let [x, y, w, h] = *rect;
                        lines(&[[x + w / 2.0, y], [x + w / 2.0, y + h]], *color);
                        lines(
                            &[[x, y + h * 0.5], [x + w / 2.0, y + h], [x + w, y + h * 0.5]],
                            *color,
                        );
                    }
                    Shape::Play { rect, color } => {
                        let [x, y, w, h] = rect.map(|v| v * s);
                        let points = [
                            PointF { X: x, Y: y },
                            PointF {
                                X: x + w,
                                Y: y + h / 2.0,
                            },
                            PointF { X: x, Y: y + h },
                        ];
                        let b = brush(*color);
                        GdipFillPolygon(g, b.cast(), points.as_ptr(), 3, FillModeAlternate);
                        GdipDeleteBrush(b.cast());
                    }
                    Shape::Alert { rect, color } => {
                        let [x, y, w, h] = *rect;
                        lines(&[[x + w / 2.0, y], [x + w / 2.0, y + h * 0.6]], *color);
                        let b = brush(*color);
                        let r = 1.3 * s;
                        GdipFillEllipse(
                            g,
                            b.cast(),
                            (x + w / 2.0) * s - r,
                            (y + h) * s - r,
                            2.0 * r,
                            2.0 * r,
                        );
                        GdipDeleteBrush(b.cast());
                    }
                }
            }
        }
    }
}

/// The window's state, for the window procedure.
struct App {
    window: HWND,
    model: Model,
    frame: Frame,
    paint: Paint,
    folder: PathBuf,
    settings_file: PathBuf,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    plan: Option<Plan>,
    options: Options,
    tracking: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// A release to show the update card with, for screenshots, without asking GitHub.
fn demo_release() -> Release {
    let asset = |name: &str, size| update::Asset {
        name: name.to_string(),
        url: format!("https://github.com/Smogi67/mixer/releases/download/v9.9.9/{name}"),
        size,
    };
    Release {
        version: Version(9, 9, 9),
        notes: "- Settings page with touchpad mode and auto-save\n- Choose the rendering engine\n\
                - Fixes: deleting recent projects"
            .into(),
        zip: asset("mixer-9.9.9-windows.zip", 3_212_544),
        fingerprint: asset("mixer-9.9.9-windows.zip.sha256", 99),
    }
}

/// A small orange rounded square, as the window's icon.
///
/// # Safety
/// GDI+ must be started.
unsafe fn make_icon(size: i32) -> HICON {
    // SAFETY: the bitmap and its surface are made, used and released here.
    unsafe {
        let mut bitmap = null_mut();
        GdipCreateBitmapFromScan0(size, size, 0, PIXEL_FORMAT_32BPP_ARGB, null(), &mut bitmap);
        if bitmap.is_null() {
            return null_mut();
        }
        let mut g = null_mut();
        GdipGetImageGraphicsContext(bitmap.cast(), &mut g);
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipGraphicsClear(g, 0);
        let frame = Frame {
            shapes: vec![Shape::Fill {
                rect: [1.0, 1.0, size as f32 - 2.0, size as f32 - 2.0],
                radius: size as f32 * 0.27,
                color: view::color::ACCENT,
            }],
            ..Frame::default()
        };
        // Drawn without scaling.
        let plain = Paint {
            families: [null_mut(); 3],
            fonts: HashMap::new(),
            format: null_mut(),
            screen: null_mut(),
            scale: 1.0,
            _collection: null_mut(),
        };
        plain.draw(g, &frame);
        GdipDeleteGraphics(g);
        let mut icon: HICON = null_mut();
        GdipCreateHICONFromBitmap(bitmap, &mut icon);
        GdipDisposeImage(bitmap.cast());
        icon
    }
}

/// Shows the launcher window and runs until it closes.
pub fn run() {
    let folder = own_folder();
    update::clean_up_after_update(&folder);
    let options = options();
    let settings_file = settings::settings_file(documents());
    let saved = LauncherSettings::load(&settings_file);
    let installed = options
        .pretend_version
        .unwrap_or_else(|| update::installed_version(&folder));
    let mut model = Model {
        page: Page::Home,
        folder: folder.display().to_string(),
        installed,
        mixer_present: folder.join(APP_EXE).exists(),
        sdl: core::find(&SDL2, &folder),
        graphics: graphics(),
        update: if saved.check_updates {
            UpdateState::Checking
        } else {
            UpdateState::Off
        },
        settings: saved,
        draft: saved,
        job: JobView::default(),
        show_log: false,
        hover: None,
        pressed: None,
        message: None,
    };
    model.page = model.resting_page();
    if options.show_update {
        model.update = UpdateState::Available(demo_release());
    }
    if options.page.as_deref() == Some("advanced") {
        model.page = Page::Advanced;
    }

    // SAFETY: standard Win32 and GDI+ setup on this thread. Every string passed stays
    // alive for its call, and every handle is used only on this thread.
    unsafe {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 0,
        };
        let mut token = 0usize;
        if GdiplusStartup(&mut token, &input, null_mut()) != 0 {
            return;
        }
        SetProcessDPIAware();
        let screen = GetDC(null_mut());
        let dpi = GetDeviceCaps(screen, LOGPIXELSX as i32).max(96);
        ReleaseDC(null_mut(), screen);
        let scale = dpi as f32 / 96.0;
        let paint = Paint::new(scale);

        let instance = GetModuleHandleW(null());
        let class = wide("MixerLauncher");
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.lpfnWndProc = Some(window_procedure);
        window_class.hInstance = instance;
        window_class.lpszClassName = class.as_ptr();
        window_class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        RegisterClassW(&window_class);

        let (width, height) = (
            (view::WIDTH * scale).round() as i32,
            (view::HEIGHT * scale).round() as i32,
        );
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut outer = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        AdjustWindowRectEx(&mut outer, style, 0, 0);
        let (outer_w, outer_h) = (outer.right - outer.left, outer.bottom - outer.top);
        let title = wide("Mixer Launcher");
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            ((GetSystemMetrics(SM_CXSCREEN) - outer_w) / 2).max(0),
            ((GetSystemMetrics(SM_CYSCREEN) - outer_h) / 2).max(0),
            outer_w,
            outer_h,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return;
        }
        // A dark title bar, on Windows 10 from 2020 and later (20), or a little earlier
        // (19). Older versions keep the light one.
        let dark: i32 = 1;
        for attribute in [20u32, 19] {
            let size = std::mem::size_of::<i32>() as u32;
            if DwmSetWindowAttribute(window, attribute, (&dark as *const i32).cast(), size) == 0 {
                break;
            }
        }
        for (which, size) in [(ICON_BIG, 32), (ICON_SMALL, 16)] {
            let icon = make_icon((size as f32 * scale).round() as i32);
            if !icon.is_null() {
                SendMessageW(window, WM_SETICON, which as WPARAM, icon as LPARAM);
            }
        }

        let check = model.update == UpdateState::Checking;
        APP.with(|app| {
            *app.borrow_mut() = Some(App {
                window,
                model,
                frame: Frame::default(),
                paint,
                folder,
                settings_file,
                shared: Arc::new(Mutex::new(Shared::default())),
                cancel: Arc::new(AtomicBool::new(false)),
                plan: None,
                options: options.clone(),
                tracking: false,
            })
        });
        if check {
            start_check();
        }
        ShowWindow(window, SW_SHOW);
        if options.press_continue {
            PostMessageW(window, WAKE, 1, 0);
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        APP.with(|app| app.borrow_mut().take());
        GdiplusShutdown(token);
    }
}

/// Asks GitHub for a newer release on a second thread.
fn start_check() {
    APP.with(|app| {
        let Ok(mut app) = app.try_borrow_mut() else {
            return;
        };
        let Some(app) = app.as_mut() else {
            return;
        };
        app.model.update = UpdateState::Checking;
        let shared = Arc::clone(&app.shared);
        let installed = app.model.installed;
        let window = app.window as isize;
        std::thread::spawn(move || {
            let result = check_for_update(installed);
            shared.lock().unwrap_or_else(|e| e.into_inner()).check = Some(result);
            // SAFETY: posting to a window from another thread is allowed; if the window
            // is gone, the post fails harmlessly.
            unsafe { PostMessageW(window as HWND, WAKE, 0, 0) };
        });
    });
}

impl App {
    fn invalidate(&self) {
        // SAFETY: the window belongs to this thread.
        unsafe { InvalidateRect(self.window, null(), 0) };
    }

    fn close(&self) {
        // SAFETY: as above; WM_CLOSE is handled once this call returns.
        unsafe { PostMessageW(self.window, WM_CLOSE, 0, 0) };
    }

    /// Looks at the folder again, after a job or on going back.
    fn refresh(&mut self) {
        self.model.sdl = core::find(&SDL2, &self.folder);
        self.model.mixer_present = self.folder.join(APP_EXE).exists();
        if self.options.pretend_version.is_none() {
            self.model.installed = update::installed_version(&self.folder);
        }
    }

    fn start_job(&mut self, plan: Plan) {
        self.model.page = Page::Working;
        self.model.message = None;
        self.model.job = JobView {
            title: plan.title.clone(),
            steps: plan.steps.clone(),
            cancellable: plan.cancellable_until > 0,
            ..JobView::default()
        };
        *self.shared.lock().unwrap_or_else(|e| e.into_inner()) = Shared {
            check: None,
            ..Shared::default()
        };
        self.cancel.store(false, Ordering::Relaxed);
        self.plan = Some(plan.clone());
        let shared = Arc::clone(&self.shared);
        let cancel = Arc::clone(&self.cancel);
        let folder = self.folder.clone();
        let window = self.window as isize;
        let for_mixer = self.options.for_mixer.clone();
        std::thread::spawn(move || {
            let wake = || {
                // SAFETY: as in `start_check`.
                unsafe { PostMessageW(window as HWND, WAKE, 0, 0) };
            };
            let mut io = WindowsIo { for_mixer };
            let mut send = |event: Event| {
                shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .events
                    .push(event);
                wake();
            };
            let result = jobs::run(&plan, &folder, &mut io, &mut send, &cancel);
            shared.lock().unwrap_or_else(|e| e.into_inner()).finished = Some(result);
            wake();
        });
        self.invalidate();
    }

    /// Takes in what the work threads sent.
    fn wake(&mut self) {
        let (events, finished, check) = {
            let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            (
                std::mem::take(&mut shared.events),
                shared.finished.take(),
                shared.check.take(),
            )
        };
        if let Some(check) = check {
            self.model.update = match check {
                Ok(Some(release)) => UpdateState::Available(release),
                Ok(None) => UpdateState::UpToDate,
                Err(error) => UpdateState::Failed(error),
            };
            if self.options.update_now
                && let UpdateState::Available(release) = &self.model.update
            {
                let plan = Plan::update(release.clone(), self.model.sdl, true);
                self.start_job(plan);
            }
        }
        let job = &mut self.model.job;
        for event in events {
            match event {
                Event::Step(step) => {
                    job.current = step;
                    job.progress = None;
                }
                Event::Line(line) => {
                    job.log.push(line);
                    if job.log.len() > 500 {
                        job.log.drain(..100);
                    }
                }
                Event::Progress(fraction, text) => job.progress = Some((fraction, text)),
            }
        }
        if let Some(plan) = &self.plan {
            job.cancellable = job.current < plan.cancellable_until;
        }
        if let Some(result) = finished {
            match result {
                Ok(()) => {
                    job.finished = true;
                    job.cancellable = false;
                    let starts = self.plan.as_ref().is_some_and(Plan::starts_mixer);
                    self.refresh();
                    if matches!(self.model.update, UpdateState::Available(_))
                        && self
                            .plan
                            .as_ref()
                            .is_some_and(|p| p.title.starts_with("Updating"))
                    {
                        self.model.update = UpdateState::UpToDate;
                    }
                    self.model.job.offer_start = self.model.mixer_present;
                    if starts {
                        self.close();
                    }
                }
                Err(error) if error == jobs::CANCELLED => {
                    self.refresh();
                    self.model.page = self.model.resting_page();
                }
                Err(error) => {
                    job.failed = Some(error.clone());
                    job.cancellable = false;
                    job.log.push(format!("ERROR: {error}"));
                }
            }
        }
        self.invalidate();
    }

    fn perform(&mut self, action: Action) {
        match action {
            Action::StartMixer => match start_mixer(&self.folder, &self.options.for_mixer) {
                Ok(()) => self.close(),
                Err(error) => {
                    self.model.page = self.model.resting_page();
                    self.model.message = Some(error);
                }
            },
            Action::UpdateNow => {
                if let UpdateState::Available(release) = self.model.update.clone() {
                    self.start_job(Plan::update(release, self.model.sdl, true));
                }
            }
            Action::NotNow => self.model.update = UpdateState::Dismissed,
            Action::Advanced => {
                self.model.draft = self.model.settings;
                self.model.page = Page::Advanced;
            }
            Action::Back => {
                self.refresh();
                self.model.page = self.model.resting_page();
            }
            Action::Save => {
                let was = self.model.settings.check_updates;
                self.model.settings = self.model.draft;
                if let Err(error) = self.model.settings.save(&self.settings_file) {
                    self.model.message = Some(format!("Could not save the settings: {error}"));
                }
                self.model.page = self.model.resting_page();
                match (was, self.model.settings.check_updates) {
                    // Starting the check needs the state this method holds, so it goes
                    // through the message queue.
                    // SAFETY: the window belongs to this thread.
                    (false, true) => unsafe {
                        PostMessageW(self.window, WAKE, 2, 0);
                    },
                    (true, false) => self.model.update = UpdateState::Off,
                    _ => {}
                }
            }
            Action::Repair => self.start_job(Plan::repair(self.model.installed)),
            Action::Pick(renderer) => {
                if renderer.built() {
                    self.model.draft.renderer = renderer;
                }
            }
            Action::ToggleUpdates => {
                self.model.draft.check_updates = !self.model.draft.check_updates;
            }
            Action::InstallAndStart => self.start_job(Plan::install()),
            Action::Close => self.close(),
            Action::TryAgain => {
                if let Some(plan) = self.plan.clone() {
                    self.start_job(plan);
                }
            }
            Action::ToggleLog => self.model.show_log = !self.model.show_log,
            Action::Cancel => self.cancel.store(true, Ordering::Relaxed),
        }
        self.invalidate();
    }

    fn paint(&mut self) {
        let paint = &self.paint;
        let measure = |text: &str, font: Font| paint.measure(text, font);
        self.frame = view::frame(&self.model, &measure);
        // SAFETY: standard double-buffered painting of this thread's window; every
        // object made is released before returning.
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(self.window, &mut ps);
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(self.window, &mut client);
            let (w, h) = (client.right.max(1), client.bottom.max(1));
            let memory = CreateCompatibleDC(dc);
            let bitmap = CreateCompatibleBitmap(dc, w, h);
            let old = SelectObject(memory, bitmap);
            let mut g = null_mut();
            GdipCreateFromHDC(memory, &mut g);
            if !g.is_null() {
                self.paint.draw(g, &self.frame);
                GdipDeleteGraphics(g);
            }
            BitBlt(dc, 0, 0, w, h, memory, 0, 0, SRCCOPY);
            SelectObject(memory, old);
            DeleteObject(bitmap);
            DeleteDC(memory);
            EndPaint(self.window, &ps);
        }
    }

    fn point(&self, l: LPARAM) -> [f32; 2] {
        let x = (l & 0xFFFF) as i16 as f32;
        let y = ((l >> 16) & 0xFFFF) as i16 as f32;
        [x / self.paint.scale, y / self.paint.scale]
    }
}

/// Runs `f` with the window's state, unless it is already in use.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|app| {
        let mut app = app.try_borrow_mut().ok()?;
        app.as_mut().map(f)
    })
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            if with_app(App::paint).is_none() {
                // SAFETY: the default painting validates the window.
                return unsafe { DefWindowProcW(window, message, w, l) };
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            with_app(|app| {
                if !app.tracking {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: window,
                        dwHoverTime: 0,
                    };
                    // SAFETY: asks for a message when the mouse leaves this window.
                    unsafe { TrackMouseEvent(&mut track) };
                    app.tracking = true;
                }
                let hover = app.frame.hit(app.point(l));
                if hover != app.model.hover {
                    app.model.hover = hover;
                    app.invalidate();
                }
            });
            0
        }
        WM_MOUSELEAVE => {
            with_app(|app| {
                app.tracking = false;
                if app.model.hover.take().is_some() {
                    app.invalidate();
                }
            });
            0
        }
        WM_SETCURSOR if (l & 0xFFFF) as u32 == HTCLIENT => {
            let hand = with_app(|app| app.model.hover.is_some()).unwrap_or(false);
            // SAFETY: standard cursors.
            unsafe {
                SetCursor(LoadCursorW(
                    null_mut(),
                    if hand { IDC_HAND } else { IDC_ARROW },
                ))
            };
            1
        }
        WM_LBUTTONDOWN => {
            with_app(|app| {
                app.model.pressed = app.frame.hit(app.point(l));
                app.invalidate();
            });
            // SAFETY: keeps the mouse until the button is released.
            unsafe { SetCapture(window) };
            0
        }
        WM_LBUTTONUP => {
            // SAFETY: gives the mouse back.
            unsafe { ReleaseCapture() };
            with_app(|app| {
                let pressed = app.model.pressed.take();
                let here = app.frame.hit(app.point(l));
                if let Some(action) = pressed.filter(|p| Some(*p) == here) {
                    app.perform(action);
                }
                app.invalidate();
            });
            0
        }
        WM_KEYDOWN => {
            with_app(|app| {
                let action = match w as u16 {
                    VK_RETURN => app.frame.enter,
                    VK_ESCAPE => app.frame.escape,
                    _ => None,
                };
                if let Some(action) = action {
                    app.perform(action);
                }
            });
            0
        }
        WAKE => {
            match w {
                // --press-continue: what Enter does on the first page.
                1 => {
                    with_app(|app| {
                        let measure = |t: &str, f: Font| app.paint.measure(t, f);
                        let enter = view::frame(&app.model, &measure).enter;
                        if let Some(action) = enter {
                            app.perform(action);
                        }
                    });
                }
                2 => start_check(),
                _ => {
                    with_app(App::wake);
                }
            }
            0
        }
        WM_DESTROY => {
            // SAFETY: posts to this thread's own queue.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: the default handling for everything else, WM_CLOSE included.
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}
