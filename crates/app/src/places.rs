//! Where Mixer keeps things: Documents › Mixer, with Projects and Quick Projects
//! inside, plus its settings and the list of recent projects.
//!
//! Setting `MIXER_HOME` puts the Mixer folder somewhere else, for testing.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::export::write_whole;

/// How many recent projects to remember.
pub const RECENT_COUNT: usize = 8;

/// The user's Documents folder.
fn documents() -> PathBuf {
    #[cfg(windows)]
    if let Some(folder) = windows_documents() {
        return folder;
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Documents")
}

#[cfg(any(windows, check_windows))]
#[cfg_attr(check_windows, allow(dead_code))]
fn windows_documents() -> Option<PathBuf> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};
    let mut path: *mut u16 = std::ptr::null_mut();
    // SAFETY: Windows writes a pointer to a zero-ended string it allocated, which is
    // read up to the zero and then freed with the function it names for that.
    unsafe {
        let result = SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut path);
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
        let text = std::slice::from_raw_parts(path, length);
        let folder = PathBuf::from(String::from_utf16_lossy(text));
        CoTaskMemFree(path as *const _);
        Some(folder)
    }
}

/// Documents › Mixer.
pub fn mixer_folder() -> PathBuf {
    match std::env::var_os("MIXER_HOME") {
        Some(home) => PathBuf::from(home),
        None => documents().join("Mixer"),
    }
}

pub fn projects_folder() -> PathBuf {
    mixer_folder().join("Projects")
}

pub fn quick_folder() -> PathBuf {
    mixer_folder().join("Quick Projects")
}

/// Where a project's exports go: an Exports folder beside it.
pub fn exports_beside(project: &Path) -> PathBuf {
    project
        .parent()
        .map(|folder| folder.join("Exports"))
        .unwrap_or_else(|| PathBuf::from("Exports"))
}

/// Mixer's own settings, kept between runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Smooth edges. Takes effect when Mixer next starts.
    pub antialiasing: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { antialiasing: true }
    }
}

impl Settings {
    fn path() -> PathBuf {
        mixer_folder().join("settings.txt")
    }

    /// Reads the settings, or the defaults if there are none or they are unreadable.
    pub fn load() -> Self {
        let text = std::fs::read_to_string(Self::path()).unwrap_or_default();
        Self::parse(&text)
    }

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            if let (Some("antialiasing"), Some(value)) = (words.next(), words.next()) {
                settings.antialiasing = value != "off";
            }
        }
        settings
    }

    fn text(&self) -> String {
        let on = if self.antialiasing { "on" } else { "off" };
        format!("# Mixer settings\nantialiasing {on}\n")
    }

    pub fn save(&self) -> Result<(), String> {
        write_whole(&Self::path(), self.text().as_bytes())
    }
}

/// A recent project, for the start window.
#[derive(Debug, Clone, PartialEq)]
pub struct Recent {
    pub path: PathBuf,
    /// When it was last saved, as "5 minutes ago", if known.
    pub age: String,
}

fn recent_path() -> PathBuf {
    mixer_folder().join("recent.txt")
}

/// Recent projects that still exist, newest first.
pub fn recent_projects() -> Vec<Recent> {
    let text = std::fs::read_to_string(recent_path()).unwrap_or_default();
    text.lines()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .take(RECENT_COUNT)
        .map(|path| {
            let age = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|when| SystemTime::now().duration_since(when).ok())
                .map(|gone| describe_age(gone.as_secs()))
                .unwrap_or_default();
            Recent { path, age }
        })
        .collect()
}

/// Puts a project at the top of the recent list.
pub fn remember(project: &Path) {
    let text = std::fs::read_to_string(recent_path()).unwrap_or_default();
    let mut paths: Vec<String> = vec![project.display().to_string()];
    paths.extend(
        text.lines()
            .filter(|line| !line.is_empty() && Path::new(line) != project)
            .take(RECENT_COUNT * 2)
            .map(str::to_string),
    );
    let _ = write_whole(&recent_path(), (paths.join("\n") + "\n").as_bytes());
}

/// "just now", "5 minutes ago", "3 hours ago", "2 days ago".
pub fn describe_age(seconds: u64) -> String {
    let unit = |n: u64, one: &str| {
        if n == 1 {
            format!("1 {one} ago")
        } else {
            format!("{n} {one}s ago")
        }
    };
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => unit(seconds / 60, "minute"),
        3600..86_400 => unit(seconds / 3600, "hour"),
        _ => unit(seconds / 86_400, "day"),
    }
}

/// The date and time now, as "2026-11-08 02.49", for naming quick projects. Local time
/// on Windows; elsewhere, where only development happens, UTC.
pub fn timestamp() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        // SAFETY: Windows fills in the structure it is given.
        let t = unsafe {
            let mut t: SYSTEMTIME = std::mem::zeroed();
            GetLocalTime(&mut t);
            t
        };
        format!(
            "{:04}-{:02}-{:02} {:02}.{:02}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute
        )
    }
    #[cfg(not(windows))]
    {
        let seconds = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
        let minutes = (seconds % 86_400) / 60;
        format!(
            "{year:04}-{month:02}-{day:02} {:02}.{:02}",
            minutes / 60,
            minutes % 60
        )
    }
}

/// The calendar date of a day counted from 1970-01-01 (Howard Hinnant's method).
#[cfg_attr(windows, allow(dead_code))]
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// A project name that can be a folder name: what Windows forbids becomes `_`.
/// `None` if nothing is left.
pub fn clean_project_name(name: &str) -> Option<String> {
    (!name.trim().is_empty()).then(|| crate::export::clean_file_name(name))
}

/// A new project in its own folder inside `parent`: `parent/stem/stem.extension`, with
/// " 2", " 3" and so on added to both if a folder of that name is already there.
pub fn unused_project_path(parent: &Path, stem: &str, extension: &str) -> PathBuf {
    let mut name = stem.to_string();
    let mut n = 2;
    while parent.join(&name).exists() {
        name = format!("{stem} {n}");
        n += 1;
    }
    parent.join(&name).join(format!("{name}.{extension}"))
}

/// What deleting a project removes: its whole folder when the project has one of its
/// own in Projects or Quick Projects (so its exports go too), otherwise only the file.
pub fn deletion_target(project: &Path) -> PathBuf {
    let own_folder = project.parent().filter(|folder| {
        let named_after = folder.file_name() == project.file_stem();
        let inside = folder
            .parent()
            .is_some_and(|outer| outer == projects_folder() || outer == quick_folder());
        named_after && inside
    });
    own_folder.unwrap_or(project).to_path_buf()
}

/// Takes a project off the recent list.
pub fn forget(project: &Path) {
    let text = std::fs::read_to_string(recent_path()).unwrap_or_default();
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !line.is_empty() && Path::new(line) != project)
        .collect();
    let _ = write_whole(&recent_path(), (kept.join("\n") + "\n").as_bytes());
}

/// Moves a file or folder to the Recycle Bin, so it can be restored. Elsewhere, where
/// only development happens, it moves to a Deleted folder inside the Mixer folder.
pub fn recycle(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_recycle(path)
    }
    #[cfg(not(windows))]
    {
        let bin = mixer_folder().join("Deleted");
        std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
        let name = path.file_name().ok_or("nothing to delete")?;
        let mut target = bin.join(name);
        let mut n = 2;
        while target.exists() {
            target = bin.join(format!("{} {n}", name.to_string_lossy()));
            n += 1;
        }
        std::fs::rename(path, target).map_err(|e| e.to_string())
    }
}

#[cfg(any(windows, check_windows))]
#[cfg_attr(check_windows, allow(dead_code))]
fn windows_recycle(path: &Path) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };
    // The list of paths ends with two zeros.
    let mut from: Vec<u16> = path.display().to_string().encode_utf16().collect();
    from.extend([0, 0]);
    // SAFETY: the structure is zeroed, then given a list that lives until the call ends.
    let result = unsafe {
        let mut operation: SHFILEOPSTRUCTW = std::mem::zeroed();
        operation.wFunc = FO_DELETE;
        operation.pFrom = from.as_ptr();
        operation.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16;
        let result = SHFileOperationW(&mut operation);
        if operation.fAnyOperationsAborted != 0 {
            return Err("Windows stopped it".to_string());
        }
        result
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "Windows could not move it to the Recycle Bin (code {result})"
        ))
    }
}

/// Shows a project in its folder: on Windows, Explorer opens with the file selected.
pub fn show_in_folder(project: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", project.display()))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Err(format!(
            "It is in {}",
            project.parent().unwrap_or(project).display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_naturally() {
        assert_eq!(describe_age(5), "just now");
        assert_eq!(describe_age(60), "1 minute ago");
        assert_eq!(describe_age(7200), "2 hours ago");
        assert_eq!(describe_age(86_400 * 3), "3 days ago");
    }

    #[test]
    fn dates_come_out_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_400), (2025, 11, 8));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn settings_read_back_and_default_to_on() {
        assert!(Settings::parse("").antialiasing);
        let off = Settings {
            antialiasing: false,
        };
        assert_eq!(Settings::parse(&off.text()), off);
    }

    #[test]
    fn each_new_project_gets_a_folder_of_its_own() {
        let parent = std::env::temp_dir().join(format!("mixer-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let first = unused_project_path(&parent, "Quick", "mix");
        assert_eq!(first, parent.join("Quick").join("Quick.mix"));
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        let second = unused_project_path(&parent, "Quick", "mix");
        assert_eq!(second, parent.join("Quick 2").join("Quick 2.mix"));
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn deleting_takes_a_projects_own_folder_but_nothing_more() {
        let own = projects_folder().join("Sword").join("Sword.mix");
        assert_eq!(deletion_target(&own), projects_folder().join("Sword"));
        let quick = quick_folder().join("Quick 1").join("Quick 1.mix");
        assert_eq!(deletion_target(&quick), quick_folder().join("Quick 1"));
        // An older quick project lies loose in Quick Projects.
        let loose = quick_folder().join("Quick 0.mix");
        assert_eq!(deletion_target(&loose), loose);
        // A project opened from anywhere else: only the file.
        let elsewhere = PathBuf::from("/somewhere/Sword/Sword.mix");
        assert_eq!(deletion_target(&elsewhere), elsewhere);
    }

    #[test]
    fn project_names_must_have_something_in_them() {
        assert_eq!(clean_project_name("  "), None);
        assert_eq!(
            clean_project_name("Sword: v2").as_deref(),
            Some("Sword_ v2")
        );
        assert_eq!(clean_project_name("model").as_deref(), Some("model"));
    }
}
