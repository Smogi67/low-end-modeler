//! The launcher's jobs as lists of steps, and running them. The download, the DLL
//! check and starting Mixer go through `Io`, which the window provides with Windows
//! itself and the tests with stand-ins.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::core::{self, Event, Found, SDL2};
use crate::update::{self, Release};

/// One part of a job.
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    /// Download, check and put in place a newer Mixer.
    Update(Box<Release>),
    /// Download, check and install SDL2, even if it is there.
    InstallSdl,
    /// SDL2 only if it is missing or different.
    EnsureSdl,
    Start,
}

/// A job: a title, its tasks, and the steps people see.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub title: String,
    pub tasks: Vec<Task>,
    pub steps: Vec<String>,
    /// Steps before this one may be cancelled; after it, files are being replaced.
    pub cancellable_until: usize,
}

fn steps_of(task: &Task) -> Vec<String> {
    match task {
        Task::Update(release) => vec![
            format!(
                "Download Mixer {} ({})",
                release.version,
                core::size_text(release.zip.size)
            ),
            "Check the download's fingerprint".into(),
            "Replace Mixer (your projects are not touched)".into(),
        ],
        Task::InstallSdl | Task::EnsureSdl => vec![
            format!(
                "Download SDL2 {} ({})",
                SDL2.version,
                core::size_text(SDL2.size)
            ),
            "Check and install SDL2".into(),
            "Check that Windows can load it".into(),
        ],
        Task::Start => vec!["Start Mixer".into()],
    }
}

impl Plan {
    fn new(title: String, tasks: Vec<Task>) -> Self {
        let mut steps = Vec::new();
        let mut cancellable_until = 0;
        for task in &tasks {
            let mine = steps_of(task);
            if let Task::Update(_) = task {
                // Up to and including the fingerprint check.
                cancellable_until = steps.len() + 2;
            } else if cancellable_until == 0 {
                cancellable_until = steps.len() + 1;
            }
            steps.extend(mine);
        }
        Self {
            title,
            tasks,
            steps,
            cancellable_until,
        }
    }

    /// Update Mixer, get SDL2 too if it is missing, then start Mixer.
    pub fn update(release: Release, sdl: Found, start: bool) -> Self {
        let title = format!("Updating to Mixer {}", release.version);
        let mut tasks = vec![Task::Update(Box::new(release))];
        if sdl != Found::Installed {
            tasks.push(Task::EnsureSdl);
        }
        if start {
            tasks.push(Task::Start);
        }
        Self::new(title, tasks)
    }

    /// The first start: SDL2, then Mixer.
    pub fn install() -> Self {
        Self::new(
            "Getting Mixer ready".into(),
            vec![Task::InstallSdl, Task::Start],
        )
    }

    /// Repair install: SDL2 again.
    pub fn repair() -> Self {
        Self::new("Repairing the install".into(), vec![Task::InstallSdl])
    }

    /// Whether the job ends by starting Mixer.
    pub fn starts_mixer(&self) -> bool {
        self.tasks.last() == Some(&Task::Start)
    }
}

/// What a job needs from the system.
pub trait Io {
    /// Downloads an https address. `progress` gets bytes so far and the total, and
    /// returns false to stop.
    fn get(
        &mut self,
        url: &str,
        progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<Vec<u8>, String>;
    /// Whether Windows can load this DLL.
    fn loads(&mut self, dll: &Path) -> bool;
    fn start_mixer(&mut self, folder: &Path) -> Result<(), String>;
}

/// Said when a job was cancelled, so the window can go back instead of showing it.
pub const CANCELLED: &str = "Cancelled.";

/// Runs a job's tasks in order. Sends Step for each step as it starts.
pub fn run(
    plan: &Plan,
    folder: &Path,
    io: &mut dyn Io,
    send: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut step = 0;
    send(Event::Line(format!("Folder: {}", folder.display())));
    if !core::can_write(folder) {
        return Err(
            "The launcher cannot write into this folder. Move the Mixer folder somewhere \
             like Documents or the desktop, and start the launcher from there."
                .into(),
        );
    }
    for task in &plan.tasks {
        match task {
            Task::Update(release) => {
                update_task(release, folder, io, send, cancel, step)?;
                step += 3;
            }
            Task::InstallSdl | Task::EnsureSdl => {
                sdl_task(*task == Task::EnsureSdl, folder, io, send, cancel, step)?;
                step += 3;
            }
            Task::Start => {
                send(Event::Step(step));
                send(Event::Line("Starting Mixer".into()));
                io.start_mixer(folder)?;
                step += 1;
            }
        }
    }
    send(Event::Step(step));
    Ok(())
}

/// Downloads with a progress line for every tenth.
fn download(
    io: &mut dyn Io,
    url: &str,
    send: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    send(Event::Line(format!("Downloading {url}")));
    let mut last_tenth = u64::MAX;
    let bytes = io.get(url, &mut |done, total| {
        let total = total.max(done).max(1);
        send(Event::Progress(
            done as f32 / total as f32,
            format!("{} of {}", core::size_text(done), core::size_text(total)),
        ));
        let tenth = done * 10 / total;
        if tenth != last_tenth {
            last_tenth = tenth;
            send(Event::Line(format!(
                "  {:>3}%  {}",
                tenth * 10,
                core::size_text(done)
            )));
        }
        !cancel.load(Ordering::Relaxed)
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    bytes
}

fn update_task(
    release: &Release,
    folder: &Path,
    io: &mut dyn Io,
    send: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
    first: usize,
) -> Result<(), String> {
    send(Event::Step(first));
    send(Event::Line(format!("Newest release: {}", release.version)));
    let fingerprint = io.get(&release.fingerprint.url, &mut |_, _| true)?;
    let zip = download(io, &release.zip.url, send, cancel)?;
    send(Event::Line(format!(
        "  downloaded {}",
        core::size_text(zip.len() as u64)
    )));

    send(Event::Step(first + 1));
    update::check_download(&zip, release, &String::from_utf8_lossy(&fingerprint))?;
    send(Event::Line(format!(
        "  fingerprint matches {}",
        &core::sha256_hex(&zip)[..16]
    )));
    let files = update::unpack(&zip, release)?;
    send(Event::Line(format!("  {} files inside", files.len())));
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }

    send(Event::Step(first + 2));
    update::apply(folder, &files, &mut |line| send(Event::Line(line)))?;
    send(Event::Line(format!(
        "Mixer is now version {}",
        update::installed_version(folder)
    )));
    Ok(())
}

fn sdl_task(
    only_if_needed: bool,
    folder: &Path,
    io: &mut dyn Io,
    send: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
    first: usize,
) -> Result<(), String> {
    send(Event::Step(first));
    let found = core::find(&SDL2, folder);
    if only_if_needed && found == Found::Installed {
        send(Event::Line(format!(
            "SDL2 {} is already installed, and matches its fingerprint. Nothing to download.",
            SDL2.version
        )));
    } else {
        if found == Found::Different {
            send(Event::Line(
                "A different SDL2.dll is here, and will be replaced.".into(),
            ));
        }
        let zip = download(io, &SDL2.url(), send, cancel)?;
        send(Event::Step(first + 1));
        core::install(&SDL2, &zip, folder, send)?;
    }
    send(Event::Step(first + 2));
    send(Event::Line(
        "Checking that Windows can load SDL2.dll".into(),
    ));
    if !io.loads(&folder.join("SDL2.dll")) {
        return Err("SDL2.dll is in place, but Windows cannot load it.".into());
    }
    send(Event::Line("  it loads".into()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tests::{make_zip, temp_folder};
    use crate::update::{Asset, Version};

    /// Serves files by address, and remembers starts.
    struct Fake {
        files: Vec<(String, Vec<u8>)>,
        started: bool,
    }

    impl Io for Fake {
        fn get(
            &mut self,
            url: &str,
            progress: &mut dyn FnMut(u64, u64) -> bool,
        ) -> Result<Vec<u8>, String> {
            let bytes = self
                .files
                .iter()
                .find(|(u, _)| u == url)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| format!("no file at {url}"))?;
            let total = bytes.len() as u64;
            for done in [total / 2, total] {
                if !progress(done, total) {
                    return Err(CANCELLED.into());
                }
            }
            Ok(bytes)
        }

        fn loads(&mut self, dll: &Path) -> bool {
            dll.exists()
        }

        fn start_mixer(&mut self, _: &Path) -> Result<(), String> {
            self.started = true;
            Ok(())
        }
    }

    fn release(zip: &[u8]) -> Release {
        Release {
            version: Version(0, 11, 0),
            notes: String::new(),
            zip: Asset {
                name: "mixer-0.11.0-windows.zip".into(),
                url: "https://github.com/zip".into(),
                size: zip.len() as u64,
            },
            fingerprint: Asset {
                name: "mixer-0.11.0-windows.zip.sha256".into(),
                url: "https://github.com/sha".into(),
                size: 90,
            },
        }
    }

    #[test]
    fn an_update_runs_its_steps_in_order_and_starts_mixer() {
        let folder = temp_folder("job-update");
        std::fs::write(folder.join("mixer.exe"), b"old").unwrap();
        // SDL2 counts as installed only with the real fingerprint, so it is "missing"
        // here and the plan includes it; skip that by asking for Installed.
        let zip = make_zip(&[
            ("mixer.exe", b"new"),
            ("mixer-launcher.exe", b"launcher"),
            ("version.txt", b"0.11.0"),
        ]);
        let release = release(&zip);
        let mut fake = Fake {
            files: vec![
                ("https://github.com/zip".into(), zip.clone()),
                (
                    "https://github.com/sha".into(),
                    format!("{}  x.zip\n", core::sha256_hex(&zip)).into_bytes(),
                ),
            ],
            started: false,
        };
        let plan = Plan::update(release, Found::Installed, true);
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(plan.cancellable_until, 2);
        assert!(plan.starts_mixer());
        let mut steps = Vec::new();
        let mut lines = Vec::new();
        run(
            &plan,
            &folder,
            &mut fake,
            &mut |e| match e {
                Event::Step(s) => steps.push(s),
                Event::Line(l) => lines.push(l),
                Event::Progress(..) => {}
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(steps, [0, 1, 2, 3, 4]);
        assert!(fake.started);
        assert_eq!(std::fs::read(folder.join("mixer.exe")).unwrap(), b"new");
        assert!(lines.iter().any(|l| l.contains("now version 0.11.0")));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_bad_fingerprint_or_a_cancel_changes_nothing() {
        let folder = temp_folder("job-bad");
        std::fs::write(folder.join("mixer.exe"), b"old").unwrap();
        let zip = make_zip(&[
            ("mixer.exe", b"new"),
            ("mixer-launcher.exe", b"launcher"),
            ("version.txt", b"0.11.0"),
        ]);
        let mut fake = Fake {
            files: vec![
                ("https://github.com/zip".into(), zip.clone()),
                ("https://github.com/sha".into(), "0".repeat(64).into_bytes()),
            ],
            started: false,
        };
        let plan = Plan::update(release(&zip), Found::Installed, true);
        let error = run(
            &plan,
            &folder,
            &mut fake,
            &mut |_| {},
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.contains("fingerprint"), "{error}");
        let error = run(
            &plan,
            &folder,
            &mut fake,
            &mut |_| {},
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(error, CANCELLED);
        assert!(!fake.started);
        assert_eq!(std::fs::read(folder.join("mixer.exe")).unwrap(), b"old");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_install_and_repair_plans_read_well() {
        let install = Plan::install();
        assert_eq!(install.steps.len(), 4);
        assert_eq!(install.cancellable_until, 1);
        assert!(install.starts_mixer());
        let repair = Plan::repair();
        assert!(!repair.starts_mixer());
        assert!(repair.steps[0].contains("SDL2"));
        let with_sdl = Plan::update(release(b"zip"), Found::Missing, true);
        assert_eq!(with_sdl.steps.len(), 7);
        assert_eq!(with_sdl.cancellable_until, 2);
    }
}
