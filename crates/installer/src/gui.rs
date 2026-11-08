//! The setup window, made from Windows's own controls so setup needs nothing it would
//! have to install first.
//!
//! First screen: the list of what the modeler needs, each marked required or
//! optional, with what is already there. Required items cannot be unticked. After
//! Continue, a console shows each real step, with a progress bar for the download.
//! The work runs on a second thread, which wakes the window to show each step.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows_sys::Win32::Foundation::FreeLibrary;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, GetDC, GetDeviceCaps, HDC, LOGPIXELSY,
    ReleaseDC, SetBkColor, SetTextColor,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::Controls::{
    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS, PBM_SETRANGE32,
    PROGRESS_CLASSW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CW_USEDEFAULT,
    CreateWindowExW, DefWindowProcW, DispatchMessageW, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY,
    GetMessageW, GetSystemMetrics, IDC_ARROW, IsDialogMessageW, LoadCursorW, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE, SW_SHOW, SendMessageW,
    SetProcessDPIAware, SetWindowTextW, ShowWindow, TranslateMessage, WM_APP, WM_COMMAND,
    WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY, WM_SETFONT, WNDCLASSW, WS_BORDER, WS_CAPTION,
    WS_CHILD, WS_DISABLED, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
    WS_VSCROLL,
};

use crate::checks;
use crate::core::{self, Event, Found, NEEDED_OPENGL, SDL2};
use crate::net;

const CONTINUE: usize = 101;
const CLOSE: usize = 102;
const START: usize = 103;
/// Statuses and notes, drawn in grey.
const MUTED: usize = 200;
const WAKE: u32 = WM_APP + 1;
const APP_EXE: &str = "mixer.exe";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// What the work thread has to show. The window reads it when woken.
#[derive(Default)]
struct Shared {
    lines: Vec<String>,
    progress: f32,
    status: String,
    finished: Option<Result<String, String>>,
}

/// The window's controls and state, for the window procedure.
struct Window {
    console: HWND,
    progress: HWND,
    status: HWND,
    continue_button: HWND,
    start_button: HWND,
    shared: Arc<Mutex<Shared>>,
    lines_shown: usize,
    folder: PathBuf,
    opengl_ok: bool,
    opengl_line: String,
}

thread_local! {
    static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) };
    /// Plain values the window procedure may need while `WINDOW` is in use.
    static BACKGROUND: Cell<isize> = const { Cell::new(0) };
}

/// Installs without a window, printing each step, for automated checks on GitHub.
/// Returns the process exit code.
pub fn run_without_window() -> i32 {
    let folder = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let opengl_line = match checks::opengl() {
        Ok(gl) => format!("OpenGL {} on {}", gl.version, gl.renderer),
        Err(reason) => reason,
    };
    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut send = |event: Event| {
        if let Event::Line(line) = event {
            println!("{line}");
        }
    };
    match work(&folder, true, &opengl_line, &mut send, &shared, &|| {}) {
        Ok(message) => {
            println!("{message}");
            0
        }
        Err(message) => {
            println!("ERROR: {message}");
            1
        }
    }
}

/// Shows the setup window and runs until it closes.
pub fn run() {
    let folder = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let found = core::find(&SDL2, &folder);
    let opengl = checks::opengl();
    let (opengl_ok, opengl_line) = match &opengl {
        Ok(gl) => match core::opengl_version(&gl.version) {
            Some(v) if v >= NEEDED_OPENGL => (
                true,
                format!("Found: OpenGL {} on {}.", gl.version, gl.renderer),
            ),
            _ => (
                false,
                format!(
                    "Too old: OpenGL {} on {}. Update the graphics driver through Windows \
                     Update or the maker's website. Setup cannot install drivers.",
                    gl.version, gl.renderer
                ),
            ),
        },
        Err(reason) => (
            false,
            format!(
                "Not found: {reason} Update the graphics driver through Windows Update or \
                 the maker's website. Setup cannot install drivers."
            ),
        ),
    };

    // SAFETY: standard Win32 window creation on this thread. Every string passed
    // stays alive for its call, and every handle is used only on this thread.
    unsafe {
        SetProcessDPIAware();
        let screen = GetDC(null_mut());
        let dpi = GetDeviceCaps(screen, LOGPIXELSY as i32);
        ReleaseDC(null_mut(), screen);
        let scale = |v: i32| v * dpi / 96;
        let font = |points: i32, weight: i32, face: &str| {
            let face = wide(face);
            CreateFontW(
                -(points * dpi / 72),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                5, // ClearType
                0,
                face.as_ptr(),
            )
        };
        let (body, title_font, mono) = (
            font(9, 400, "Segoe UI"),
            font(15, 600, "Segoe UI"),
            font(9, 400, "Consolas"),
        );

        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        InitCommonControlsEx(&controls);

        let instance = GetModuleHandleW(null());
        let class = wide("MixerSetup");
        let background = CreateSolidBrush(0x00FF_FFFF);
        BACKGROUND.set(background as isize);
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.lpfnWndProc = Some(window_procedure);
        window_class.hInstance = instance;
        window_class.lpszClassName = class.as_ptr();
        window_class.hbrBackground = background;
        window_class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        RegisterClassW(&window_class);

        let (width, height) = (scale(640), scale(600));
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut frame = windows_sys::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        AdjustWindowRect(&mut frame, style, 0);
        let (outer_w, outer_h) = (frame.right - frame.left, frame.bottom - frame.top);
        let title = wide("Mixer setup");
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
        let _ = CW_USEDEFAULT;

        let add = |kind: &str, text: &str, style: u32, id: usize, rect: [i32; 4], font| {
            let (kind, text) = (wide(kind), wide(text));
            let control = CreateWindowExW(
                0,
                kind.as_ptr(),
                text.as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                scale(rect[0]),
                scale(rect[1]),
                scale(rect[2]),
                scale(rect[3]),
                window,
                id as _,
                instance,
                null(),
            );
            SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
            control
        };
        let static_class = "STATIC";
        add(
            static_class,
            "Mixer setup",
            0,
            0,
            [24, 16, 592, 32],
            title_font,
        );
        add(
            static_class,
            &format!(
                "Setup installs only what Mixer needs, into the folder it is in:\n{}",
                folder.display()
            ),
            0,
            MUTED,
            [24, 54, 592, 40],
            body,
        );

        // The list. Both items are required, so their boxes are ticked and locked.
        let sdl_box = add(
            "BUTTON",
            &format!(
                "{} {}  (required)  for {}",
                SDL2.name, SDL2.version, SDL2.purpose
            ),
            BS_AUTOCHECKBOX as u32 | WS_DISABLED,
            0,
            [24, 106, 592, 22],
            body,
        );
        SendMessageW(sdl_box, BM_SETCHECK, 1, 0);
        let sdl_status = match found {
            Found::Installed => "Already installed. Nothing to download.".to_string(),
            Found::Missing => format!(
                "Not installed yet. Downloads {} from github.com/libsdl-org/SDL, the official \
                 site.",
                core::size_text(SDL2.size)
            ),
            Found::Different => format!(
                "A different SDL2.dll is here. It will be replaced. Downloads {}.",
                core::size_text(SDL2.size)
            ),
        };
        add(
            static_class,
            &sdl_status,
            0,
            MUTED,
            [44, 130, 572, 22],
            body,
        );

        let gl_box = add(
            "BUTTON",
            "Graphics driver with OpenGL 2.1 or newer  (required)  checked, not installed",
            BS_AUTOCHECKBOX as u32 | WS_DISABLED,
            0,
            [24, 162, 592, 22],
            body,
        );
        SendMessageW(gl_box, BM_SETCHECK, 1, 0);
        add(
            static_class,
            &opengl_line,
            0,
            MUTED,
            [44, 186, 572, 40],
            body,
        );
        add(
            static_class,
            "Optional: nothing. The C runtime is built into Mixer.",
            0,
            MUTED,
            [24, 234, 592, 22],
            body,
        );

        // The console and progress, shown after Continue.
        let console = add(
            "EDIT",
            "",
            ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32
                | WS_VSCROLL
                | WS_BORDER,
            0,
            [24, 266, 592, 210],
            mono,
        );
        let progress = add(
            &String::from_utf16_lossy(&PROGRESS_CLASSW_TEXT),
            "",
            0,
            0,
            [24, 486, 592, 14],
            body,
        );
        SendMessageW(progress, PBM_SETRANGE32, 0, 1000);
        let status = add(static_class, "", 0, MUTED, [24, 506, 592, 22], body);
        for control in [console, progress, status] {
            ShowWindow(control, SW_HIDE);
        }

        let start_button = add(
            "BUTTON",
            "Start Mixer",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            START,
            [252, 548, 176, 32],
            body,
        );
        ShowWindow(start_button, SW_HIDE);
        let continue_button = add(
            "BUTTON",
            "Continue",
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            CONTINUE,
            [436, 548, 88, 32],
            body,
        );
        add(
            "BUTTON",
            "Close",
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            CLOSE,
            [532, 548, 84, 32],
            body,
        );

        WINDOW.with(|w| {
            *w.borrow_mut() = Some(Window {
                console,
                progress,
                status,
                continue_button,
                start_button,
                shared: Arc::new(Mutex::new(Shared::default())),
                lines_shown: 0,
                folder,
                opengl_ok,
                opengl_line,
            })
        });

        ShowWindow(window, SW_SHOW);
        // For automated screenshots: press Continue straight away.
        if std::env::args().any(|a| a == "--press-continue") {
            PostMessageW(window, WM_COMMAND, CONTINUE, 0);
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

/// The progress bar's class name, without its trailing zero.
const PROGRESS_CLASSW_TEXT: [u16; 17] = {
    let mut out = [0u16; 17];
    let mut i = 0;
    while i < 17 {
        // SAFETY: PROGRESS_CLASSW points at "msctls_progress32" and its zero.
        out[i] = unsafe { *PROGRESS_CLASSW.add(i) };
        i += 1;
    }
    out
};

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND => {
            match w & 0xFFFF {
                CONTINUE => start_work(window),
                CLOSE => {
                    // SAFETY: posts to this thread's own queue.
                    unsafe { PostQuitMessage(0) };
                }
                START => {
                    let exe = WINDOW.with(|state| {
                        state
                            .try_borrow()
                            .ok()
                            .and_then(|s| s.as_ref().map(|s| s.folder.join(APP_EXE)))
                    });
                    if let Some(exe) = exe
                        && std::process::Command::new(&exe)
                            .current_dir(exe.parent().unwrap_or(&exe))
                            .spawn()
                            .is_ok()
                    {
                        // SAFETY: as above.
                        unsafe { PostQuitMessage(0) };
                    }
                }
                _ => {}
            }
            0
        }
        WAKE => {
            show_progress();
            0
        }
        // Text on a white background, with statuses in grey.
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let device = w as HDC;
            let control = l as HWND;
            // SAFETY: the device context and control come from Windows for this call.
            unsafe {
                let id = windows_sys::Win32::UI::WindowsAndMessaging::GetDlgCtrlID(control);
                SetBkColor(device, 0x00FF_FFFF);
                if id as usize == MUTED {
                    SetTextColor(device, 0x0055_5555);
                }
            }
            BACKGROUND.get() as LRESULT
        }
        WM_DESTROY => {
            // SAFETY: posts to this thread's own queue.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: the default handling for everything else.
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

/// Runs the install on a second thread, which wakes the window for each step.
fn start_work(window: HWND) {
    WINDOW.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else {
            return;
        };
        let Some(state) = state.as_mut() else {
            return;
        };
        // SAFETY: the controls belong to this thread's window.
        unsafe {
            for control in [state.console, state.progress, state.status] {
                ShowWindow(control, SW_SHOW);
            }
            ShowWindow(state.start_button, SW_HIDE);
            EnableWindow(state.continue_button, 0);
        }
        *state.shared.lock().unwrap_or_else(|e| e.into_inner()) = Shared::default();
        state.lines_shown = 0;
        // SAFETY: clears the console's text.
        unsafe { SetWindowTextW(state.console, wide("").as_ptr()) };

        let shared = Arc::clone(&state.shared);
        let folder = state.folder.clone();
        let (opengl_ok, opengl_line) = (state.opengl_ok, state.opengl_line.clone());
        // Window handles are plain numbers to other threads.
        let window = window as isize;
        std::thread::spawn(move || {
            let wake = || {
                // SAFETY: posting to a window from another thread is allowed; if the
                // window is gone, the post fails harmlessly.
                unsafe { PostMessageW(window as HWND, WAKE, 0, 0) };
            };
            let mut send = |event: Event| {
                {
                    let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
                    match event {
                        Event::Line(line) => shared.lines.push(line),
                        Event::Progress(p) => shared.progress = p,
                    }
                }
                wake();
            };
            let result = work(&folder, opengl_ok, &opengl_line, &mut send, &shared, &wake);
            {
                // The status line keeps the outcome, whatever wake-ups are still queued.
                let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
                shared.status = match &result {
                    Ok(message) => message.clone(),
                    Err(_) => "Something went wrong. See above.".to_string(),
                };
                shared.finished = Some(result);
            }
            wake();
        });
    });
}

/// The install itself, step by step.
fn work(
    folder: &std::path::Path,
    opengl_ok: bool,
    opengl_line: &str,
    send: &mut dyn FnMut(Event),
    shared: &Arc<Mutex<Shared>>,
    wake: &dyn Fn(),
) -> Result<String, String> {
    let status = |text: String| {
        shared.lock().unwrap_or_else(|e| e.into_inner()).status = text;
        wake();
    };
    send(Event::Line(format!("Folder: {}", folder.display())));
    if !core::can_write(folder) {
        return Err(
            "Setup cannot write into this folder. Move the unzipped folder somewhere like \
             Documents or the desktop, and run setup from there."
                .into(),
        );
    }
    send(Event::Line("  can be written to".into()));
    send(Event::Line(format!("Graphics driver: {opengl_line}")));

    match core::find(&SDL2, folder) {
        Found::Installed => {
            send(Event::Line(format!(
                "SDL2 {} is already installed, and matches its fingerprint. Nothing to \
                 download.",
                SDL2.version
            )));
            send(Event::Progress(0.95));
        }
        found => {
            if found == Found::Different {
                send(Event::Line(
                    "A different SDL2.dll is here, and will be replaced.".into(),
                ));
            }
            send(Event::Line(format!(
                "Downloading SDL2 {} from {}",
                SDL2.version,
                SDL2.url()
            )));
            status("Connecting to github.com...".into());
            let started = Instant::now();
            let mut last_tenth = u64::MAX;
            let zip = net::download(&SDL2, &mut |done, total| {
                let fraction = done as f32 / total.max(1) as f32;
                send(Event::Progress(0.8 * fraction));
                status(format!(
                    "Downloading: {} of {}",
                    core::size_text(done),
                    core::size_text(total)
                ));
                // A console line for every tenth, so the log shows real progress.
                let tenth = done * 10 / total.max(1);
                if tenth != last_tenth {
                    last_tenth = tenth;
                    send(Event::Line(format!(
                        "  {:>3}%  {}",
                        tenth * 10,
                        core::size_text(done)
                    )));
                }
            })?;
            send(Event::Line(format!(
                "  downloaded {} in {:.1} seconds",
                core::size_text(zip.len() as u64),
                started.elapsed().as_secs_f32()
            )));
            status("Checking and unzipping...".into());
            core::install(&SDL2, &zip, folder, send)?;
        }
    }

    // The real test: can Windows load the DLL?
    send(Event::Line(
        "Checking that Windows can load SDL2.dll...".into(),
    ));
    let path = wide(&folder.join("SDL2.dll").to_string_lossy());
    // SAFETY: the path ends in a zero; the library is freed straight away.
    let loaded = unsafe {
        let module = LoadLibraryW(path.as_ptr());
        let ok = !module.is_null();
        if ok {
            FreeLibrary(module);
        }
        ok
    };
    if !loaded {
        return Err("SDL2.dll is in place, but Windows cannot load it.".into());
    }
    send(Event::Line("  it loads".into()));
    send(Event::Progress(1.0));
    Ok(if opengl_ok {
        "Done. Mixer is ready to start.".to_string()
    } else {
        "Done, but the graphics driver lacks OpenGL 2.1, so Mixer may not start. \
         Update the driver, then start it."
            .to_string()
    })
}

/// Shows what the work thread has done since last time.
fn show_progress() {
    WINDOW.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else {
            return;
        };
        let Some(state) = state.as_mut() else {
            return;
        };
        let (new_lines, progress, status, finished) = {
            let shared = state.shared.lock().unwrap_or_else(|e| e.into_inner());
            (
                shared.lines[state.lines_shown.min(shared.lines.len())..].to_vec(),
                shared.progress,
                shared.status.clone(),
                shared.finished.clone(),
            )
        };
        state.lines_shown += new_lines.len();
        // SAFETY: all controls belong to this thread's window; strings end in a zero.
        unsafe {
            for line in new_lines {
                append(state.console, &format!("{line}\r\n"));
            }
            SendMessageW(state.progress, PBM_SETPOS, (progress * 1000.0) as WPARAM, 0);
            match &finished {
                None => {
                    SetWindowTextW(state.status, wide(&status).as_ptr());
                }
                Some(Ok(message)) => {
                    append(state.console, &format!("{message}\r\n"));
                    SetWindowTextW(state.status, wide(&status).as_ptr());
                    ShowWindow(state.continue_button, SW_HIDE);
                    ShowWindow(state.start_button, SW_SHOW);
                    state
                        .shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .finished = None;
                }
                Some(Err(message)) => {
                    append(state.console, &format!("ERROR: {message}\r\n"));
                    SetWindowTextW(state.status, wide(&status).as_ptr());
                    SetWindowTextW(state.continue_button, wide("Try again").as_ptr());
                    EnableWindow(state.continue_button, 1);
                    state
                        .shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .finished = None;
                }
            }
        }
    });
}

/// Adds text at the end of the console and scrolls to it.
///
/// # Safety
/// `console` must be an edit control of this thread.
unsafe fn append(console: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: the caller's promise; the text ends in a zero. -1 selects the end.
    unsafe {
        SendMessageW(
            console,
            windows_sys::Win32::UI::Controls::EM_SETSEL,
            usize::MAX,
            -1,
        );
        SendMessageW(
            console,
            windows_sys::Win32::UI::Controls::EM_REPLACESEL,
            0,
            text.as_ptr() as LPARAM,
        );
    }
}
