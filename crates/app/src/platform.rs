//! What only Windows has: its own file dialogs, message boxes, and the check for
//! SDL2.dll before anything uses it. On other systems the app asks for file paths in
//! its own window instead (see `ui.rs`).
//!
//! The development machine cannot build for Windows, so this code is type-checked
//! there with `RUSTFLAGS="--cfg check_windows" cargo check`, and built by GitHub
//! Actions.

#[cfg(any(windows, check_windows))]
pub use windows_only::*;

#[cfg(any(windows, check_windows))]
#[cfg_attr(check_windows, allow(dead_code))]
mod windows_only {
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
        OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_ICONWARNING, MB_YESNO, MessageBoxW,
    };

    /// Text as Windows wants it: UTF-16 ending in a zero.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A file type for a dialog: its description and its pattern, such as `*.lem`.
    pub struct FileType<'a> {
        pub name: &'a str,
        pub pattern: &'a str,
    }

    /// Shows Windows's Open or Save As dialog. Returns the chosen path, or `None` if
    /// the person cancelled. Save As asks before replacing a file.
    pub fn pick_file(
        save: bool,
        title: &str,
        types: &[FileType],
        folder: &Path,
        name: &str,
        extension: &str,
    ) -> Option<PathBuf> {
        // Pairs of description and pattern, each ending in a zero, then one more zero.
        let mut filter: Vec<u16> = Vec::new();
        for kind in types {
            filter.extend(kind.name.encode_utf16());
            filter.push(0);
            filter.extend(kind.pattern.encode_utf16());
            filter.push(0);
        }
        filter.push(0);
        // Room for a long path, starting with the suggested name.
        let mut file = vec![0u16; 4096];
        for (slot, unit) in file.iter_mut().zip(name.encode_utf16().take(4000)) {
            *slot = unit;
        }
        let folder = wide(&folder.to_string_lossy());
        let title = wide(title);
        let extension = wide(extension);
        let mut flags = OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST;
        flags |= if save {
            OFN_OVERWRITEPROMPT
        } else {
            OFN_FILEMUSTEXIST
        };
        // SAFETY: OPENFILENAMEW is plain data, and all-zero is a valid empty value.
        let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        // SAFETY: no arguments; returns the active window of this thread, or null.
        dialog.hwndOwner = unsafe { GetActiveWindow() };
        dialog.lpstrFilter = filter.as_ptr();
        dialog.nFilterIndex = 1;
        dialog.lpstrFile = file.as_mut_ptr();
        dialog.nMaxFile = file.len() as u32;
        dialog.lpstrInitialDir = folder.as_ptr();
        dialog.lpstrTitle = title.as_ptr();
        dialog.lpstrDefExt = extension.as_ptr();
        dialog.Flags = flags;
        // SAFETY: every pointer in `dialog` points into a buffer that outlives the call,
        // and the file buffer's length is given.
        let chosen = unsafe {
            if save {
                GetSaveFileNameW(&mut dialog)
            } else {
                GetOpenFileNameW(&mut dialog)
            }
        };
        if chosen == 0 {
            return None;
        }
        let end = file.iter().position(|&c| c == 0).unwrap_or(file.len());
        Some(PathBuf::from(String::from_utf16_lossy(&file[..end])))
    }

    /// Asks a yes or no question in a Windows message box. True means yes.
    pub fn ask(title: &str, text: &str) -> bool {
        let (title, text) = (wide(title), wide(text));
        // SAFETY: both strings end in a zero and outlive the call.
        let answer = unsafe {
            MessageBoxW(
                GetActiveWindow(),
                text.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            )
        };
        answer == IDYES
    }

    /// True if SDL2.dll cannot be loaded. The .exe loads it late (see build.rs), so
    /// this runs before anything uses SDL.
    pub fn sdl_is_missing() -> bool {
        let name = wide("SDL2.dll");
        // SAFETY: the name ends in a zero. A loaded DLL stays loaded, which SDL needs.
        unsafe { LoadLibraryW(name.as_ptr()).is_null() }
    }
}
