//! Checks for what no program can install: OpenGL support in the graphics driver.
//! Asks Windows for an OpenGL context in a hidden window, the same way the app's
//! window gets one, and reads what the driver reports.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
use windows_sys::Win32::Graphics::OpenGL::{
    ChoosePixelFormat, GL_RENDERER, GL_VERSION, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW,
    PFD_MAIN_PLANE, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR, SetPixelFormat,
    glGetString, wglCreateContext, wglDeleteContext, wglMakeCurrent,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WNDCLASSW, WS_OVERLAPPED,
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// What the graphics driver reports.
pub struct OpenGl {
    pub version: String,
    pub renderer: String,
}

/// Reads the OpenGL version and graphics chip, or says why it could not.
pub fn opengl() -> Result<OpenGl, String> {
    // SAFETY: a hidden window and context are made, used and destroyed in order;
    // every handle is checked, and strings from OpenGL are read up to their zero.
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide("LowEndModelerSetupGlCheck");
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.lpfnWndProc = Some(DefWindowProcW);
        window_class.hInstance = instance;
        window_class.lpszClassName = class.as_ptr();
        RegisterClassW(&window_class);
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            16,
            16,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return Err("Windows could not make a test window.".into());
        }
        let device = GetDC(window);
        let mut format: PIXELFORMATDESCRIPTOR = std::mem::zeroed();
        format.nSize = std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16;
        format.nVersion = 1;
        format.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
        format.iPixelType = PFD_TYPE_RGBA;
        format.cColorBits = 32;
        format.cDepthBits = 24;
        format.iLayerType = PFD_MAIN_PLANE as u8;
        let chosen = ChoosePixelFormat(device, &format);
        let result = if chosen == 0 || SetPixelFormat(device, chosen, &format) == 0 {
            Err("The graphics driver offers no OpenGL display mode.".to_string())
        } else {
            let context = wglCreateContext(device);
            if context.is_null() {
                Err("The graphics driver would not start OpenGL.".to_string())
            } else {
                wglMakeCurrent(device, context);
                let read = |name: u32| {
                    let text = glGetString(name);
                    if text.is_null() {
                        String::new()
                    } else {
                        std::ffi::CStr::from_ptr(text.cast())
                            .to_string_lossy()
                            .into_owned()
                    }
                };
                let found = OpenGl {
                    version: read(GL_VERSION),
                    renderer: read(GL_RENDERER),
                };
                wglMakeCurrent(device, null_mut());
                wglDeleteContext(context);
                Ok(found)
            }
        };
        ReleaseDC(window, device);
        DestroyWindow(window);
        result
    }
}
