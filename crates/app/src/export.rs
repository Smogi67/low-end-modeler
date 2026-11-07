//! Saving and opening files: projects, OBJ and glTF exports, and what to tell the
//! user first.

use std::path::{Path, PathBuf};

use modeler_mesh::{FileKind, LoadReport, Mesh, ROBLOX_MAX_TRIANGLES};

/// The project file's extension.
pub const PROJECT_EXTENSION: &str = "lem";

/// Which format the Export button writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportFormat {
    #[default]
    Obj,
    Glb,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Obj => "obj",
            ExportFormat::Glb => "glb",
        }
    }
}

/// A safe file name: Windows's forbidden characters become `_`, as do leading and
/// trailing dots and spaces, and an empty or reserved name gets a default.
pub fn clean_file_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ').to_string();
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if cleaned.is_empty() || reserved {
        "model".to_string()
    } else {
        cleaned
    }
}

/// Things to warn about before exporting to Roblox.
pub fn warnings(triangles: usize, open_edges: usize) -> Vec<String> {
    let mut out = Vec::new();
    if triangles > ROBLOX_MAX_TRIANGLES {
        out.push(format!(
            "Over Roblox's limit of {ROBLOX_MAX_TRIANGLES} triangles per mesh."
        ));
    }
    if open_edges > 0 {
        out.push(format!(
            "{open_edges} open edges. Roblox asks for closed shapes with thickness."
        ));
    }
    out
}

/// Writes a file so that it appears only once complete: a failure part-way cannot
/// leave half a file behind, or damage an older file of the same name.
pub fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("Could not create {}: {error}", folder.display()))?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("Could not save {}: {error}", path.display())
    })
}

/// Writes `<folder>/<name>.obj` or `.glb`, replacing any earlier file of that name.
pub fn write_export(
    mesh: &Mesh,
    folder: &Path,
    name: &str,
    scale: f32,
    format: ExportFormat,
) -> Result<PathBuf, String> {
    let name = clean_file_name(name);
    let bytes = match format {
        ExportFormat::Obj => mesh.to_obj(&name, scale).map(String::into_bytes),
        ExportFormat::Glb => mesh.to_glb(scale),
    }
    .map_err(|error| error.to_string())?;
    let path = folder.join(format!("{name}.{}", format.extension()));
    write_whole(&path, &bytes)?;
    Ok(path)
}

/// Saves a project file.
pub fn save_project(mesh: &Mesh, path: &Path) -> Result<(), String> {
    write_whole(path, mesh.to_project().as_bytes())
}

/// Opens a project, or imports an OBJ file, going by the file's extension.
pub fn open_file(path: &Path) -> Result<(Mesh, LoadReport, FileKind), String> {
    let kind = match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("obj") => FileKind::Obj,
        _ => FileKind::Project,
    };
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let (mesh, report) = Mesh::from_text(&text, kind).map_err(|error| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        format!("Could not read {name}: {error}")
    })?;
    Ok((mesh, report, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use modeler_mesh::Primitive;

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(clean_file_name("Sword"), "Sword");
        assert_eq!(clean_file_name("  my model  "), "my model");
        assert_eq!(
            clean_file_name("a/b\\c:d*e?f\"g<h>i|j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(clean_file_name("..\\..\\evil"), "_.._evil");
        assert_eq!(clean_file_name("name."), "name");
        assert_eq!(clean_file_name(""), "model");
        assert_eq!(clean_file_name("..."), "model");
        assert_eq!(clean_file_name("con"), "model");
        assert_eq!(clean_file_name("LPT1"), "model");
        assert_eq!(clean_file_name("nul.txt"), "model");
        assert_eq!(clean_file_name("console"), "console");
        assert_eq!(clean_file_name("Comet"), "Comet");
    }

    #[test]
    fn warnings_appear_only_when_needed() {
        assert!(warnings(ROBLOX_MAX_TRIANGLES, 0).is_empty());
        let over = warnings(ROBLOX_MAX_TRIANGLES + 1, 0);
        assert_eq!(over.len(), 1);
        assert!(over[0].contains("20000"));
        let open = warnings(10, 4);
        assert_eq!(open.len(), 1);
        assert!(open[0].starts_with("4 open edges"));
        assert_eq!(warnings(ROBLOX_MAX_TRIANGLES + 1, 1).len(), 2);
    }

    #[test]
    fn writes_replaces_and_leaves_no_temporary_file() {
        let folder = std::env::temp_dir().join(format!("modeler-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let mut mesh = Mesh::new();
        mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
            .unwrap();

        let path = write_export(&mesh, &folder, "Cube/1", 1.0, ExportFormat::Obj).unwrap();
        assert_eq!(path.file_name().unwrap(), "Cube_1.obj");
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("o Cube_1"));
        assert_eq!(first.lines().filter(|l| l.starts_with("f ")).count(), 12);

        mesh.add_primitive(Primitive::Cube { size: 2.0 }, [5.0, 0.0, 0.0])
            .unwrap();
        write_export(&mesh, &folder, "Cube/1", 1.0, ExportFormat::Obj).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(second.lines().filter(|l| l.starts_with("f ")).count(), 24);

        let files = std::fs::read_dir(&folder).unwrap().count();
        assert_eq!(files, 1, "the temporary file must be gone");
        assert!(write_export(&mesh, &folder, "bad", 0.0, ExportFormat::Obj).is_err());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn glb_export_and_project_save_and_open() {
        let folder = std::env::temp_dir().join(format!("modeler-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let mut mesh = Mesh::new();
        mesh.add_primitive(
            Primitive::Cylinder {
                radius: 1.0,
                height: 2.0,
                segments: 7,
            },
            [0.0; 3],
        )
        .unwrap();
        let glb = write_export(&mesh, &folder, "Tube", 1.0, ExportFormat::Glb).unwrap();
        assert_eq!(glb.file_name().unwrap(), "Tube.glb");
        assert_eq!(&std::fs::read(&glb).unwrap()[..4], b"glTF");

        let project = folder.join("tube.lem");
        save_project(&mesh, &project).unwrap();
        let (back, report, kind) = open_file(&project).unwrap();
        assert_eq!(kind, FileKind::Project);
        assert_eq!(report.skipped_faces, 0);
        assert_eq!(back.to_project(), mesh.to_project());

        // An OBJ export opens as an import, by its extension, whatever its case.
        let obj = write_export(&mesh, &folder, "Tube", 1.0, ExportFormat::Obj).unwrap();
        let upper = folder.join("TUBE.OBJ");
        std::fs::rename(&obj, &upper).unwrap();
        let (imported, _, kind) = open_file(&upper).unwrap();
        assert_eq!(kind, FileKind::Obj);
        assert_eq!(imported.triangle_count(), mesh.triangle_count());

        // Missing and broken files explain themselves.
        assert!(
            open_file(&folder.join("nope.lem"))
                .unwrap_err()
                .contains("Could not open")
        );
        std::fs::write(folder.join("bad.lem"), "hello").unwrap();
        let error = open_file(&folder.join("bad.lem")).unwrap_err();
        assert!(error.contains("Could not read bad.lem"), "{error}");
        let leftovers = std::fs::read_dir(&folder)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_be_created_reports_an_error() {
        let file = std::env::temp_dir().join(format!("modeler-export-file-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let mesh = Mesh::new();
        let error =
            write_export(&mesh, &file.join("inside"), "a", 1.0, ExportFormat::Obj).unwrap_err();
        assert!(error.contains("Could not create"), "{error}");
        std::fs::remove_file(&file).unwrap();
    }
}
