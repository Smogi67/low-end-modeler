//! Which rendering engines this computer offers. Asking the graphics drivers can take a
//! moment, and a bad driver could even crash, so a separate copy of Mixer does it
//! (`mixer --check-engines`) and writes the answer to Documents › Mixer › engines.txt.
//! Mixer's settings and the Mixer Launcher both read that file.

use std::path::PathBuf;

use crate::places::mixer_folder;
use crate::wgpu_renderer::{Engine, Support, check};

/// What engines.txt says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// The Mixer version that wrote it.
    pub version: String,
    /// Each newer engine's support, and the graphics card or driver that offers it.
    pub engines: Vec<(Engine, Support, String)>,
}

impl Report {
    pub fn support(&self, engine: Engine) -> Option<Support> {
        self.engines
            .iter()
            .find(|(e, ..)| *e == engine)
            .map(|(_, support, _)| *support)
    }

    /// The engine to suggest, by its key: DirectX 12 on a graphics card that offers
    /// it (Windows' own, with the best-kept drivers there), else Vulkan, else
    /// OpenGL 2.1, which works almost everywhere. OpenGL 3.3 is never first: where it
    /// works, OpenGL 2.1 does too, and it has been tried for longer.
    pub fn recommended(&self) -> &'static str {
        for engine in [Engine::DirectX12, Engine::Vulkan] {
            if self.support(engine) == Some(Support::Yes) {
                return engine.key();
            }
        }
        "opengl2.1"
    }

    pub fn text(&self) -> String {
        let mut text = String::from(
            "# Which rendering engines this computer offers. Written by mixer --check-engines.\n",
        );
        text.push_str(&format!("version {}\n", self.version));
        text.push_str("opengl2.1 yes\n");
        for (engine, support, name) in &self.engines {
            text.push_str(&format!("{} {} {}\n", engine.key(), support.word(), name));
        }
        text
    }

    pub fn parse(text: &str) -> Self {
        let mut report = Report::default();
        for line in text.lines() {
            let mut words = line.splitn(3, ' ');
            let (Some(key), Some(value)) = (words.next(), words.next()) else {
                continue;
            };
            let rest = words.next().unwrap_or("").trim().to_string();
            if key == "version" {
                report.version = value.to_string();
            } else if let (Some(engine), Some(support)) =
                (Engine::from_key(key), Support::from_word(value))
            {
                report.engines.retain(|(e, ..)| *e != engine);
                report.engines.push((engine, support, rest));
            }
        }
        report
    }
}

fn path() -> PathBuf {
    mixer_folder().join("engines.txt")
}

/// The last report, if there is one.
pub fn read() -> Option<Report> {
    let text = std::fs::read_to_string(path()).ok()?;
    Some(Report::parse(&text))
}

/// Asks every newer engine and writes engines.txt. Run by `mixer --check-engines`.
pub fn check_all() -> Result<Report, String> {
    let mut report = Report {
        version: env!("CARGO_PKG_VERSION").to_string(),
        engines: Vec::new(),
    };
    for engine in Engine::ALL {
        // A driver that panics only fails its own engine.
        let (support, name) = std::panic::catch_unwind(|| check(engine)).unwrap_or((
            Support::No,
            "the driver failed while being asked".to_string(),
        ));
        report.engines.push((engine, support, name));
    }
    let _ = std::fs::create_dir_all(mixer_folder());
    crate::export::write_whole(&path(), report.text().as_bytes())?;
    Ok(report)
}

/// Starts `mixer --check-engines` in the background when this version has not
/// checked yet. The answer shows the next time the settings open.
pub fn check_in_background() {
    let current = env!("CARGO_PKG_VERSION");
    if read().is_some_and(|report| report.version == current) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .arg("--check-engines")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_read_back_and_recommend_sensibly() {
        let report = Report {
            version: "0.13.0".to_string(),
            engines: vec![
                (Engine::Vulkan, Support::Yes, "Some Card 3000".to_string()),
                (
                    Engine::DirectX12,
                    Support::Software,
                    "Microsoft Basic Render Driver".to_string(),
                ),
                (Engine::OpenGl33, Support::No, String::new()),
            ],
        };
        assert_eq!(Report::parse(&report.text()), report);
        // Software DirectX 12 is slow, so a real Vulkan card wins.
        assert_eq!(report.recommended(), "vulkan");
        let mut with_dx12 = report.clone();
        with_dx12.engines[1].1 = Support::Yes;
        assert_eq!(with_dx12.recommended(), "directx12");
        // Nothing newer on a card, as on the owner's laptop: OpenGL 2.1.
        let old = Report::parse("version 0.13.0\nvulkan no\ndirectx12 software x\n");
        assert_eq!(old.recommended(), "opengl2.1");
        assert_eq!(Report::parse("").recommended(), "opengl2.1");
        assert_eq!(old.support(Engine::OpenGl33), None);
    }
}
