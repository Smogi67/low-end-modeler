//! Long random edit sequences, checked against an independent model.
//!
//! The model below stores the mesh the plain way: a map of positions, a set of
//! edges, and a list of vertices per face. It shares no code with the mesh, so a
//! bug would have to be made twice, the same way, to go unnoticed.
//!
//! After every action this checks that:
//! - the mesh passes `validate`;
//! - the mesh holds the same shape as the model;
//! - a refused edit changed nothing;
//! - undo and redo restore the exact earlier state.
//!
//! The random numbers come from fixed seeds, so a failure repeats exactly.
//! Set `MESH_FUZZ_SEEDS` and `MESH_FUZZ_STEPS` to run longer.

mod common;
use common::{cube, normalize_cycle};
use mixer_mesh::{
    Document, EdgeId, Element, FaceId, HistoryLimits, Mesh, MeshError, SelectMode, SelectOp,
    VertexId,
};
use std::collections::{BTreeMap, BTreeSet};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // SplitMix64 scrambling, so nearby seeds give unrelated sequences.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((z ^ (z >> 31)) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 1001) as f32 / 1000.0
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        if items.is_empty() {
            None
        } else {
            Some(items[self.below(items.len())])
        }
    }
}

fn pair(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    (a.min(b), a.max(b))
}

#[derive(Clone, PartialEq, Debug, Default)]
struct Model {
    positions: BTreeMap<VertexId, [f32; 3]>,
    edges: BTreeSet<(VertexId, VertexId)>,
    faces: BTreeMap<FaceId, Vec<VertexId>>,
}

fn face_edges(cycle: &[VertexId]) -> impl Iterator<Item = (VertexId, VertexId)> + '_ {
    (0..cycle.len()).map(move |i| pair(cycle[i], cycle[(i + 1) % cycle.len()]))
}

impl Model {
    fn read(mesh: &Mesh) -> Self {
        Self {
            positions: mesh.vertices().map(|(id, v)| (id, v.position())).collect(),
            edges: mesh
                .edges()
                .map(|(_, e)| {
                    let [a, b] = e.vertices();
                    pair(a, b)
                })
                .collect(),
            faces: mesh
                .faces()
                .map(|(id, _)| {
                    let cycle: Vec<_> = mesh.face_vertices(id).unwrap().collect();
                    (id, normalize_cycle(&cycle))
                })
                .collect(),
        }
    }

    fn add_face(&mut self, face: FaceId, cycle: &[VertexId]) {
        self.edges.extend(face_edges(cycle));
        self.faces.insert(face, normalize_cycle(cycle));
    }

    fn remove_edge(&mut self, edge: (VertexId, VertexId)) {
        self.faces
            .retain(|_, cycle| !face_edges(cycle).any(|e| e == edge));
        self.edges.remove(&edge);
    }

    fn remove_vertex(&mut self, vertex: VertexId) {
        self.faces.retain(|_, cycle| !cycle.contains(&vertex));
        self.edges.retain(|&(a, b)| a != vertex && b != vertex);
        self.positions.remove(&vertex);
    }

    fn split_edge(&mut self, edge: (VertexId, VertexId), middle: VertexId, position: [f32; 3]) {
        self.positions.insert(middle, position);
        self.edges.remove(&edge);
        self.edges.insert(pair(edge.0, middle));
        self.edges.insert(pair(middle, edge.1));
        for cycle in self.faces.values_mut() {
            let n = cycle.len();
            if let Some(i) = (0..n).find(|&i| pair(cycle[i], cycle[(i + 1) % n]) == edge) {
                cycle.insert(i + 1, middle);
                *cycle = normalize_cycle(cycle);
            }
        }
    }

    fn split_face(&mut self, face: FaceId, new_face: FaceId, a: VertexId, b: VertexId) {
        let cycle = self.faces[&face].clone();
        let n = cycle.len();
        let start = cycle.iter().position(|&v| v == a).unwrap();
        let rotated: Vec<_> = (0..n).map(|i| cycle[(start + i) % n]).collect();
        let at_b = rotated.iter().position(|&v| v == b).unwrap();
        let kept = rotated[..=at_b].to_vec();
        let mut moved = rotated[at_b..].to_vec();
        moved.push(a);
        self.edges.insert(pair(a, b));
        self.faces.insert(face, normalize_cycle(&kept));
        self.faces.insert(new_face, normalize_cycle(&moved));
    }

    /// `a -> b` must be the direction along which the kept face runs.
    fn join(&mut self, kept: FaceId, removed: FaceId, a: VertexId, b: VertexId) {
        let rotate_from = |cycle: &[VertexId], start: VertexId| -> Vec<VertexId> {
            let n = cycle.len();
            let s = cycle.iter().position(|&v| v == start).unwrap();
            (0..n).map(|i| cycle[(s + i) % n]).collect()
        };
        let first = rotate_from(&self.faces[&kept], b);
        let second = rotate_from(&self.faces[&removed], a);
        let mut merged = first;
        merged.extend_from_slice(&second[1..second.len() - 1]);
        self.faces.insert(kept, normalize_cycle(&merged));
        self.faces.remove(&removed);
        self.edges.remove(&pair(a, b));
    }

    fn flip(&mut self, face: FaceId) {
        let cycle = self.faces.get_mut(&face).unwrap();
        cycle.reverse();
        *cycle = normalize_cycle(cycle);
    }
}

/// Counts how often each action really happened, so the test can fail if random
/// choices stop reaching something and it quietly tests less than it claims.
#[derive(Default, Debug)]
struct Coverage {
    succeeded: BTreeMap<&'static str, usize>,
    refused: BTreeMap<&'static str, usize>,
}

struct Run {
    doc: Document,
    model: Model,
    /// State before each undo step, oldest first.
    undo_states: Vec<(Mesh, Model)>,
    redo_states: Vec<(Mesh, Model)>,
    /// All IDs ever seen, including dead ones, to test stale-ID handling.
    seen_vertices: Vec<VertexId>,
    seen_edges: Vec<EdgeId>,
    seen_faces: Vec<FaceId>,
    coverage: Coverage,
}

impl Run {
    fn new(mesh: Mesh) -> Self {
        let model = Model::read(&mesh);
        let limits = HistoryLimits {
            max_bytes: usize::MAX,
            max_steps: usize::MAX,
        };
        let mut run = Self {
            doc: Document::with_limits(mesh, limits),
            model,
            undo_states: Vec::new(),
            redo_states: Vec::new(),
            seen_vertices: Vec::new(),
            seen_edges: Vec::new(),
            seen_faces: Vec::new(),
            coverage: Coverage::default(),
        };
        run.remember_ids();
        run
    }

    fn remember_ids(&mut self) {
        let mesh = self.doc.mesh();
        self.seen_vertices.extend(mesh.vertices().map(|(id, _)| id));
        self.seen_edges.extend(mesh.edges().map(|(id, _)| id));
        self.seen_faces.extend(mesh.faces().map(|(id, _)| id));
        const CAP: usize = 4000;
        for list_len in [
            self.seen_vertices.len(),
            self.seen_edges.len(),
            self.seen_faces.len(),
        ] {
            if list_len > CAP {
                self.seen_vertices.sort_unstable();
                self.seen_vertices.dedup();
                self.seen_edges.sort_unstable();
                self.seen_edges.dedup();
                self.seen_faces.sort_unstable();
                self.seen_faces.dedup();
                let keep = |len: usize| len.saturating_sub(CAP / 2);
                let (v, e, f) = (
                    keep(self.seen_vertices.len()),
                    keep(self.seen_edges.len()),
                    keep(self.seen_faces.len()),
                );
                self.seen_vertices.drain(..v);
                self.seen_edges.drain(..e);
                self.seen_faces.drain(..f);
                break;
            }
        }
    }

    fn live_vertices(&self) -> Vec<VertexId> {
        self.doc.mesh().vertices().map(|(id, _)| id).collect()
    }
    fn live_edges(&self) -> Vec<EdgeId> {
        self.doc.mesh().edges().map(|(id, _)| id).collect()
    }
    fn live_faces(&self) -> Vec<FaceId> {
        self.doc.mesh().faces().map(|(id, _)| id).collect()
    }

    /// Mostly a live ID, sometimes any ID ever seen, which may be stale.
    fn any_vertex(&self, rng: &mut Rng) -> Option<VertexId> {
        if rng.below(8) == 0 {
            rng.pick(&self.seen_vertices)
        } else {
            rng.pick(&self.live_vertices())
        }
    }
    fn any_edge(&self, rng: &mut Rng) -> Option<EdgeId> {
        if rng.below(8) == 0 {
            rng.pick(&self.seen_edges)
        } else {
            rng.pick(&self.live_edges())
        }
    }
    fn any_face(&self, rng: &mut Rng) -> Option<FaceId> {
        if rng.below(8) == 0 {
            rng.pick(&self.seen_faces)
        } else {
            rng.pick(&self.live_faces())
        }
    }

    /// Runs one edit. On success, checks the result against what `update` does to
    /// the model. On refusal, checks that nothing changed.
    fn apply<T: Copy>(
        &mut self,
        name: &'static str,
        edit: impl FnOnce(&mut Mesh) -> Result<T, MeshError>,
        update: impl FnOnce(&mut Model, T, &Mesh),
    ) {
        let before_mesh = self.doc.mesh().clone();
        let before_model = self.model.clone();
        let undo_count = self.doc.undo_count();
        let redo_count = self.doc.redo_count();
        match self.doc.edit(name, edit) {
            Ok(value) => {
                *self.coverage.succeeded.entry(name).or_default() += 1;
                update(&mut self.model, value, self.doc.mesh());
                if self.doc.undo_count() == undo_count + 1 {
                    self.undo_states.push((before_mesh, before_model));
                    self.redo_states.clear();
                    assert_eq!(self.doc.redo_count(), 0);
                } else {
                    assert_eq!(self.doc.undo_count(), undo_count);
                    assert_eq!(self.doc.redo_count(), redo_count);
                    assert_eq!(
                        *self.doc.mesh(),
                        before_mesh,
                        "{name} made no step but changed"
                    );
                }
            }
            Err(_) => {
                *self.coverage.refused.entry(name).or_default() += 1;
                assert_eq!(
                    *self.doc.mesh(),
                    before_mesh,
                    "refused {name} changed the mesh"
                );
                assert_eq!(self.doc.undo_count(), undo_count);
                assert_eq!(self.doc.redo_count(), redo_count);
            }
        }
    }

    fn check(&self, context: &str) {
        let mesh = self.doc.mesh();
        if let Err(error) = mesh.validate() {
            panic!("{context}: {error}");
        }
        assert_eq!(
            Model::read(mesh),
            self.model,
            "{context}: the mesh and the model differ"
        );
        for (id, face) in mesh.faces() {
            assert_eq!(
                face.corner_count(),
                self.model.faces[&id].len(),
                "{context}"
            );
        }
        assert_eq!(self.undo_states.len(), self.doc.undo_count(), "{context}");
        assert_eq!(self.redo_states.len(), self.doc.redo_count(), "{context}");
    }

    fn step(&mut self, rng: &mut Rng) {
        match rng.below(25) {
            0 | 1 => {
                let position = [rng.unit() * 4.0, rng.unit() * 4.0, rng.unit() * 4.0];
                self.apply(
                    "add vertex",
                    |m| m.add_vertex(position),
                    |model, id, _| {
                        model.positions.insert(id, position);
                    },
                );
            }
            2 => {
                let Some(vertex) = self.any_vertex(rng) else {
                    return;
                };
                let position = [rng.unit(), rng.unit(), rng.unit()];
                self.apply(
                    "move",
                    |m| m.set_vertex_position(vertex, position),
                    |model, (), _| {
                        model.positions.insert(vertex, position);
                    },
                );
            }
            3 | 4 => {
                let (Some(a), Some(b)) = (self.any_vertex(rng), self.any_vertex(rng)) else {
                    return;
                };
                self.apply(
                    "add edge",
                    |m| m.add_edge(a, b),
                    |model, _, _| {
                        model.edges.insert(pair(a, b));
                    },
                );
            }
            5..=8 => {
                let count = 3 + rng.below(4);
                let chosen: Vec<_> = (0..count).filter_map(|_| self.any_vertex(rng)).collect();
                let cycle = chosen.clone();
                self.apply(
                    "add face",
                    move |m| m.add_face(&chosen),
                    move |model, face, _| model.add_face(face, &cycle),
                );
            }
            9 => {
                let Some(face) = self.any_face(rng) else {
                    return;
                };
                self.apply(
                    "remove face",
                    |m| m.remove_face(face),
                    |model, (), _| {
                        model.faces.remove(&face);
                    },
                );
            }
            10 => {
                let Some(edge) = self.any_edge(rng) else {
                    return;
                };
                let ends = self.doc.mesh().edge(edge).map(|e| e.vertices());
                self.apply(
                    "remove edge",
                    |m| m.remove_edge(edge),
                    |model, (), _| {
                        let [a, b] = ends.unwrap();
                        model.remove_edge(pair(a, b));
                    },
                );
            }
            11 => {
                let Some(vertex) = self.any_vertex(rng) else {
                    return;
                };
                self.apply(
                    "remove vertex",
                    |m| m.remove_vertex(vertex),
                    |model, (), _| model.remove_vertex(vertex),
                );
            }
            12..=14 => {
                let Some(edge) = self.any_edge(rng) else {
                    return;
                };
                let factor = if rng.below(10) == 0 { 1.5 } else { rng.unit() };
                let ends = self.doc.mesh().edge(edge).map(|e| e.vertices());
                self.apply(
                    "split edge",
                    |m| m.split_edge(edge, factor),
                    |model, (middle, _), _| {
                        let [a, b] = ends.unwrap();
                        let (pa, pb) = (model.positions[&a], model.positions[&b]);
                        let position =
                            std::array::from_fn(|i| pa[i] * (1.0 - factor) + pb[i] * factor);
                        model.split_edge(pair(a, b), middle, position);
                    },
                );
            }
            15..=17 => {
                let Some(face) = self.any_face(rng) else {
                    return;
                };
                let corners: Vec<VertexId> = match self.doc.mesh().face_vertices(face) {
                    Ok(vertices) => vertices.collect(),
                    Err(_) => vec![],
                };
                let a = rng.pick(&corners).or(self.any_vertex(rng));
                let b = rng.pick(&corners).or(self.any_vertex(rng));
                let (Some(a), Some(b)) = (a, b) else { return };
                self.apply(
                    "split face",
                    |m| m.split_face(face, a, b).map(|(new_face, _)| new_face),
                    |model, new_face, _| model.split_face(face, new_face, a, b),
                );
            }
            18 | 19 => {
                // Mostly aim at edges where a join can actually work.
                let joinable: Vec<EdgeId> = self
                    .live_edges()
                    .into_iter()
                    .filter(|&e| self.doc.mesh().edge_faces(e).unwrap().count() == 2)
                    .collect();
                let aimed = if rng.below(4) == 0 {
                    None
                } else {
                    rng.pick(&joinable)
                };
                let Some(edge) = aimed.or(self.any_edge(rng)) else {
                    return;
                };
                let info = self.doc.mesh().edge(edge).map(|data| data.vertices());
                self.apply(
                    "join",
                    |m| m.join_faces(edge),
                    |model, kept, _| {
                        let [a, b] = info.unwrap();
                        let key = pair(a, b);
                        let mut on_edge: Vec<FaceId> = model
                            .faces
                            .iter()
                            .filter(|(_, cycle)| face_edges(cycle).any(|e| e == key))
                            .map(|(id, _)| *id)
                            .collect();
                        assert_eq!(on_edge.len(), 2, "join needs two faces in the model");
                        on_edge.retain(|&id| id != kept);
                        let removed = on_edge[0];
                        let cycle = &model.faces[&kept];
                        let n = cycle.len();
                        let forward = (0..n).any(|i| cycle[i] == a && cycle[(i + 1) % n] == b);
                        if forward {
                            model.join(kept, removed, a, b);
                        } else {
                            model.join(kept, removed, b, a);
                        }
                    },
                );
            }
            20 => {
                let Some(face) = self.any_face(rng) else {
                    return;
                };
                self.apply(
                    "flip",
                    |m| m.flip_face(face),
                    |model, (), _| model.flip(face),
                );
            }
            21 if rng.below(4) == 0 => {
                // Selection changes nothing the model tracks, but must stay valid
                // through every other edit, and undo must restore it exactly.
                let modes = [SelectMode::Vertex, SelectMode::Edge, SelectMode::Face];
                let mode = modes[rng.below(3)];
                let choice = rng.below(6);
                let picked: Vec<Element> = match mode {
                    SelectMode::Vertex => rng
                        .pick(&self.live_vertices())
                        .map(Element::Vertex)
                        .into_iter()
                        .collect(),
                    SelectMode::Edge => rng
                        .pick(&self.live_edges())
                        .map(Element::Edge)
                        .into_iter()
                        .collect(),
                    SelectMode::Face => rng
                        .pick(&self.live_faces())
                        .map(Element::Face)
                        .into_iter()
                        .collect(),
                };
                self.apply(
                    "select",
                    move |m| {
                        match choice {
                            0 => m.select_elements(mode, &picked, SelectOp::Replace)?,
                            1 => m.select_elements(mode, &picked, SelectOp::Add)?,
                            2 => m.select_linked(mode),
                            3 => m.select_more(mode),
                            4 => m.select_less(mode),
                            _ => m.invert_selection(mode),
                        }
                        Ok(())
                    },
                    |_, (), _| {},
                );
            }
            21 if rng.below(2) == 0 => {
                // A new face beside an existing one, with matching winding, which
                // gives later joins and splits a well-formed surface to work on.
                let corners: Vec<_> = self.doc.mesh().corners().map(|(id, _)| id).collect();
                let Some(corner) = rng.pick(&corners) else {
                    return;
                };
                let mesh = self.doc.mesh();
                let data = *mesh.corner(corner).unwrap();
                let a = data.vertex();
                let b = mesh.corner(data.next()).unwrap().vertex();
                let extra = 1 + rng.below(3);
                let positions: Vec<[f32; 3]> = (0..extra)
                    .map(|_| [rng.unit(), rng.unit(), rng.unit()])
                    .collect();
                let stored = positions.clone();
                self.apply(
                    "add neighbor",
                    move |m| {
                        let mut cycle = vec![b, a];
                        for position in positions {
                            cycle.push(m.add_vertex(position)?);
                        }
                        m.add_face(&cycle)
                    },
                    move |model, face, mesh| {
                        let cycle: Vec<_> = mesh.face_vertices(face).unwrap().collect();
                        assert_eq!((cycle[0], cycle[1]), (b, a));
                        for (&vertex, &position) in cycle[2..].iter().zip(&stored) {
                            model.positions.insert(vertex, position);
                        }
                        model.add_face(face, &cycle);
                    },
                );
            }
            21 => {
                // A compound edit that is then cancelled.
                let before = self.doc.mesh().clone();
                self.doc.begin_edit("cancelled").unwrap();
                let (a, b) = (
                    rng.pick(&self.live_vertices()),
                    rng.pick(&self.live_edges()),
                );
                let mesh = self.doc.active_mesh().unwrap();
                let _ = mesh.add_vertex([1.0; 3]);
                if let Some(edge) = b {
                    let _ = mesh.split_edge(edge, 0.5);
                }
                if let Some(vertex) = a {
                    let _ = mesh.remove_vertex(vertex);
                }
                self.doc.cancel_edit().unwrap();
                assert_eq!(*self.doc.mesh(), before, "cancel did not restore the mesh");
                *self.coverage.succeeded.entry("cancel").or_default() += 1;
            }
            22 => {
                for _ in 0..1 + rng.below(4) {
                    let Some((mesh, model)) = self.undo_states.pop() else {
                        assert_eq!(self.doc.undo(), None);
                        break;
                    };
                    let now = (self.doc.mesh().clone(), self.model.clone());
                    assert!(self.doc.undo().is_some());
                    assert_eq!(*self.doc.mesh(), mesh, "undo did not restore the mesh");
                    self.model = model;
                    self.redo_states.push(now);
                    self.check("during undo");
                    *self.coverage.succeeded.entry("undo").or_default() += 1;
                }
            }
            _ => {
                for _ in 0..1 + rng.below(4) {
                    let Some((mesh, model)) = self.redo_states.pop() else {
                        assert_eq!(self.doc.redo(), None);
                        break;
                    };
                    let now = (self.doc.mesh().clone(), self.model.clone());
                    assert!(self.doc.redo().is_some());
                    assert_eq!(*self.doc.mesh(), mesh, "redo did not restore the mesh");
                    self.model = model;
                    self.undo_states.push(now);
                    self.check("during redo");
                    *self.coverage.succeeded.entry("redo").or_default() += 1;
                }
            }
        }
        self.remember_ids();
    }
}

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[test]
fn random_edits_match_the_model() {
    let seeds = setting("MESH_FUZZ_SEEDS", 40);
    let steps = setting("MESH_FUZZ_STEPS", 400);
    let mut total: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    // The coverage demands below are set for the default run, and scale with its size.
    let scale = |at_default: u64| (at_default * seeds * steps / (40 * 400)).max(1) as usize;

    for seed in 0..seeds {
        let mut rng = Rng::new(seed);
        let mut run = Run::new(if seed % 2 == 0 { cube().0 } else { Mesh::new() });
        for step in 0..steps {
            run.step(&mut rng);
            run.check(&format!("seed {seed}, step {step}"));
        }
        // Unwind everything, then redo exactly as many steps to return to the end.
        // Steps that were already undone when the run ended stay undone.
        let end = run.doc.mesh().clone();
        let mut undone = 0;
        while let Some((mesh, _)) = run.undo_states.pop() {
            assert!(run.doc.undo().is_some());
            assert_eq!(*run.doc.mesh(), mesh, "seed {seed}: full unwind");
            run.doc.mesh().validate().unwrap();
            undone += 1;
        }
        for _ in 0..undone {
            assert!(run.doc.redo().is_some());
            run.doc.mesh().validate().unwrap();
        }
        assert_eq!(*run.doc.mesh(), end, "seed {seed}: full redo");

        for (name, count) in run.coverage.succeeded {
            total.entry(name).or_default().0 += count;
        }
        for (name, count) in run.coverage.refused {
            total.entry(name).or_default().1 += count;
        }
    }

    // A quiet test that does nothing should fail, not pass.
    println!("action: (succeeded, refused)\n{total:#?}");
    let actions = [
        "add vertex",
        "move",
        "add edge",
        "add face",
        "remove face",
        "remove edge",
        "remove vertex",
        "split edge",
        "split face",
        "join",
        "flip",
        "cancel",
        "undo",
        "redo",
        "add neighbor",
        "select",
    ];
    for action in actions {
        let (succeeded, _) = total.get(action).copied().unwrap_or_default();
        assert!(
            succeeded >= scale(100),
            "{action} succeeded only {succeeded} times"
        );
    }
    let refusable = [
        "move",
        "add edge",
        "add face",
        "remove face",
        "remove edge",
        "remove vertex",
        "split edge",
        "split face",
        "join",
        "flip",
    ];
    for action in refusable {
        let (_, refused) = total.get(action).copied().unwrap_or_default();
        assert!(
            refused >= scale(5),
            "{action} was refused only {refused} times"
        );
    }
}

/// On a closed surface these edits must keep it closed, with the same Euler number.
#[test]
fn local_edits_preserve_a_closed_surface() {
    use common::{euler_characteristic, is_closed_and_consistent};
    for seed in 0..20 {
        let mut rng = Rng::new(1000 + seed);
        let (mesh, _, _) = cube();
        let mut doc = Document::new(mesh);
        let mut joins = 0;
        for _ in 0..300 {
            let edges: Vec<EdgeId> = doc.mesh().edges().map(|(id, _)| id).collect();
            let faces: Vec<FaceId> = doc.mesh().faces().map(|(id, _)| id).collect();
            match rng.below(3) {
                0 => {
                    let edge = rng.pick(&edges).unwrap();
                    doc.edit("split edge", |m| m.split_edge(edge, 0.5).map(|_| ()))
                        .unwrap();
                }
                1 => {
                    let face = rng.pick(&faces).unwrap();
                    let corners: Vec<_> = doc.mesh().face_vertices(face).unwrap().collect();
                    let (a, b) = (rng.pick(&corners).unwrap(), rng.pick(&corners).unwrap());
                    // An existing edge would add a third face to it, so skip those.
                    if a != b && doc.mesh().edge_between(a, b).unwrap().is_none() {
                        let _ = doc.edit("split face", |m| m.split_face(face, a, b).map(|_| ()));
                    }
                }
                _ => {
                    let edge = rng.pick(&edges).unwrap();
                    if doc.edit("join", |m| m.join_faces(edge).map(|_| ())).is_ok() {
                        joins += 1;
                    }
                }
            }
            let mesh = doc.mesh();
            mesh.validate().unwrap();
            assert_eq!(euler_characteristic(mesh), 2, "seed {seed}");
            assert!(is_closed_and_consistent(mesh), "seed {seed}");
        }
        assert!(joins > 0, "seed {seed} never exercised join");
    }
}

/// The memory cap must hold under a realistic mix of edits, and whatever remains
/// after dropping old steps must still undo and redo correctly.
#[test]
fn memory_cap_holds_during_random_edits() {
    for seed in 0..20 {
        let mut rng = Rng::new(2000 + seed);
        let (mesh, _, _) = cube();
        let limits = HistoryLimits {
            max_bytes: 4096,
            max_steps: usize::MAX,
        };
        let mut doc = Document::with_limits(mesh, limits);
        let mut states = vec![];
        for _ in 0..500 {
            let edges: Vec<EdgeId> = doc.mesh().edges().map(|(id, _)| id).collect();
            let vertices: Vec<VertexId> = doc.mesh().vertices().map(|(id, _)| id).collect();
            let before = doc.mesh().clone();
            let result = match rng.below(4) {
                0 => doc.edit("add", |m| m.add_vertex([rng.unit(); 3]).map(|_| ())),
                1 => {
                    let (a, b) = (rng.pick(&vertices).unwrap(), rng.pick(&vertices).unwrap());
                    doc.edit("edge", |m| m.add_edge(a, b).map(|_| ()))
                }
                2 => match rng.pick(&edges) {
                    Some(edge) => doc.edit("split", |m| m.split_edge(edge, 0.5).map(|_| ())),
                    None => Ok(()),
                },
                _ => {
                    let vertex = rng.pick(&vertices).unwrap();
                    let moved = [rng.unit(), rng.unit(), rng.unit()];
                    doc.edit("move", |m| m.set_vertex_position(vertex, moved))
                }
            };
            // With the cap active, the step count can stay level when a step is
            // recorded, so "the mesh changed" is the reliable sign of a new step.
            if result.is_ok() && *doc.mesh() != before {
                states.push(before);
            }
            let step_alone = doc.undo_count() + doc.redo_count() == 1;
            assert!(doc.history_bytes() <= limits.max_bytes || step_alone);
        }
        assert!(
            doc.undo_count() > 1,
            "the cap should still leave several steps"
        );
        let mut expected = states.iter().rev();
        while doc.undo().is_some() {
            assert_eq!(doc.mesh(), expected.next().unwrap(), "seed {seed}");
            doc.mesh().validate().unwrap();
            assert!(doc.history_bytes() <= limits.max_bytes);
        }
        while doc.redo().is_some() {
            doc.mesh().validate().unwrap();
            assert!(doc.history_bytes() <= limits.max_bytes);
        }
    }
}
