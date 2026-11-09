//! Feature search, as Blender's F3 menu: type part of a name, pick a command, and it
//! runs as if its button or key had been used (Stage 10 step 5).

use mixer_mesh::BooleanOp;

use crate::theme::icon;
use crate::transform::Kind;
use crate::ui::{Action, ModeChoice, ModifierKind, Shape, ViewPreset};

/// One thing the search can run.
pub struct Command {
    pub name: &'static str,
    /// Other words people might type for it.
    pub words: &'static str,
    /// Its shortcut, shown beside it.
    pub keys: &'static str,
    pub icon: char,
    pub action: Action,
}

const fn c(
    name: &'static str,
    words: &'static str,
    keys: &'static str,
    icon: char,
    action: Action,
) -> Command {
    Command {
        name,
        words,
        keys,
        icon,
        action,
    }
}

/// Everything the search offers, in the order shown when nothing is typed.
pub fn commands() -> Vec<Command> {
    use Action as A;
    vec![
        c(
            "Move",
            "grab translate position",
            "G",
            icon::MOVE,
            A::Transform(Kind::Move),
        ),
        c(
            "Rotate",
            "turn spin",
            "R",
            icon::ROTATE,
            A::Transform(Kind::Rotate),
        ),
        c(
            "Scale",
            "resize size bigger smaller",
            "S",
            icon::SCALE,
            A::Transform(Kind::Scale),
        ),
        c("Extrude", "pull push out", "E", icon::EXTRUDE, A::Extrude),
        c("Inset faces", "inset", "I", icon::INSET, A::Inset),
        c(
            "Bevel edges",
            "bevel round chamfer corner",
            "Ctrl+B",
            icon::BEVEL,
            A::Bevel,
        ),
        c(
            "Bevel vertices",
            "bevel corner point",
            "Ctrl+Shift+B",
            icon::BEVEL,
            A::BevelVertices,
        ),
        c(
            "Loop cut",
            "cut loop edge ring slice",
            "Ctrl+R",
            icon::LOOP_CUT,
            A::LoopCut,
        ),
        c(
            "Edge slide",
            "slide",
            "",
            icon::SLIDE,
            A::Transform(Kind::Slide),
        ),
        c("Knife", "cut draw line", "K", icon::KNIFE, A::Knife),
        c(
            "Bisect",
            "cut plane slice half",
            "",
            icon::BISECT,
            A::Bisect,
        ),
        c(
            "Bridge edge loops",
            "bridge connect",
            "",
            icon::BRIDGE,
            A::Bridge,
        ),
        c("Fill", "make face close hole", "F", icon::FILL, A::Fill),
        c(
            "Subdivide",
            "split smaller more faces",
            "",
            icon::SUBDIVIDE,
            A::Subdivide,
        ),
        c(
            "Merge",
            "collapse weld join points",
            "M",
            icon::MERGE,
            A::Merge,
        ),
        c(
            "Merge close vertices",
            "remove doubles by distance weld",
            "",
            icon::MERGE_CLOSE,
            A::MergeByDistance,
        ),
        c(
            "Duplicate",
            "copy clone",
            "Shift+D",
            icon::DUPLICATE,
            A::Duplicate,
        ),
        c("Mirror along X", "flip", "", icon::MIRROR, A::Mirror(0)),
        c("Mirror along Y", "flip", "", icon::MIRROR, A::Mirror(1)),
        c("Mirror along Z", "flip", "", icon::MIRROR, A::Mirror(2)),
        c("Delete", "remove erase", "X", icon::DELETE, A::Delete),
        c(
            "Join shapes",
            "combine merge objects",
            "Ctrl+J",
            icon::JOIN,
            A::Join,
        ),
        c(
            "Separate",
            "split loose parts objects",
            "P",
            icon::SEPARATE,
            A::Separate,
        ),
        c(
            "Union",
            "boolean add combine",
            "",
            icon::UNION,
            A::Boolean(BooleanOp::Union),
        ),
        c(
            "Subtract",
            "boolean difference cut hole",
            "",
            icon::SUBTRACT,
            A::Boolean(BooleanOp::Subtract),
        ),
        c(
            "Intersect",
            "boolean common overlap",
            "",
            icon::INTERSECT,
            A::Boolean(BooleanOp::Intersect),
        ),
        c(
            "Mirror modifier",
            "symmetry modifier",
            "",
            icon::MIRROR,
            A::AddModifier(ModifierKind::Mirror),
        ),
        c(
            "Array modifier",
            "repeat copies modifier",
            "",
            icon::DUPLICATE,
            A::AddModifier(ModifierKind::Array),
        ),
        c(
            "Apply modifiers",
            "modifier apply",
            "",
            icon::CHECK,
            A::ApplyModifiers,
        ),
        c(
            "Add cube",
            "new box shape",
            "",
            icon::CUBE,
            A::Add(Shape::Cube),
        ),
        c(
            "Add cone",
            "new shape pyramid",
            "",
            icon::CONE,
            A::Add(Shape::Cone),
        ),
        c(
            "Add cylinder",
            "new shape tube",
            "",
            icon::CYLINDER,
            A::Add(Shape::Cylinder),
        ),
        c(
            "Add UV sphere",
            "new ball shape",
            "",
            icon::UV_SPHERE,
            A::Add(Shape::UvSphere),
        ),
        c(
            "Add icosphere",
            "new ball shape",
            "",
            icon::ICOSPHERE,
            A::Add(Shape::Icosphere),
        ),
        c(
            "Add grid",
            "new plane floor shape",
            "",
            icon::GRID,
            A::Add(Shape::Grid),
        ),
        c(
            "Add circle",
            "new ring shape",
            "",
            icon::CIRCLE,
            A::Add(Shape::Circle),
        ),
        c("Select all", "everything", "A", icon::CLICK, A::SelectAll),
        c(
            "Select none",
            "deselect clear",
            "",
            icon::CLICK,
            A::SelectNone,
        ),
        c(
            "Invert selection",
            "select inverse",
            "Ctrl+I",
            icon::CLICK,
            A::SelectInvert,
        ),
        c(
            "Select linked",
            "connected",
            "Ctrl+L",
            icon::CLICK,
            A::SelectLinked,
        ),
        c("Select more", "grow", "Ctrl +", icon::CLICK, A::SelectMore),
        c(
            "Select less",
            "shrink",
            "Ctrl -",
            icon::CLICK,
            A::SelectLess,
        ),
        c(
            "Select loops",
            "edge loop",
            "",
            icon::LOOP_CUT,
            A::SelectLoops,
        ),
        c(
            "Select rings",
            "edge ring",
            "",
            icon::LOOP_CUT,
            A::SelectRings,
        ),
        c(
            "Vertex mode",
            "points select",
            "1",
            icon::VERTEX,
            A::SetMode(ModeChoice::Vertex),
        ),
        c(
            "Edge mode",
            "lines select",
            "2",
            icon::EDGE,
            A::SetMode(ModeChoice::Edge),
        ),
        c(
            "Face mode",
            "faces select",
            "3",
            icon::FACE,
            A::SetMode(ModeChoice::Face),
        ),
        c(
            "Part mode",
            "object select",
            "4",
            icon::PART,
            A::SetMode(ModeChoice::Part),
        ),
        c("Shade flat", "faceted shading", "", icon::FACE, A::Shade(0)),
        c("Shade smooth", "soft shading", "", icon::FACE, A::Shade(1)),
        c(
            "Shade auto smooth",
            "angle shading",
            "",
            icon::FACE,
            A::Shade(2),
        ),
        c(
            "Mark sharp",
            "crease hard edge",
            "",
            icon::EDGE,
            A::MarkSharp(true),
        ),
        c(
            "Clear sharp",
            "soft edge",
            "",
            icon::EDGE,
            A::MarkSharp(false),
        ),
        c(
            "Switch Edit and View mode",
            "look preview",
            "Tab",
            icon::VIEW,
            A::ToggleView,
        ),
        c(
            "Front view",
            "camera",
            "F1",
            icon::HOME,
            A::View(ViewPreset::Front),
        ),
        c(
            "Back view",
            "camera",
            "Ctrl+F1",
            icon::HOME,
            A::View(ViewPreset::Back),
        ),
        c(
            "Right view",
            "camera side",
            "F3",
            icon::HOME,
            A::View(ViewPreset::Right),
        ),
        c(
            "Left view",
            "camera side",
            "Ctrl+F3",
            icon::HOME,
            A::View(ViewPreset::Left),
        ),
        c(
            "Top view",
            "camera above",
            "F7",
            icon::HOME,
            A::View(ViewPreset::Top),
        ),
        c(
            "Bottom view",
            "camera below",
            "Ctrl+F7",
            icon::HOME,
            A::View(ViewPreset::Bottom),
        ),
        c(
            "Flat view",
            "orthographic perspective",
            "F5",
            icon::ORTHO,
            A::ToggleOrtho,
        ),
        c(
            "Reset view",
            "home camera",
            "Home",
            icon::HOME,
            A::ResetView,
        ),
        c("Undo", "back", "Ctrl+Z", icon::UNDO, A::Undo),
        c("Redo", "again", "Ctrl+Y", icon::REDO, A::Redo),
        c("New project", "file", "Ctrl+N", icon::NEW, A::New),
        c("Open project", "file load", "Ctrl+O", icon::FOLDER, A::Open),
        c("Save", "file", "Ctrl+S", icon::SAVE, A::Save),
        c(
            "Save as",
            "file copy",
            "Ctrl+Shift+S",
            icon::SAVE,
            A::SaveAs,
        ),
        c(
            "Export",
            "obj gltf glb file",
            "Ctrl+E",
            icon::EXPORT,
            A::Export,
        ),
        c(
            "Start window",
            "home recent projects",
            "",
            icon::HOME,
            A::StartWindow,
        ),
        c(
            "Settings",
            "preferences options theme colors engine",
            "Ctrl+,",
            icon::SETTINGS,
            A::Settings,
        ),
    ]
}

/// The commands matching `query`, best first: every typed word must appear in the
/// name or the other words; names starting with what was typed come first.
pub fn matching(query: &str, commands: &[Command]) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<(u8, usize)> = commands
        .iter()
        .enumerate()
        .filter_map(|(i, command)| {
            let name = command.name.to_lowercase();
            let all = format!("{name} {}", command.words);
            if !words.iter().all(|w| all.contains(w)) {
                return None;
            }
            let rank = if query.is_empty() || name.starts_with(&query) {
                0
            } else if words.iter().all(|w| name.contains(w)) {
                1
            } else {
                2
            };
            Some((rank, i))
        })
        .collect();
    found.sort_by_key(|&(rank, i)| (rank, i));
    found.into_iter().map(|(_, i)| i).collect()
}

/// The search window's own state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchState {
    pub query: String,
    /// Which match the arrow keys point at.
    pub selected: usize,
    /// True on the first frame, to put the typing cursor in the field.
    pub focus: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_finds_the_command_and_ranks_names_first() {
        let all = commands();
        let name = |q: &str| matching(q, &all).first().map(|&i| all[i].name);
        assert_eq!(name("bev"), Some("Bevel edges"));
        assert_eq!(name("loop c"), Some("Loop cut"));
        assert_eq!(name("SUBDIV"), Some("Subdivide"));
        // Other words count too.
        assert_eq!(name("grab"), Some("Move"));
        assert_eq!(name("theme"), Some("Settings"));
        assert!(matching("nothing like this", &all).is_empty());
        assert_eq!(matching("", &all).len(), all.len());
        // Every name is there once.
        let mut names: Vec<_> = all.iter().map(|c| c.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
    }
}
