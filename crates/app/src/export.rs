//! Saving OBJ files, and what to tell the user first.

use std::path::{Path, PathBuf};

use modeler_mesh::{Mesh, ROBLOX_MAX_TRIANGLES};

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

/// Writes `<folder>/<name>.obj`, replacing any earlier file of that name. The file
/// appears only once it is complete, so a failure cannot leave half a model behind.
pub fn write_obj(mesh: &Mesh, folder: &Path, name: &str, scale: f32) -> Result<PathBuf, String> {
    let name = clean_file_name(name);
    let text = mesh
        .to_obj(&name, scale)
        .map_err(|error| error.to_string())?;
    std::fs::create_dir_all(folder)
        .map_err(|error| format!("Could not create {}: {error}", folder.display()))?;
    let path = folder.join(format!("{name}.obj"));
    let temporary = folder.join(format!("{name}.obj.tmp"));
    std::fs::write(&temporary, text)
        .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, &path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("Could not save {}: {error}", path.display())
    })?;
    Ok(path)
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

        let path = write_obj(&mesh, &folder, "Cube/1", 1.0).unwrap();
        assert_eq!(path.file_name().unwrap(), "Cube_1.obj");
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("o Cube_1"));
        assert_eq!(first.lines().filter(|l| l.starts_with("f ")).count(), 12);

        mesh.add_primitive(Primitive::Cube { size: 2.0 }, [5.0, 0.0, 0.0])
            .unwrap();
        write_obj(&mesh, &folder, "Cube/1", 1.0).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(second.lines().filter(|l| l.starts_with("f ")).count(), 24);

        let files = std::fs::read_dir(&folder).unwrap().count();
        assert_eq!(files, 1, "the temporary file must be gone");
        assert!(write_obj(&mesh, &folder, "bad", 0.0).is_err());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_be_created_reports_an_error() {
        let file = std::env::temp_dir().join(format!("modeler-export-file-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let mesh = Mesh::new();
        let error = write_obj(&mesh, &file.join("inside"), "a", 1.0).unwrap_err();
        assert!(error.contains("Could not create"), "{error}");
        std::fs::remove_file(&file).unwrap();
    }
}
