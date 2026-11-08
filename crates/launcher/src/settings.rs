//! The launcher's lines in Documents › Mixer › settings.txt, the file Mixer keeps its
//! own settings in. Lines the launcher does not know are kept as they are, and Mixer
//! keeps the launcher's.

use std::path::{Path, PathBuf};

/// How Mixer draws: OpenGL 2.1 with Mixer's own renderer, or a newer engine through
/// wgpu (Stage 10 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    OpenGl21,
    Vulkan,
    DirectX12,
    OpenGl33,
}

impl Renderer {
    pub const ALL: [Renderer; 4] = [
        Renderer::OpenGl21,
        Renderer::Vulkan,
        Renderer::DirectX12,
        Renderer::OpenGl33,
    ];

    /// Its word in settings.txt.
    pub fn key(self) -> &'static str {
        match self {
            Renderer::OpenGl21 => "opengl2.1",
            Renderer::Vulkan => "vulkan",
            Renderer::DirectX12 => "directx12",
            Renderer::OpenGl33 => "opengl3.3",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Renderer::OpenGl21 => "OpenGL 2.1",
            Renderer::Vulkan => "Vulkan",
            Renderer::DirectX12 => "DirectX 12",
            Renderer::OpenGl33 => "OpenGL 3.3",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Renderer::OpenGl21 => "Works on almost every PC, including old ones.",
            Renderer::Vulkan => "Modern and fast on newer graphics cards.",
            Renderer::DirectX12 => "Windows' own modern engine.",
            Renderer::OpenGl33 => "Newer OpenGL, through the same system as Vulkan.",
        }
    }

    fn from_key(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.key() == word)
    }
}

/// How well an engine works on this PC, as Mixer found out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// A graphics card offers it.
    Yes,
    /// Only a slow software stand-in offers it.
    Software,
    No,
}

/// Documents › Mixer › engines.txt, which `mixer --check-engines` writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Engines {
    /// The Mixer version that checked.
    pub version: String,
    found: Vec<(Renderer, Support)>,
}

impl Engines {
    pub fn parse(text: &str) -> Self {
        let mut engines = Engines::default();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let (Some(key), Some(value)) = (words.next(), words.next()) else {
                continue;
            };
            if key == "version" {
                engines.version = value.to_string();
                continue;
            }
            let support = match value {
                "yes" => Support::Yes,
                "software" => Support::Software,
                "no" => Support::No,
                _ => continue,
            };
            if let Some(renderer) = Renderer::from_key(key) {
                engines.found.retain(|(r, _)| *r != renderer);
                engines.found.push((renderer, support));
            }
        }
        engines
    }

    pub fn load(file: &Path) -> Self {
        Self::parse(&std::fs::read_to_string(file).unwrap_or_default())
    }

    /// `None` until Mixer has checked.
    pub fn support(&self, renderer: Renderer) -> Option<Support> {
        self.found
            .iter()
            .find(|(r, _)| *r == renderer)
            .map(|(_, s)| *s)
    }

    /// The same suggestion Mixer's settings make: DirectX 12 on a graphics card that
    /// offers it, else Vulkan, else OpenGL 2.1.
    pub fn recommended(&self) -> Renderer {
        [Renderer::DirectX12, Renderer::Vulkan]
            .into_iter()
            .find(|&r| self.support(r) == Some(Support::Yes))
            .unwrap_or(Renderer::OpenGl21)
    }
}

/// What the launcher keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LauncherSettings {
    pub renderer: Renderer,
    /// Ask GitHub for a newer version when the launcher opens.
    pub check_updates: bool,
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            renderer: Renderer::OpenGl21,
            check_updates: true,
        }
    }
}

fn value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some(key)).then(|| words.next()).flatten()
    })
}

/// The text with `key` set to `value`: its line replaced, or added at the end.
fn with_value(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut out = String::with_capacity(text.len() + 32);
    for line in text.lines() {
        if line.split_whitespace().next() == Some(key) {
            if !found {
                out.push_str(&format!("{key} {value}\n"));
            }
            found = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !found {
        if out.is_empty() {
            out.push_str("# Mixer settings\n");
        }
        out.push_str(&format!("{key} {value}\n"));
    }
    out
}

impl LauncherSettings {
    pub fn parse(text: &str) -> Self {
        let defaults = Self::default();
        Self {
            renderer: value(text, "renderer")
                .and_then(Renderer::from_key)
                .unwrap_or(defaults.renderer),
            check_updates: value(text, "updates").map_or(defaults.check_updates, |v| v != "off"),
        }
    }

    /// The settings file's text with these settings in it.
    pub fn written_into(&self, text: &str) -> String {
        let text = with_value(text, "renderer", self.renderer.key());
        with_value(
            &text,
            "updates",
            if self.check_updates { "on" } else { "off" },
        )
    }

    pub fn load(file: &Path) -> Self {
        Self::parse(&std::fs::read_to_string(file).unwrap_or_default())
    }

    /// Saves, keeping every other line of the file. Written whole, then renamed.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let old = std::fs::read_to_string(file).unwrap_or_default();
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let temporary = file.with_extension("txt.saving");
        std::fs::write(&temporary, self.written_into(&old)).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, file).map_err(|e| e.to_string())
    }
}

/// Documents › Mixer › settings.txt, given the Documents folder. `MIXER_HOME` moves the
/// Mixer folder, for testing, as it does for Mixer.
pub fn settings_file(documents: Option<PathBuf>) -> PathBuf {
    let mixer = match std::env::var_os("MIXER_HOME") {
        Some(home) => PathBuf::from(home),
        None => documents
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Mixer"),
    };
    mixer.join("settings.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file Mixer writes (see crates/app/src/engines.rs).
    #[test]
    fn engines_are_read_as_mixer_writes_them() {
        let text = "# Which rendering engines this computer offers.\nversion 0.13.0\n\
                    opengl2.1 yes\nvulkan yes NVIDIA GeForce GTX 1050\n\
                    directx12 software Microsoft Basic Render Driver\nopengl3.3 no \n";
        let engines = Engines::parse(text);
        assert_eq!(engines.version, "0.13.0");
        assert_eq!(engines.support(Renderer::Vulkan), Some(Support::Yes));
        assert_eq!(
            engines.support(Renderer::DirectX12),
            Some(Support::Software)
        );
        assert_eq!(engines.support(Renderer::OpenGl33), Some(Support::No));
        assert_eq!(engines.recommended(), Renderer::Vulkan);
        let laptop = Engines::parse("version 0.13.0\nvulkan no\ndirectx12 software x\n");
        assert_eq!(laptop.recommended(), Renderer::OpenGl21);
        assert_eq!(Engines::parse("").support(Renderer::Vulkan), None);
        assert_eq!(Engines::parse("").recommended(), Renderer::OpenGl21);
    }

    #[test]
    fn the_launchers_lines_go_in_without_disturbing_mixers() {
        let mixer = "# Mixer settings\nantialiasing off\n";
        let settings = LauncherSettings {
            check_updates: false,
            ..Default::default()
        };
        let text = settings.written_into(mixer);
        assert_eq!(
            text,
            "# Mixer settings\nantialiasing off\nrenderer opengl2.1\nupdates off\n"
        );
        assert_eq!(LauncherSettings::parse(&text), settings);
        // Changing it again replaces the lines instead of adding more.
        let again = LauncherSettings::default().written_into(&text);
        assert_eq!(again.matches("updates").count(), 1);
        assert!(again.contains("updates on") && again.contains("antialiasing off"));
        assert_eq!(LauncherSettings::parse(""), LauncherSettings::default());
        assert_eq!(
            LauncherSettings::parse("renderer vulkan\n").renderer,
            Renderer::Vulkan
        );
        assert_eq!(
            LauncherSettings::default().written_into(""),
            "# Mixer settings\nrenderer opengl2.1\nupdates on\n"
        );
    }

    #[test]
    fn saving_keeps_the_file_whole() {
        let folder = crate::core::tests::temp_folder("settings");
        let file = folder.join("Mixer").join("settings.txt");
        LauncherSettings::default().save(&file).unwrap();
        std::fs::write(&file, "antialiasing on\nupdates on\n").unwrap();
        LauncherSettings {
            check_updates: false,
            ..Default::default()
        }
        .save(&file)
        .unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("antialiasing on\nupdates off\n"), "{text}");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
