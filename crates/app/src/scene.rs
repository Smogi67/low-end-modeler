//! Turns the mesh into plain number lists for the graphics card, and updates just the
//! moved parts during a transform (spec section 4, rule 3). No graphics code, so it is
//! tested without a window.

use std::collections::{HashMap, HashSet};

use mixer_mesh::{EdgeId, FaceId, Mesh, SelectMode, VertexId};

use crate::picking::face_dot;

/// Position, normal and selected (0 or 1), for each corner of each triangle.
pub const FLOATS_PER_TRIANGLE_VERTEX: usize = 7;
/// Position and color, for each end of an edge and each point.
pub const FLOATS_PER_LINE_VERTEX: usize = 6;

const NORMAL_COLOR: [f32; 3] = [0.11, 0.115, 0.13];
const SELECTED_COLOR: [f32; 3] = [1.0, 0.55, 0.10];

/// A range of floats that changed, to send to the graphics card.
pub type Range = std::ops::Range<usize>;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Changed {
    pub triangles: Vec<Range>,
    pub edges: Vec<Range>,
    pub points: Vec<Range>,
}

/// The parts of the scene to rewrite when some vertices move.
#[derive(Debug, Default, Clone)]
pub struct MovedParts {
    faces: Vec<FaceId>,
    edges: Vec<EdgeId>,
    vertices: Vec<VertexId>,
}

/// The mesh's number lists, and where each element sits in them.
#[derive(Debug, Default)]
pub struct Scene {
    pub triangles: Vec<f32>,
    pub edges: Vec<f32>,
    pub points: Vec<f32>,
    mode: Option<SelectMode>,
    face_spans: HashMap<FaceId, Range>,
    edge_offsets: HashMap<EdgeId, usize>,
    vertex_points: HashMap<VertexId, usize>,
    face_points: HashMap<FaceId, usize>,
}

fn color(selected: bool) -> [f32; 3] {
    if selected {
        SELECTED_COLOR
    } else {
        NORMAL_COLOR
    }
}

fn write_face(mesh: &Mesh, face: FaceId, out: &mut [f32]) {
    let normal = mesh
        .face_normal(face)
        .ok()
        .flatten()
        .unwrap_or([0.0, 0.0, 1.0]);
    let selected = f32::from(u8::from(mesh.face(face).is_some_and(|f| f.is_selected())));
    let mut chunks = out.chunks_exact_mut(FLOATS_PER_TRIANGLE_VERTEX);
    for triangle in mesh.face_triangles(face).unwrap_or_default() {
        for vertex in triangle {
            let (Some(chunk), Some(vertex)) = (chunks.next(), mesh.vertex(vertex)) else {
                return;
            };
            chunk[..3].copy_from_slice(&vertex.position());
            chunk[3..6].copy_from_slice(&normal);
            chunk[6] = selected;
        }
    }
}

fn write_edge(mesh: &Mesh, edge: EdgeId, out: &mut [f32]) {
    let Some(data) = mesh.edge(edge) else {
        return;
    };
    let tint = color(data.is_selected());
    for (chunk, vertex) in out
        .chunks_exact_mut(FLOATS_PER_LINE_VERTEX)
        .zip(data.vertices())
    {
        if let Some(vertex) = mesh.vertex(vertex) {
            chunk[..3].copy_from_slice(&vertex.position());
            chunk[3..].copy_from_slice(&tint);
        }
    }
}

/// Joins ranges that touch, so that fewer separate uploads are needed.
fn merged(mut ranges: Vec<Range>) -> Vec<Range> {
    ranges.sort_by_key(|range| range.start);
    let mut out: Vec<Range> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match out.last_mut() {
            Some(last) if last.end >= range.start => last.end = last.end.max(range.end),
            _ => out.push(range),
        }
    }
    out
}

impl Scene {
    /// Rebuilds everything. Used after any change other than vertices moving.
    pub fn rebuild(&mut self, mesh: &Mesh, mode: SelectMode) {
        *self = Self {
            triangles: std::mem::take(&mut self.triangles),
            edges: std::mem::take(&mut self.edges),
            points: std::mem::take(&mut self.points),
            mode: Some(mode),
            ..Self::default()
        };
        self.triangles.clear();
        self.edges.clear();
        self.points.clear();

        self.triangles
            .resize(mesh.triangle_count() * 3 * FLOATS_PER_TRIANGLE_VERTEX, 0.0);
        let mut start = 0;
        for (face, data) in mesh.faces() {
            let end = start + (data.corner_count() - 2) * 3 * FLOATS_PER_TRIANGLE_VERTEX;
            write_face(mesh, face, &mut self.triangles[start..end]);
            self.face_spans.insert(face, start..end);
            start = end;
        }

        self.edges
            .resize(mesh.edge_count() * 2 * FLOATS_PER_LINE_VERTEX, 0.0);
        for (i, (edge, _)) in mesh.edges().enumerate() {
            let at = i * 2 * FLOATS_PER_LINE_VERTEX;
            write_edge(
                mesh,
                edge,
                &mut self.edges[at..at + 2 * FLOATS_PER_LINE_VERTEX],
            );
            self.edge_offsets.insert(edge, at);
        }

        match mode {
            SelectMode::Vertex => {
                for (vertex, data) in mesh.vertices() {
                    self.vertex_points.insert(vertex, self.points.len());
                    self.points.extend_from_slice(&data.position());
                    self.points.extend_from_slice(&color(data.is_selected()));
                }
            }
            SelectMode::Edge => {}
            SelectMode::Face => {
                for (face, data) in mesh.faces() {
                    self.face_points.insert(face, self.points.len());
                    self.points.extend_from_slice(&face_dot(mesh, face));
                    self.points.extend_from_slice(&color(data.is_selected()));
                }
            }
        }
    }

    /// Works out which parts depend on these vertices. Done once, as a transform starts.
    pub fn moved_parts(mesh: &Mesh, vertices: &[VertexId]) -> MovedParts {
        let mut faces = HashSet::new();
        let mut edges = HashSet::new();
        for &vertex in vertices {
            if let Ok(found) = mesh.vertex_faces(vertex) {
                faces.extend(found);
            }
            if let Ok(found) = mesh.vertex_edges(vertex) {
                edges.extend(found);
            }
        }
        MovedParts {
            faces: faces.into_iter().collect(),
            edges: edges.into_iter().collect(),
            vertices: vertices.to_vec(),
        }
    }

    /// Rewrites only the given parts, and returns the ranges to send to the card.
    pub fn update_moved(&mut self, mesh: &Mesh, parts: &MovedParts) -> Changed {
        let mut triangles = Vec::with_capacity(parts.faces.len());
        for face in &parts.faces {
            if let Some(span) = self.face_spans.get(face) {
                write_face(mesh, *face, &mut self.triangles[span.clone()]);
                triangles.push(span.clone());
            }
        }
        let mut edges = Vec::with_capacity(parts.edges.len());
        for edge in &parts.edges {
            if let Some(&at) = self.edge_offsets.get(edge) {
                let span = at..at + 2 * FLOATS_PER_LINE_VERTEX;
                write_edge(mesh, *edge, &mut self.edges[span.clone()]);
                edges.push(span);
            }
        }
        let mut points = Vec::new();
        match self.mode {
            Some(SelectMode::Vertex) => {
                for vertex in &parts.vertices {
                    if let (Some(&at), Some(data)) =
                        (self.vertex_points.get(vertex), mesh.vertex(*vertex))
                    {
                        self.points[at..at + 3].copy_from_slice(&data.position());
                        points.push(at..at + 3);
                    }
                }
            }
            Some(SelectMode::Face) => {
                for face in &parts.faces {
                    if let Some(&at) = self.face_points.get(face) {
                        self.points[at..at + 3].copy_from_slice(&face_dot(mesh, *face));
                        points.push(at..at + 3);
                    }
                }
            }
            _ => {}
        }
        Changed {
            triangles: merged(triangles),
            edges: merged(edges),
            points: merged(points),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mixer_mesh::{Element, Primitive, SelectOp};

    fn cube() -> Mesh {
        let mut mesh = Mesh::new();
        mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
            .unwrap();
        mesh
    }

    fn built(mesh: &Mesh, mode: SelectMode) -> Scene {
        let mut scene = Scene::default();
        scene.rebuild(mesh, mode);
        scene
    }

    #[test]
    fn an_empty_mesh_gives_nothing() {
        let scene = built(&Mesh::new(), SelectMode::Vertex);
        assert!(scene.triangles.is_empty() && scene.edges.is_empty() && scene.points.is_empty());
    }

    #[test]
    fn a_cube_gives_12_triangles_and_12_edges() {
        let scene = built(&cube(), SelectMode::Edge);
        assert_eq!(scene.triangles.len(), 12 * 3 * FLOATS_PER_TRIANGLE_VERTEX);
        assert_eq!(scene.edges.len(), 12 * 2 * FLOATS_PER_LINE_VERTEX);
        assert!(scene.points.is_empty(), "edge mode draws no points");
    }

    #[test]
    fn points_follow_the_mode() {
        let mesh = cube();
        assert_eq!(
            built(&mesh, SelectMode::Vertex).points.len(),
            8 * FLOATS_PER_LINE_VERTEX
        );
        assert_eq!(
            built(&mesh, SelectMode::Face).points.len(),
            6 * FLOATS_PER_LINE_VERTEX
        );
    }

    #[test]
    fn each_corner_carries_its_faces_normal() {
        let mut mesh = Mesh::new();
        mesh.add_primitive(
            Primitive::Circle {
                radius: 1.0,
                segments: 8,
            },
            [0.0, 0.0, 5.0],
        )
        .unwrap();
        let scene = built(&mesh, SelectMode::Vertex);
        assert_eq!(scene.triangles.len(), 6 * 3 * FLOATS_PER_TRIANGLE_VERTEX);
        for vertex in scene.triangles.chunks_exact(FLOATS_PER_TRIANGLE_VERTEX) {
            assert_eq!(vertex[2], 5.0);
            assert_eq!(vertex[3..6], [0.0, 0.0, 1.0]);
            assert_eq!(vertex[6], 0.0);
        }
    }

    #[test]
    fn selection_shows_in_the_data() {
        let mut mesh = cube();
        mesh.select_all();
        let scene = built(&mesh, SelectMode::Vertex);
        assert!(
            scene
                .triangles
                .chunks_exact(FLOATS_PER_TRIANGLE_VERTEX)
                .all(|v| v[6] == 1.0)
        );
        assert!(
            scene
                .edges
                .chunks_exact(FLOATS_PER_LINE_VERTEX)
                .all(|v| v[3..] == SELECTED_COLOR)
        );
        assert!(
            scene
                .points
                .chunks_exact(FLOATS_PER_LINE_VERTEX)
                .all(|v| v[3..] == SELECTED_COLOR)
        );
    }

    #[test]
    fn wire_edges_are_drawn_and_a_face_without_area_gets_a_usable_normal() {
        let mut mesh = Mesh::new();
        let v: Vec<_> = (0..3)
            .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
            .collect();
        mesh.add_edge(v[0], v[1]).unwrap();
        let scene = built(&mesh, SelectMode::Vertex);
        assert_eq!(scene.edges.len(), 2 * FLOATS_PER_LINE_VERTEX);
        assert!(scene.triangles.is_empty());
        mesh.add_face(&v).unwrap();
        let scene = built(&mesh, SelectMode::Vertex);
        assert_eq!(scene.triangles[3..6], [0.0, 0.0, 1.0]);
    }

    /// The whole point: updating only the moved parts must give exactly what a full
    /// rebuild gives, in every mode, and must touch less than everything.
    #[test]
    fn partial_updates_match_a_full_rebuild() {
        for mode in [SelectMode::Vertex, SelectMode::Edge, SelectMode::Face] {
            let mut mesh = Mesh::new();
            mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
                .unwrap();
            let far = mesh
                .add_primitive(
                    Primitive::Icosphere {
                        radius: 1.0,
                        subdivisions: 2,
                    },
                    [9.0, 0.0, 0.0],
                )
                .unwrap();
            mesh.select_elements(
                SelectMode::Face,
                &[Element::Face(far[0])],
                SelectOp::Replace,
            )
            .unwrap();

            let mut scene = built(&mesh, mode);
            let moved = mesh.selected_vertices();
            let parts = Scene::moved_parts(&mesh, &moved);
            for &vertex in &moved {
                let mut p = mesh.vertex(vertex).unwrap().position();
                p[2] += 3.0;
                mesh.set_vertex_position(vertex, p).unwrap();
            }
            let changed = scene.update_moved(&mesh, &parts);

            let fresh = built(&mesh, mode);
            assert_eq!(scene.triangles, fresh.triangles, "{mode:?}");
            assert_eq!(scene.edges, fresh.edges, "{mode:?}");
            assert_eq!(scene.points, fresh.points, "{mode:?}");

            let sent: usize = changed.triangles.iter().map(|r| r.len()).sum();
            assert!(sent > 0);
            assert!(
                sent < scene.triangles.len() / 2,
                "{mode:?}: sent {sent} of {}",
                scene.triangles.len()
            );
            for ranges in [&changed.triangles, &changed.edges, &changed.points] {
                assert!(ranges.windows(2).all(|pair| pair[0].end < pair[1].start));
            }
        }
    }

    #[test]
    fn merging_joins_only_ranges_that_touch() {
        assert_eq!(merged(vec![5..7, 0..2, 2..3, 6..9]), vec![0..3, 5..9]);
        assert_eq!(merged(vec![]), Vec::<Range>::new());
    }
}
