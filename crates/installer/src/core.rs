//! What the setup program installs, and how, without any Windows code, so it is
//! tested on any system. The window and the download are in `gui.rs` and `net.rs`.
//!
//! Spec section 6 sets the rules: list only what the app really needs, use official
//! sources over HTTPS, check every file before installing it, and show every real
//! step. Since 2026-11-07 the app's only download is SDL2.dll (owner's decision). The
//! C runtime is built into the .exe. OpenGL comes with the graphics driver, which no
//! program can install, so setup checks it and gives advice.

use std::path::Path;

use sha2::{Digest, Sha256};

/// A file that comes from a downloaded zip.
pub struct Part {
    /// Its name inside the zip, which is also its name once installed.
    pub name: &'static str,
    /// Its SHA-256 fingerprint, to check it after unzipping and to recognize it
    /// later.
    pub sha256: &'static str,
}

/// Something to download and install next to the app.
pub struct Download {
    pub name: &'static str,
    pub version: &'static str,
    pub purpose: &'static str,
    /// Official HTTPS address, split for WinHTTP.
    pub host: &'static str,
    pub path: &'static str,
    /// Size of the download in bytes, for showing before it starts.
    pub size: u64,
    /// SHA-256 fingerprint of the whole download.
    pub sha256: &'static str,
    pub parts: &'static [Part],
}

impl Download {
    pub fn url(&self) -> String {
        format!("https://{}{}", self.host, self.path)
    }
}

/// SDL2, the official 64-bit Windows build. The app is built against SDL 2.26.4,
/// and later 2.x releases keep working with it. The fingerprints were taken from
/// the file on SDL's GitHub releases page on 2026-11-07.
pub const SDL2: Download = Download {
    name: "SDL2",
    version: "2.32.10",
    purpose: "the window, mouse and keyboard",
    host: "github.com",
    path: "/libsdl-org/SDL/releases/download/release-2.32.10/SDL2-2.32.10-win32-x64.zip",
    size: 608_965,
    sha256: "6cf9706eefd0a4a06dc764007934d428afaf029fabdd408a9e646048c91e18fb",
    parts: &[
        Part {
            name: "SDL2.dll",
            sha256: "b37740a72a7a9706216df9f0134894bb7a850b356fd149398c67d874cbcfacb4",
        },
        // SDL asks for this notice, with its license, to go with the DLL.
        Part {
            name: "README-SDL.txt",
            sha256: "f17d8919136f9627468b4dffbd7bddd188ef2aaa8ed21914d2107d1c759e99d7",
        },
    ],
};

/// The oldest OpenGL the app can draw with.
pub const NEEDED_OPENGL: (u32, u32) = (2, 1);

/// Whether a download is already in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// Every part is there and matches its fingerprint.
    Installed,
    /// Nothing is there yet.
    Missing,
    /// Something is there, but not the expected file. It will be replaced.
    Different,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks what of a download is already in `folder`.
pub fn find(download: &Download, folder: &Path) -> Found {
    let mut any = false;
    let mut all_match = true;
    for part in download.parts {
        match std::fs::read(folder.join(part.name)) {
            Ok(bytes) => {
                any = true;
                all_match &= sha256_hex(&bytes) == part.sha256;
            }
            Err(_) => all_match = false,
        }
    }
    match (any, all_match) {
        (_, true) => Found::Installed,
        (false, _) => Found::Missing,
        (true, false) => Found::Different,
    }
}

/// Reads the version from an OpenGL version string such as "3.1.0 - Build 9.17.10".
pub fn opengl_version(text: &str) -> Option<(u32, u32)> {
    let mut numbers = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty());
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers.next()?.parse().ok()?;
    Some((major, minor))
}

/// Sizes in words people read.
pub fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// A step of the install, for the console and the progress bar.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A line for the console.
    Line(String),
    /// Overall progress, from 0 to 1.
    Progress(f32),
}

/// Checks a downloaded zip, unpacks the parts and writes them into `folder`. Each
/// file is written whole, so a failure cannot leave a broken DLL behind. Returns a
/// message for the person if anything is wrong; nothing is installed in that case,
/// unless the failure came while writing, where earlier parts may already be in
/// place and correct.
pub fn install(
    download: &Download,
    zip: &[u8],
    folder: &Path,
    events: &mut dyn FnMut(Event),
) -> Result<(), String> {
    events(Event::Line(
        "Checking the download's SHA-256 fingerprint...".to_string(),
    ));
    let fingerprint = sha256_hex(zip);
    if fingerprint != download.sha256 {
        return Err(format!(
            "The download does not match its fingerprint, so it was changed or damaged on \
             the way. Nothing was installed.\r\n  expected {}\r\n  received {fingerprint}",
            download.sha256
        ));
    }
    events(Event::Line(format!("  matches {}", short(&fingerprint))));
    events(Event::Progress(0.85));

    let mut unpacked = Vec::with_capacity(download.parts.len());
    for (i, part) in download.parts.iter().enumerate() {
        events(Event::Line(format!("Unzipping {}...", part.name)));
        let bytes = unzip_file(zip, part.name)?;
        let found = sha256_hex(&bytes);
        if found != part.sha256 {
            return Err(format!(
                "{} inside the download does not match its fingerprint. Nothing was installed.",
                part.name
            ));
        }
        events(Event::Line(format!(
            "  {} ({}), fingerprint matches",
            part.name,
            size_text(bytes.len() as u64)
        )));
        events(Event::Progress(
            0.85 + 0.1 * (i + 1) as f32 / download.parts.len() as f32,
        ));
        unpacked.push((part.name, bytes));
    }

    for (name, bytes) in unpacked {
        let target = folder.join(name);
        events(Event::Line(format!("Writing {}...", target.display())));
        let temporary = folder.join(format!("{name}.download"));
        std::fs::write(&temporary, &bytes)
            .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &target).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!(
                "Could not put {name} in place: {error}. If Mixer is open, close it \
                 and try again."
            )
        })?;
    }
    events(Event::Progress(1.0));
    Ok(())
}

fn short(fingerprint: &str) -> String {
    format!(
        "{}...{}",
        &fingerprint[..8],
        &fingerprint[fingerprint.len() - 8..]
    )
}

/// True if a file can be created in `folder`.
pub fn can_write(folder: &Path) -> bool {
    let probe = folder.join("setup-write-test.tmp");
    let ok = std::fs::write(&probe, b"test").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn u32_at(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// Reads one file from a zip archive: stored or deflated, checked against the zip's
/// own CRC-32. Only what the official SDL zip uses is supported: no encryption, no
/// ZIP64.
pub fn unzip_file(zip: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let broken = || "The download is not a readable zip file.".to_string();
    // The end record sits at the end, after a comment of up to 65535 bytes.
    let lowest = zip.len().saturating_sub(22 + 65535);
    let end = (lowest..=zip.len().saturating_sub(22))
        .rev()
        .find(|&i| zip[i..].starts_with(b"PK\x05\x06"))
        .ok_or_else(broken)?;
    let count = u16_at(zip, end + 10).ok_or_else(broken)?;
    let mut at = u32_at(zip, end + 16).ok_or_else(broken)?;
    for _ in 0..count {
        if !zip
            .get(at..)
            .is_some_and(|rest| rest.starts_with(b"PK\x01\x02"))
        {
            return Err(broken());
        }
        let method = u16_at(zip, at + 10).ok_or_else(broken)?;
        let crc = u32_at(zip, at + 16).ok_or_else(broken)? as u32;
        let packed = u32_at(zip, at + 20).ok_or_else(broken)?;
        let size = u32_at(zip, at + 24).ok_or_else(broken)?;
        let name_length = u16_at(zip, at + 28).ok_or_else(broken)?;
        let extra = u16_at(zip, at + 30).ok_or_else(broken)?;
        let comment = u16_at(zip, at + 32).ok_or_else(broken)?;
        let local = u32_at(zip, at + 42).ok_or_else(broken)?;
        let entry = zip.get(at + 46..at + 46 + name_length).ok_or_else(broken)?;
        at += 46 + name_length + extra + comment;
        if entry != name.as_bytes() {
            continue;
        }
        if !zip
            .get(local..)
            .is_some_and(|rest| rest.starts_with(b"PK\x03\x04"))
        {
            return Err(broken());
        }
        let start = local
            + 30
            + u16_at(zip, local + 26).ok_or_else(broken)?
            + u16_at(zip, local + 28).ok_or_else(broken)?;
        let data = zip.get(start..start + packed).ok_or_else(broken)?;
        let bytes = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, size)
                .map_err(|_| broken())?,
            _ => return Err(format!("{name} is packed in a way setup cannot unpack.")),
        };
        if bytes.len() != size || crc32(&bytes) != crc {
            return Err(format!("{name} inside the download is damaged."));
        }
        return Ok(bytes);
    }
    Err(format!("The download does not contain {name}."))
}

/// The CRC-32 that zip files use.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip with these files, the first stored and the rest deflated.
    fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for (i, (name, data)) in files.iter().enumerate() {
            let (method, packed): (u16, Vec<u8>) = if i == 0 {
                (0, data.to_vec())
            } else {
                (8, miniz_oxide::deflate::compress_to_vec(data, 6))
            };
            let local = out.len() as u32;
            let crc = crc32(data);
            let header = |sig: &[u8], central: bool| {
                let mut h = Vec::new();
                h.extend_from_slice(sig);
                if central {
                    h.extend_from_slice(&20u16.to_le_bytes());
                }
                h.extend_from_slice(&20u16.to_le_bytes());
                h.extend_from_slice(&0u16.to_le_bytes());
                h.extend_from_slice(&method.to_le_bytes());
                h.extend_from_slice(&[0; 4]);
                h.extend_from_slice(&crc.to_le_bytes());
                h.extend_from_slice(&(packed.len() as u32).to_le_bytes());
                h.extend_from_slice(&(data.len() as u32).to_le_bytes());
                h.extend_from_slice(&(name.len() as u16).to_le_bytes());
                h.extend_from_slice(&0u16.to_le_bytes());
                if central {
                    // Comment length, disk number and internal attributes, then
                    // external attributes.
                    h.extend_from_slice(&[0; 6]);
                    h.extend_from_slice(&[0; 4]);
                    h.extend_from_slice(&local.to_le_bytes());
                }
                h.extend_from_slice(name.as_bytes());
                h
            };
            out.extend(header(b"PK\x03\x04", false));
            out.extend(&packed);
            directory.extend(header(b"PK\x01\x02", true));
        }
        let start = out.len() as u32;
        out.extend(&directory);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn stored_and_deflated_files_unzip() {
        let big: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let zip = make_zip(&[("a.txt", b"hello"), ("b.bin", &big)]);
        assert_eq!(unzip_file(&zip, "a.txt").unwrap(), b"hello");
        assert_eq!(unzip_file(&zip, "b.bin").unwrap(), big);
        assert!(
            unzip_file(&zip, "c")
                .unwrap_err()
                .contains("does not contain")
        );
        assert!(unzip_file(b"not a zip", "a.txt").is_err());
        // Damage inside a file is caught by its CRC.
        let mut damaged = zip.clone();
        damaged[30 + 5] ^= 1;
        assert!(
            unzip_file(&damaged, "a.txt")
                .unwrap_err()
                .contains("damaged")
        );
    }

    const TEST_PARTS: &[Part] = &[Part {
        name: "Thing.dll",
        sha256: "", // Filled in by the test through a leaked string.
    }];

    fn test_download(zip: &[u8], dll: &[u8]) -> Download {
        let zip_hash: &'static str = Box::leak(sha256_hex(zip).into_boxed_str());
        let dll_hash: &'static str = Box::leak(sha256_hex(dll).into_boxed_str());
        let parts: &'static [Part] = Box::leak(Box::new([Part {
            name: TEST_PARTS[0].name,
            sha256: dll_hash,
        }]));
        Download {
            name: "Thing",
            version: "1",
            purpose: "testing",
            host: "example.com",
            path: "/thing.zip",
            size: zip.len() as u64,
            sha256: zip_hash,
            parts,
        }
    }

    fn temp_folder(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn install_checks_fingerprints_and_writes_the_parts() {
        let dll: Vec<u8> = (0..10_000u32).map(|i| (i * 7 % 256) as u8).collect();
        let zip = make_zip(&[("README.txt", b"read me"), ("Thing.dll", &dll)]);
        let download = test_download(&zip, &dll);
        let folder = temp_folder("install");
        assert!(can_write(&folder));
        assert_eq!(find(&download, &folder), Found::Missing);

        let mut seen = Vec::new();
        install(&download, &zip, &folder, &mut |e| seen.push(e)).unwrap();
        assert_eq!(std::fs::read(folder.join("Thing.dll")).unwrap(), dll);
        assert_eq!(find(&download, &folder), Found::Installed);
        assert!(seen.contains(&Event::Progress(1.0)));
        assert!(
            seen.iter()
                .any(|e| matches!(e, Event::Line(l) if l.contains("matches")))
        );
        let leftovers = std::fs::read_dir(&folder).unwrap().count();
        assert_eq!(leftovers, 1, "only the DLL, no temporary files");

        // A different file in its place is noticed, and replaced by installing again.
        std::fs::write(folder.join("Thing.dll"), b"old").unwrap();
        assert_eq!(find(&download, &folder), Found::Different);
        install(&download, &zip, &folder, &mut |_| {}).unwrap();
        assert_eq!(find(&download, &folder), Found::Installed);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_changed_download_installs_nothing() {
        let dll = b"the real thing".to_vec();
        let zip = make_zip(&[("Thing.dll", &dll)]);
        let download = test_download(&zip, &dll);
        let folder = temp_folder("tampered");
        let mut tampered = zip.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        let error = install(&download, &tampered, &folder, &mut |_| {}).unwrap_err();
        assert!(error.contains("does not match its fingerprint"), "{error}");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);

        // A zip whose own fingerprint matches but holds a different DLL is refused too.
        let other = make_zip(&[("Thing.dll", b"something else")]);
        let mismatched = Download {
            sha256: Box::leak(sha256_hex(&other).into_boxed_str()),
            ..test_download(&zip, &dll)
        };
        let error = install(&mismatched, &other, &folder, &mut |_| {}).unwrap_err();
        assert!(error.contains("Thing.dll inside the download"), "{error}");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn opengl_versions_and_sizes_read_well() {
        assert_eq!(opengl_version("3.1.0 - Build 9.17.10.4459"), Some((3, 1)));
        assert_eq!(opengl_version("2.1 Mesa 25.2.8"), Some((2, 1)));
        assert_eq!(opengl_version("1.1.0"), Some((1, 1)));
        assert_eq!(opengl_version("nothing"), None);
        let enough = |v: (u32, u32)| v >= NEEDED_OPENGL;
        assert!(enough((2, 1)));
        assert!(enough((3, 1)));
        assert!(!enough((1, 1)));
        assert!(!enough((2, 0)));
        assert_eq!(size_text(608_965), "595 KB");
        assert_eq!(size_text(1_586_176), "1.5 MB");
        assert_eq!(size_text(12), "12 bytes");
    }

    /// With the real download at hand (set SDL_ZIP to its path), checks the pinned
    /// fingerprints against it.
    #[test]
    fn the_pinned_sdl_fingerprints_match_the_real_download() {
        let Ok(path) = std::env::var("SDL_ZIP") else {
            return;
        };
        let zip = std::fs::read(path).unwrap();
        assert_eq!(zip.len() as u64, SDL2.size);
        let folder = temp_folder("sdl");
        install(&SDL2, &zip, &folder, &mut |_| {}).unwrap();
        assert_eq!(find(&SDL2, &folder), Found::Installed);
        assert_eq!(
            SDL2.url(),
            "https://github.com/libsdl-org/SDL/releases/download/release-2.32.10/SDL2-2.32.10-win32-x64.zip"
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
