//! Live modifiers: mirror and array. A modifier belongs to a part, through the
//! part's group number, and changes how the part looks and exports without changing
//! the faces that are edited. [`Mesh::evaluated`] gives the mesh with every
//! modifier worked in; [`Mesh::apply_modifiers_of_selected`] makes that permanent.
//!
//! Modifiers live in a pool beside the faces, so that undo covers them like any
//! other change.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::error::MeshError;
use crate::id::{FaceId, ModifierId, VertexId};
use crate::mesh::{Look, Mesh};
use crate::selection::SelectMode;

type V3 = [f32; 3];

/// The most copies an array makes.
pub const MAX_ARRAY_COUNT: u32 = 64;
/// The most a merge distance can be.
pub const MAX_MERGE_DISTANCE: f32 = 1.0;
/// The usual merge distance, as in Blender.
pub const DEFAULT_MERGE_DISTANCE: f32 = 0.001;

/// A live change to a part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Modifier {
    /// Adds a mirror image across the world's middle on each chosen axis (X, Y, Z).
    /// With `merge`, points this close to the middle are pulled onto it and joined
    /// with their mirror image, so the halves make one closed shape.
    Mirror {
        /// X, Y and Z: which axes to mirror across.
        axes: [bool; 3],
        /// How close to the middle a point must be to join its image.
        merge: Option<f32>,
    },
    /// Repeats the part `count` times in all. Each copy moves by `offset` times the
    /// part's size along each axis, so (1, 0, 0) puts copies side by side along X.
    /// With `merge`, points of neighboring copies this close together are joined.
    Array {
        /// How many in all, counting the original.
        count: u32,
        /// How far each copy moves, in part sizes along X, Y and Z.
        offset: V3,
        /// How close points of neighboring copies must be to join.
        merge: Option<f32>,
    },
}

impl Modifier {
    /// A mirror across X, merging at the middle.
    pub fn mirror() -> Self {
        Modifier::Mirror {
            axes: [true, false, false],
            merge: Some(DEFAULT_MERGE_DISTANCE),
        }
    }

    /// Two copies side by side along X.
    pub fn array() -> Self {
        Modifier::Array {
            count: 2,
            offset: [1.0, 0.0, 0.0],
            merge: None,
        }
    }

    fn check(&self) -> Result<(), MeshError> {
        let merge_ok = |m: Option<f32>| {
            m.is_none_or(|d| d.is_finite() && (0.0..=MAX_MERGE_DISTANCE).contains(&d))
        };
        let ok = match *self {
            Modifier::Mirror { merge, .. } => merge_ok(merge),
            Modifier::Array {
                count,
                offset,
                merge,
            } => {
                (1..=MAX_ARRAY_COUNT).contains(&count)
                    && offset.iter().all(|c| c.is_finite() && c.abs() <= 1000.0)
                    && merge_ok(merge)
            }
        };
        if ok {
            Ok(())
        } else {
            Err(MeshError::InvalidModifier)
        }
    }
}

/// A modifier and the part it belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModifierEntry {
    /// The group number of the part.
    pub group: u16,
    /// What it does.
    pub modifier: Modifier,
    /// Modifiers of one part run in this order, lowest first.
    pub order: u32,
}

/// A part's faces as plain polygons, while modifiers work on them.
#[derive(Debug, Clone, Default)]
struct Soup {
    points: Vec<V3>,
    /// Corner point numbers, smoothing, and whether the face is an original one
    /// that was selected.
    faces: Vec<(Vec<usize>, Look, bool)>,
}

impl Soup {
    fn bounds(&self) -> (V3, V3) {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in &self.points {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }

    /// Adds a copy changed by `place`, with faces turned around if `flip`.
    fn add_copy(&mut self, place: impl Fn(V3) -> V3, flip: bool, original: &Soup) {
        let base = self.points.len();
        self.points
            .extend(original.points.iter().map(|&p| place(p)));
        for (face, smooth, _) in &original.faces {
            let mut face: Vec<usize> = face.iter().map(|i| i + base).collect();
            if flip {
                face.reverse();
            }
            self.faces.push((face, *smooth, false));
        }
    }

    /// Joins points within `distance`, following chains of close points.
    fn weld(&mut self, distance: f32) {
        let cell = distance.max(1e-6);
        let key = |p: V3| p.map(|c| (c / cell).floor() as i64);
        let mut parent: Vec<usize> = (0..self.points.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        for i in 0..self.points.len() {
            let p = self.points[i];
            let c = key(p);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = cells.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                            for &j in list {
                                let q = self.points[j];
                                let d2 = (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f32>();
                                if d2 <= distance * distance {
                                    let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                                    parent[ri] = rj;
                                }
                            }
                        }
                    }
                }
            }
            cells.entry(c).or_default().push(i);
        }
        for face in &mut self.faces {
            for i in face.0.iter_mut() {
                *i = root(&mut parent, *i);
            }
        }
    }

    /// Drops faces that welding squashed, and copies that landed exactly on another
    /// face facing the same way. Pairs facing each other on the same points are walls
    /// between halves or copies that now touch, so both go.
    fn clean(&mut self) {
        /// The corners starting from the lowest, keeping their direction.
        fn key(face: &[usize]) -> Vec<usize> {
            let start = (0..face.len()).min_by_key(|&i| face[i]).unwrap_or(0);
            face[start..]
                .iter()
                .chain(&face[..start])
                .copied()
                .collect()
        }
        let mut faces = Vec::with_capacity(self.faces.len());
        let mut seen: HashSet<Vec<usize>> = HashSet::new();
        for (mut face, smooth, selected) in std::mem::take(&mut self.faces) {
            face.dedup();
            while face.len() > 1 && face.first() == face.last() {
                face.pop();
            }
            let unique: HashSet<usize> = face.iter().copied().collect();
            if face.len() >= 3 && unique.len() == face.len() && seen.insert(key(&face)) {
                faces.push((face, smooth, selected));
            }
        }
        faces.retain(|(face, _, _)| {
            let reversed: Vec<usize> = face.iter().rev().copied().collect();
            !seen.contains(&key(&reversed))
        });
        self.faces = faces;
    }

    fn run(&mut self, modifier: &Modifier) {
        match *modifier {
            Modifier::Mirror { axes, merge } => {
                for (axis, &on) in axes.iter().enumerate() {
                    if !on {
                        continue;
                    }
                    if let Some(d) = merge {
                        // Onto the middle, so that a point and its image are the same.
                        for p in &mut self.points {
                            if p[axis].abs() <= d {
                                p[axis] = 0.0;
                            }
                        }
                    }
                    let original = self.clone();
                    self.add_copy(
                        |mut p| {
                            p[axis] = -p[axis];
                            p
                        },
                        true,
                        &original,
                    );
                    if merge.is_some() {
                        // Exactly equal now, so the smallest distance finds them.
                        self.weld(0.0);
                    }
                    self.clean();
                }
            }
            Modifier::Array {
                count,
                offset,
                merge,
            } => {
                let (lo, hi) = self.bounds();
                let step: V3 = std::array::from_fn(|k| {
                    let size = if hi[k] >= lo[k] { hi[k] - lo[k] } else { 0.0 };
                    offset[k] * size
                });
                let original = self.clone();
                for i in 1..count {
                    let shift: V3 = std::array::from_fn(|k| step[k] * i as f32);
                    self.add_copy(
                        |p| [p[0] + shift[0], p[1] + shift[1], p[2] + shift[2]],
                        false,
                        &original,
                    );
                }
                if let Some(d) = merge {
                    self.weld(d);
                }
                self.clean();
            }
        }
    }
}

impl Mesh {
    /// Every modifier, in no particular order.
    pub fn modifiers(&self) -> impl Iterator<Item = (ModifierId, &ModifierEntry)> + '_ {
        self.modifiers.iter()
    }

    /// True if any part has a modifier.
    pub fn has_modifiers(&self) -> bool {
        self.modifiers.live_count() > 0
    }

    /// A part's modifiers, in the order they run.
    pub fn modifiers_of_group(&self, group: u16) -> Vec<(ModifierId, Modifier)> {
        let mut list: Vec<(u32, ModifierId, Modifier)> = self
            .modifiers
            .iter()
            .filter(|(_, m)| m.group == group && group != 0)
            .map(|(id, m)| (m.order, id, m.modifier))
            .collect();
        list.sort_by_key(|(order, _, _)| *order);
        list.into_iter().map(|(_, id, m)| (id, m)).collect()
    }

    /// The one part with something selected, if exactly one has.
    pub fn single_selected_part(&self) -> Option<Vec<FaceId>> {
        let mut parts = self
            .parts()
            .into_iter()
            .filter(|part| part.iter().any(|&f| self.faces[f].selected));
        let part = parts.next()?;
        parts.next().is_none().then_some(part)
    }

    /// The modifiers of the one selected part.
    pub fn modifiers_of_selected(&self) -> Vec<(ModifierId, Modifier)> {
        match self.single_selected_part() {
            Some(part) => match self.modified_group(&part) {
                Some(group) => self.modifiers_of_group(group),
                None => Vec::new(),
            },
            None => Vec::new(),
        }
    }

    /// True if any of these faces belongs to a part with modifiers.
    pub(crate) fn part_has_modifiers(&self, faces: &[FaceId]) -> bool {
        self.modified_group(faces).is_some()
    }

    /// The group of these faces that has modifiers, if any does.
    pub(crate) fn modified_group(&self, faces: &[FaceId]) -> Option<u16> {
        let modified: HashSet<u16> = self.modifiers.iter().map(|(_, m)| m.group).collect();
        faces
            .iter()
            .map(|&f| self.faces[f].group)
            .filter(|g| *g != 0 && modified.contains(g))
            .min()
    }

    /// After an edit, every face of a part with modifiers takes the part's group, so
    /// new faces from tools are included; parts that grew together share one list;
    /// and modifiers whose part is gone are dropped.
    pub(crate) fn tidy_modifier_groups(&mut self) {
        if !self.has_modifiers() {
            return;
        }
        let modified: HashSet<u16> = self.modifiers.iter().map(|(_, m)| m.group).collect();
        for part in self.parts() {
            let mut groups: Vec<u16> = part
                .iter()
                .map(|&f| self.faces[f].group)
                .filter(|g| modified.contains(g))
                .collect();
            groups.sort_unstable();
            groups.dedup();
            let Some(&group) = groups.first() else {
                continue;
            };
            for &face in &part {
                if self.faces[face].group != group {
                    self.faces[face].group = group;
                }
            }
            let moved: Vec<ModifierId> = self
                .modifiers
                .iter()
                .filter(|(_, m)| m.group != group && groups.contains(&m.group))
                .map(|(id, _)| id)
                .collect();
            for id in moved {
                self.modifiers[id].group = group;
            }
        }
        let live: HashSet<u16> = self.faces().map(|(_, f)| f.group).collect();
        let gone: Vec<ModifierId> = self
            .modifiers
            .iter()
            .filter(|(_, m)| !live.contains(&m.group))
            .map(|(id, _)| id)
            .collect();
        for id in gone {
            self.modifiers.remove(id);
        }
    }

    /// Adds a modifier to the one selected part, after any it has. A part that is
    /// not joined to anything gets a group number of its own to hold it.
    pub fn add_modifier_to_selected(
        &mut self,
        modifier: Modifier,
    ) -> Result<ModifierId, MeshError> {
        modifier.check()?;
        let part = self.single_selected_part().ok_or(MeshError::NeedsOnePart)?;
        let first = self.faces[part[0]].group;
        let group = match self.modified_group(&part) {
            Some(group) => group,
            None if first != 0 && part.iter().all(|&f| self.faces[f].group == first) => first,
            None => self.next_group()?,
        };
        for &face in &part {
            if self.faces[face].group != group {
                self.faces[face].group = group;
            }
        }
        let order = self
            .modifiers
            .iter()
            .map(|(_, m)| m.order + 1)
            .max()
            .unwrap_or(0);
        if !self.modifiers.can_allocate(1) {
            return Err(MeshError::IdsExhausted);
        }
        Ok(self.modifiers.insert_with(|_| ModifierEntry {
            group,
            modifier,
            order,
        }))
    }

    /// Adds a modifier for a group as read from a file, after its others.
    pub(crate) fn add_modifier_unchecked(
        &mut self,
        group: u16,
        modifier: Modifier,
    ) -> Result<ModifierId, MeshError> {
        modifier.check()?;
        if group == 0 {
            return Err(MeshError::NeedsOnePart);
        }
        let order = self
            .modifiers
            .iter()
            .map(|(_, m)| m.order + 1)
            .max()
            .unwrap_or(0);
        if !self.modifiers.can_allocate(1) {
            return Err(MeshError::IdsExhausted);
        }
        Ok(self.modifiers.insert_with(|_| ModifierEntry {
            group,
            modifier,
            order,
        }))
    }

    /// Changes a modifier's settings. A mirror cannot become an array.
    pub fn set_modifier(&mut self, id: ModifierId, modifier: Modifier) -> Result<(), MeshError> {
        modifier.check()?;
        let entry = self.modifiers.get(id).ok_or(MeshError::StaleModifier)?;
        if std::mem::discriminant(&entry.modifier) != std::mem::discriminant(&modifier) {
            return Err(MeshError::InvalidModifier);
        }
        self.modifiers[id].modifier = modifier;
        Ok(())
    }

    /// Deletes a modifier. The part goes back to how it is without it.
    pub fn remove_modifier(&mut self, id: ModifierId) -> Result<(), MeshError> {
        self.modifiers.get(id).ok_or(MeshError::StaleModifier)?;
        self.modifiers.remove(id);
        Ok(())
    }

    /// Makes the selected part's modifiers permanent: its faces become what they
    /// showed, and the modifiers go.
    pub fn apply_modifiers_of_selected(&mut self, mode: SelectMode) -> Result<usize, MeshError> {
        let part = self.single_selected_part().ok_or(MeshError::NeedsOnePart)?;
        let group = self.modified_group(&part).ok_or(MeshError::NoModifiers)?;
        let list = self.modifiers_of_group(group);
        if list.is_empty() {
            return Err(MeshError::NoModifiers);
        }
        let mut trial = self.clone();
        trial.bake_group(group, &list, true)?;
        let faces = self.bake_group(group, &list, true)?;
        for (id, _) in list {
            self.modifiers.remove(id);
        }
        self.flush_selection(SelectMode::Face);
        self.flush_selection(mode);
        Ok(faces)
    }

    /// The mesh with every modifier worked in: what is drawn and exported. Borrows
    /// this mesh when there are none.
    pub fn evaluated(&self) -> Cow<'_, Mesh> {
        if !self.has_modifiers() {
            return Cow::Borrowed(self);
        }
        let mut result = self.clone();
        let mut groups: Vec<u16> = self.modifiers.iter().map(|(_, m)| m.group).collect();
        groups.sort_unstable();
        groups.dedup();
        for group in groups {
            let list = self.modifiers_of_group(group);
            // A modifier whose part has gone does nothing.
            let _ = result.bake_group(group, &list, false);
        }
        let ids: Vec<ModifierId> = result.modifiers.iter().map(|(id, _)| id).collect();
        for id in ids {
            result.modifiers.remove(id);
        }
        result.flush_selection(SelectMode::Face);
        Cow::Owned(result)
    }

    /// Replaces a group's faces with what its modifiers make. `select` keeps the
    /// whole result selected, as after applying; otherwise only the original faces
    /// keep their selection, for drawing. Returns how many faces it made.
    fn bake_group(
        &mut self,
        group: u16,
        list: &[(ModifierId, Modifier)],
        select: bool,
    ) -> Result<usize, MeshError> {
        let faces: Vec<FaceId> = self
            .faces()
            .filter(|(_, f)| f.group == group)
            .map(|(id, _)| id)
            .collect();
        if faces.is_empty() {
            return Ok(0);
        }
        // The part as a soup of numbered points.
        let mut soup = Soup::default();
        let mut numbers: HashMap<VertexId, usize> = HashMap::new();
        for &face in &faces {
            let mut cycle = Vec::new();
            for corner in self.face_walk(face) {
                let v = self.corners[corner].vertex;
                let n = *numbers.entry(v).or_insert_with(|| {
                    soup.points.push(self.vertices[v].position);
                    soup.points.len() - 1
                });
                cycle.push(n);
            }
            let f = self.faces[face];
            soup.faces.push((cycle, f.look(), f.selected));
        }
        for (_, modifier) in list {
            soup.run(modifier);
        }
        // Out with the part, in with the result.
        let vertices: Vec<VertexId> = numbers.keys().copied().collect();
        for v in vertices {
            if self.vertices.contains(v) {
                self.remove_vertex(v)?;
            }
        }
        let mut ids: HashMap<usize, VertexId> = HashMap::new();
        let mut made = 0;
        for (face, smooth, selected) in &soup.faces {
            let mut cycle = Vec::with_capacity(face.len());
            for &i in face {
                let id = match ids.get(&i) {
                    Some(&id) => id,
                    None => {
                        let id = self.add_vertex(soup.points[i])?;
                        ids.insert(i, id);
                        id
                    }
                };
                cycle.push(id);
            }
            match self.add_face(&cycle) {
                Ok(new) => {
                    let f = &mut self.faces[new];
                    f.group = group;
                    f.set_look(*smooth);
                    f.selected = select || *selected;
                    made += 1;
                }
                Err(MeshError::FaceExists(_) | MeshError::RepeatedVertex(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(made)
    }
}
