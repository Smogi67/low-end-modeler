//! Updating Mixer from its GitHub Releases, without any Windows code, so it is tested
//! on any system.
//!
//! Each finished stage is published as a release named after its version (`v0.10.0`)
//! with two files: `mixer-0.10.0-windows.zip`, the same folder people unzip by hand,
//! and `mixer-0.10.0-windows.zip.sha256`, its fingerprint. The launcher asks GitHub
//! for the newest release, compares versions, downloads the zip, checks the
//! fingerprint and size, and replaces Mixer's files in its folder. Projects live in
//! Documents and are never touched.

use std::fmt;
use std::path::Path;

use crate::core::{sha256_hex, unzip_all};
use crate::json::{self, Value};

/// Where the releases are.
pub const API_HOST: &str = "api.github.com";
pub const LATEST_PATH: &str = "/repos/Smogi67/mixer/releases/latest";
/// Followed by a tag such as v0.10.0.
pub const TAG_PATH: &str = "/repos/Smogi67/mixer/releases/tags/";

/// The program files a release contains, besides the license texts.
pub const APP_EXE: &str = "mixer.exe";
pub const LAUNCHER_EXE: &str = "mixer-launcher.exe";
pub const VERSION_FILE: &str = "version.txt";
/// The launcher's previous .exe, moved aside while it runs, deleted on the next start.
pub const OLD_LAUNCHER: &str = "mixer-launcher.old.exe";
const NEW_SUFFIX: &str = ".new";
/// A release zip bigger than this is refused.
const MOST_ZIP_BYTES: u64 = 64 * 1024 * 1024;

/// A version number such as 0.10.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// This launcher's own version.
    pub fn own() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self(0, 0, 0))
    }

    /// Reads "0.10.0", "v0.10.0" or "0.10", ignoring spaces and a line end.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let mut parts = text.split('.');
        let mut number = || -> Option<u32> {
            match parts.next() {
                Some(part) => part.parse().ok(),
                None => Some(0),
            }
        };
        let version = Self(number()?, number()?, number()?);
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The version of Mixer in `folder`: what its version.txt says, or, for a folder from
/// before version files, this launcher's own version.
pub fn installed_version(folder: &Path) -> Version {
    std::fs::read_to_string(folder.join(VERSION_FILE))
        .ok()
        .and_then(|text| Version::parse(&text))
        .unwrap_or_else(Version::own)
}

/// A file attached to a release.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// The newest release, as GitHub describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    pub notes: String,
    pub zip: Asset,
    pub fingerprint: Asset,
}

/// Reads GitHub's answer to "latest release".
pub fn parse_release(text: &str) -> Result<Release, String> {
    let value = json::parse(text)?;
    let tag = value
        .get("tag_name")
        .and_then(Value::text)
        .ok_or("GitHub's answer has no version name.")?;
    let version = Version::parse(tag)
        .ok_or_else(|| format!("The newest release is called {tag:?}, which is not a version."))?;
    let notes = value
        .get("body")
        .and_then(Value::text)
        .unwrap_or_default()
        .to_string();
    let assets: Vec<Asset> = value
        .get("assets")
        .and_then(Value::list)
        .unwrap_or_default()
        .iter()
        .filter_map(|asset| {
            Some(Asset {
                name: asset.get("name")?.text()?.to_string(),
                url: asset.get("browser_download_url")?.text()?.to_string(),
                size: asset.get("size")?.number()? as u64,
            })
        })
        .collect();
    let zip = assets
        .iter()
        .find(|a| a.name.ends_with("-windows.zip"))
        .cloned()
        .ok_or_else(|| format!("Release {version} has no Windows download."))?;
    let fingerprint = assets
        .iter()
        .find(|a| a.name == format!("{}.sha256", zip.name))
        .cloned()
        .ok_or_else(|| format!("Release {version} has no fingerprint file for its download."))?;
    if !zip.url.starts_with("https://github.com/")
        || !fingerprint.url.starts_with("https://github.com/")
    {
        return Err("The release's download is not on github.com, so it was not used.".into());
    }
    if zip.size > MOST_ZIP_BYTES {
        return Err("The release's download is far bigger than expected.".into());
    }
    Ok(Release {
        version,
        notes,
        zip,
        fingerprint,
    })
}

/// Up to three short lines about what is new, from the release notes' list items.
pub fn notes_summary(notes: &str) -> Vec<String> {
    let items: Vec<String> = notes
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")))
        .map(|line| line.replace("**", "").replace('`', ""))
        .filter(|line| !line.is_empty())
        .take(3)
        .collect();
    if items.is_empty() {
        notes
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| vec![line.to_string()])
            .unwrap_or_default()
    } else {
        items
    }
}

/// The fingerprint written in a .sha256 file: its first word, 64 hex digits.
pub fn read_fingerprint(text: &str) -> Option<String> {
    let word = text.split_whitespace().next()?.to_ascii_lowercase();
    (word.len() == 64 && word.bytes().all(|b| b.is_ascii_hexdigit())).then_some(word)
}

/// Checks a downloaded release zip against its fingerprint and size.
pub fn check_download(zip: &[u8], release: &Release, fingerprint_file: &str) -> Result<(), String> {
    let expected = read_fingerprint(fingerprint_file)
        .ok_or("The release's fingerprint file is unreadable, so nothing was changed.")?;
    if zip.len() as u64 != release.zip.size {
        return Err(format!(
            "The download is {} bytes, but GitHub said {}. Nothing was changed.",
            zip.len(),
            release.zip.size
        ));
    }
    let found = sha256_hex(zip);
    if found != expected {
        return Err(format!(
            "The download does not match its fingerprint, so it was changed or damaged on \
             the way. Nothing was changed.\r\n  expected {expected}\r\n  received {found}"
        ));
    }
    Ok(())
}

/// A file from a release, with its path inside the Mixer folder.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFile {
    /// Relative, with `/` between folders.
    pub path: String,
    pub bytes: Vec<u8>,
}

/// True for the paths a release may write: files at the top of the folder, and in
/// `licenses`. Nothing outside the folder, and nothing else inside it.
fn allowed(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let plain = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.contains(':')
            && !part.ends_with(NEW_SUFFIX)
    };
    match parts[..] {
        [file] => plain(file) && file != OLD_LAUNCHER,
        ["licenses", file] => plain(file),
        _ => false,
    }
}

/// Unpacks a release zip into the files to write, after checking that it holds Mixer,
/// the launcher and a version file that matches the release.
pub fn unpack(zip: &[u8], release: &Release) -> Result<Vec<NewFile>, String> {
    let files = unzip_all(zip)?;
    let mut out = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        // The zip may hold its files in one top folder; that folder is left out.
        let path = path.trim_start_matches("./").to_string();
        if !allowed(&path) {
            return Err(format!(
                "The download holds {path:?}, which the launcher does not write. Nothing \
                 was changed."
            ));
        }
        out.push(NewFile { path, bytes });
    }
    for needed in [APP_EXE, LAUNCHER_EXE, VERSION_FILE] {
        if !out.iter().any(|f| f.path == needed) {
            return Err(format!("The download lacks {needed}. Nothing was changed."));
        }
    }
    let stated = out
        .iter()
        .find(|f| f.path == VERSION_FILE)
        .and_then(|f| Version::parse(&String::from_utf8_lossy(&f.bytes)));
    if stated != Some(release.version) {
        return Err(format!(
            "The download says it is version {}, not {}. Nothing was changed.",
            stated.map_or("unknown".to_string(), |v| v.to_string()),
            release.version
        ));
    }
    Ok(out)
}

/// Writes the new files into `folder`. Each is written beside its old one first, so a
/// failure while writing changes nothing. Mixer goes in first: if it is open, Windows
/// refuses, and nothing has changed yet. The running launcher cannot be overwritten,
/// but it can be renamed, so it moves aside and is deleted on the next start. The
/// version file goes last, so an update cut short is offered again.
pub fn apply(
    folder: &Path,
    files: &[NewFile],
    events: &mut dyn FnMut(String),
) -> Result<(), String> {
    let target =
        |file: &NewFile| folder.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
    let staged = |file: &NewFile| {
        let mut name = target(file).into_os_string();
        name.push(NEW_SUFFIX);
        std::path::PathBuf::from(name)
    };
    let clean_up = || {
        for file in files {
            let _ = std::fs::remove_file(staged(file));
        }
        // Only if empty: a licenses folder made for the new files.
        let _ = std::fs::remove_dir(folder.join("licenses"));
    };

    for file in files {
        let path = staged(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not make {}: {e}", parent.display()))?;
        }
        if let Err(error) = std::fs::write(&path, &file.bytes) {
            clean_up();
            return Err(format!("Could not write {}: {error}", path.display()));
        }
    }
    events(format!(
        "Wrote {} new files beside the old ones",
        files.len()
    ));

    let order = |file: &&NewFile| match file.path.as_str() {
        APP_EXE => 0,
        VERSION_FILE => 2,
        _ => 1,
    };
    let mut sorted: Vec<&NewFile> = files.iter().collect();
    sorted.sort_by_key(order);
    for (i, file) in sorted.iter().enumerate() {
        let to = target(file);
        if file.path == LAUNCHER_EXE && to.exists() {
            let aside = folder.join(OLD_LAUNCHER);
            let _ = std::fs::remove_file(&aside);
            std::fs::rename(&to, &aside)
                .map_err(|e| format!("Could not move the old launcher aside: {e}"))?;
        }
        if let Err(error) = std::fs::rename(staged(file), &to) {
            if i == 0 {
                clean_up();
                return Err(format!(
                    "Could not replace {}: {error}. If Mixer is open, close it and press Try \
                     again. Nothing was changed.",
                    file.path
                ));
            }
            return Err(format!(
                "Could not replace {}: {error}. Press Try again to finish the update.",
                file.path
            ));
        }
        events(format!("  replaced {}", file.path));
    }
    Ok(())
}

/// Deletes what an earlier update left behind: the old launcher and unfinished files.
pub fn clean_up_after_update(folder: &Path) {
    let _ = std::fs::remove_file(folder.join(OLD_LAUNCHER));
    for dir in [folder.to_path_buf(), folder.join("licenses")] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(NEW_SUFFIX) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Splits an https address into host and path, for WinHTTP.
pub fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let slash = rest.find('/')?;
    Some((&rest[..slash], &rest[slash..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tests::{make_zip, temp_folder};

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(Version::parse("v0.10.0"), Some(Version(0, 10, 0)));
        assert_eq!(Version::parse("0.10\n"), Some(Version(0, 10, 0)));
        assert_eq!(Version::parse("1"), Some(Version(1, 0, 0)));
        assert_eq!(Version::parse("0.10.0.1"), None);
        assert_eq!(Version::parse("beta"), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 0));
        assert_eq!(Version(0, 10, 0).to_string(), "0.10.0");
        assert_eq!(Version::own().to_string(), env!("CARGO_PKG_VERSION"));
    }

    /// The shape of GitHub's answer, trimmed to what matters.
    fn answer(tag: &str, zip_url: &str) -> String {
        format!(
            r#"{{"url":"https://api.github.com/repos/Smogi67/mixer/releases/1",
            "tag_name":"{tag}","name":"Mixer {tag}","draft":false,"prerelease":false,
            "body":"What is new:\r\n\r\n- **Settings** page\r\n- Pick the `rendering` engine\r\n* Fixes\r\n- More",
            "assets":[
              {{"name":"mixer-0.11.0-windows.zip.sha256","size":97,
                "browser_download_url":"https://github.com/Smogi67/mixer/releases/download/{tag}/mixer-0.11.0-windows.zip.sha256"}},
              {{"name":"mixer-0.11.0-windows.zip","size":3212544,"uploader":{{"login":"github-actions[bot]"}},
                "browser_download_url":"{zip_url}"}}
            ]}}"#
        )
    }

    #[test]
    fn reads_the_newest_release() {
        let url =
            "https://github.com/Smogi67/mixer/releases/download/v0.11.0/mixer-0.11.0-windows.zip";
        let release = parse_release(&answer("v0.11.0", url)).unwrap();
        assert_eq!(release.version, Version(0, 11, 0));
        assert_eq!(release.zip.size, 3_212_544);
        assert_eq!(release.zip.url, url);
        assert!(release.fingerprint.name.ends_with(".sha256"));
        assert_eq!(
            notes_summary(&release.notes),
            ["Settings page", "Pick the rendering engine", "Fixes"]
        );
        assert_eq!(notes_summary("Just one line.\n"), ["Just one line."]);
        assert!(notes_summary("").is_empty());

        // Downloads from anywhere but GitHub are refused.
        assert!(parse_release(&answer("v0.11.0", "https://example.com/x.zip")).is_err());
        assert!(parse_release(&answer("nightly", url)).is_err());
        assert!(parse_release("{\"message\":\"Not Found\"}").is_err());
        assert_eq!(
            split_url(url),
            Some((
                "github.com",
                "/Smogi67/mixer/releases/download/v0.11.0/mixer-0.11.0-windows.zip"
            ))
        );
    }

    fn release_for(zip: &[u8], version: Version) -> Release {
        let asset = |name: &str, size| Asset {
            name: name.to_string(),
            url: format!("https://github.com/{name}"),
            size,
        };
        Release {
            version,
            notes: String::new(),
            zip: asset("mixer-windows.zip", zip.len() as u64),
            fingerprint: asset("mixer-windows.zip.sha256", 90),
        }
    }

    #[test]
    fn downloads_are_checked_before_use() {
        let zip = make_zip(&[("mixer.exe", b"new app")]);
        let release = release_for(&zip, Version(0, 11, 0));
        let good = format!("{}  mixer-windows.zip\n", sha256_hex(&zip));
        check_download(&zip, &release, &good).unwrap();
        let mut changed = zip.clone();
        changed[3] ^= 1;
        assert!(check_download(&changed, &release, &good).is_err());
        assert!(check_download(&zip[1..], &release, &good).is_err());
        assert!(check_download(&zip, &release, "not a fingerprint").is_err());
        assert_eq!(
            read_fingerprint(&good.to_uppercase()),
            Some(sha256_hex(&zip))
        );
    }

    fn release_zip(version: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
        let mut files: Vec<(&str, &[u8])> = vec![
            ("mixer.exe", b"new app"),
            ("mixer-launcher.exe", b"new launcher"),
            ("README.txt", b"read me"),
            ("licenses\\Inter-LICENSE.txt", b"license"),
            ("version.txt", version.as_bytes()),
        ];
        files.extend_from_slice(extra);
        make_zip(&files)
    }

    #[test]
    fn only_mixers_own_files_are_written() {
        let zip = release_zip("0.11.0\n", &[]);
        let files = unpack(&zip, &release_for(&zip, Version(0, 11, 0))).unwrap();
        assert!(files.iter().any(|f| f.path == "licenses/Inter-LICENSE.txt"));
        // A version file that disagrees with the release.
        assert!(unpack(&zip, &release_for(&zip, Version(0, 12, 0))).is_err());
        // Paths that would leave the folder, or touch other files.
        for bad in [
            "../evil.exe",
            "licenses/../../evil",
            "C:/Windows/evil.dll",
            "sub/deep/file",
            "mixer.exe.new",
            "mixer-launcher.old.exe",
            "/root",
        ] {
            let zip = release_zip("0.11.0", &[(bad, b"x")]);
            assert!(
                unpack(&zip, &release_for(&zip, Version(0, 11, 0))).is_err(),
                "accepted {bad}"
            );
        }
        let incomplete = make_zip(&[("version.txt", b"0.11.0"), ("mixer.exe", b"app")]);
        let error = unpack(&incomplete, &release_for(&incomplete, Version(0, 11, 0))).unwrap_err();
        assert!(error.contains("mixer-launcher.exe"), "{error}");
    }

    #[test]
    fn applying_replaces_the_files_and_moves_the_launcher_aside() {
        let folder = temp_folder("apply");
        std::fs::write(folder.join("mixer.exe"), b"old app").unwrap();
        std::fs::write(folder.join("mixer-launcher.exe"), b"old launcher").unwrap();
        std::fs::write(folder.join("SDL2.dll"), b"sdl").unwrap();
        std::fs::write(folder.join("version.txt"), b"0.10.0").unwrap();
        assert_eq!(installed_version(&folder), Version(0, 10, 0));

        let zip = release_zip("0.11.0", &[]);
        let files = unpack(&zip, &release_for(&zip, Version(0, 11, 0))).unwrap();
        let mut log = Vec::new();
        apply(&folder, &files, &mut |line| log.push(line)).unwrap();
        let read = |name: &str| std::fs::read(folder.join(name)).unwrap();
        assert_eq!(read("mixer.exe"), b"new app");
        assert_eq!(read("mixer-launcher.exe"), b"new launcher");
        assert_eq!(read(OLD_LAUNCHER), b"old launcher");
        assert_eq!(read("licenses/Inter-LICENSE.txt"), b"license");
        assert_eq!(read("SDL2.dll"), b"sdl", "files not in the release stay");
        assert_eq!(installed_version(&folder), Version(0, 11, 0));
        assert!(log.iter().any(|l| l.contains("replaced mixer.exe")));
        assert_eq!(log.iter().position(|l| l.contains("mixer.exe")), Some(1));

        clean_up_after_update(&folder);
        assert!(!folder.join(OLD_LAUNCHER).exists());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let folder = temp_folder("blocked");
        std::fs::write(folder.join("mixer.exe"), b"old app").unwrap();
        // A folder where the new launcher should go makes its rename fail; mixer.exe
        // comes first and succeeds, so the failure is reported as unfinished.
        let zip = release_zip("0.11.0", &[]);
        let files = unpack(&zip, &release_for(&zip, Version(0, 11, 0))).unwrap();
        // Make mixer.exe itself impossible to replace: a folder in its way.
        std::fs::remove_file(folder.join("mixer.exe")).unwrap();
        std::fs::create_dir_all(folder.join("mixer.exe").join("inside")).unwrap();
        let error = apply(&folder, &files, &mut |_| {}).unwrap_err();
        assert!(error.contains("Nothing was changed"), "{error}");
        let left: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, ["mixer.exe"], "no new files left behind");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
