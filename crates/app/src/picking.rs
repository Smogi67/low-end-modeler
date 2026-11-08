//! Finding what is under the mouse or inside a dragged shape. No graphics code: the
//! depth check is passed in, so this is tested without a window.

use mixer_mesh::{EdgeId, Element, FaceId, Mesh, SelectMode, VertexId};

use crate::camera::Mat4;

const VERTEX_PICK_PIXELS: f32 = 14.0;
const EDGE_PICK_PIXELS: f32 = 10.0;
/// Points tested along an edge, so a long edge can be picked by its middle.
const EDGE_SAMPLES: u32 = 8;

/// The current view, in pixels.
pub struct ViewInfo {
    pub matrix: Mat4,
    pub width: f32,
    pub height: f32,
}

impl ViewInfo {
    /// Screen position and depth from 0 to 1, or `None` if behind the camera.
    pub fn project(&self, p: [f32; 3]) -> Option<[f32; 3]> {
        let m = &self.matrix;
        let at = |row: usize| m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row];
        let w = at(3);
        if w <= 1e-6 {
            return None;
        }
        Some([
            (at(0) / w + 1.0) / 2.0 * self.width,
            (1.0 - at(1) / w) / 2.0 * self.height,
            (at(2) / w + 1.0) / 2.0,
        ])
    }
}

/// A dragged selection shape, in pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum Region {
    Box {
        a: [f32; 2],
        b: [f32; 2],
    },
    /// A circle brush swept along a path.
    Brush {
        path: Vec<[f32; 2]>,
        radius: f32,
    },
    Lasso {
        points: Vec<[f32; 2]>,
    },
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let squared = dx * dx + dy * dy;
    let t = if squared > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

fn in_polygon(p: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut j = polygon.len().wrapping_sub(1);
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl Region {
    pub fn contains(&self, p: [f32; 2]) -> bool {
        match self {
            Region::Box { a, b } => {
                (a[0].min(b[0])..=a[0].max(b[0])).contains(&p[0])
                    && (a[1].min(b[1])..=a[1].max(b[1])).contains(&p[1])
            }
            Region::Brush { path, radius } => match path.as_slice() {
                [] => false,
                [only] => segment_distance(p, *only, *only) <= *radius,
                _ => path
                    .windows(2)
                    .any(|s| segment_distance(p, s[0], s[1]) <= *radius),
            },
            Region::Lasso { points } => points.len() >= 3 && in_polygon(p, points),
        }
    }
}

/// Where a face's selection dot goes: the middle of its first triangle, which always
/// lies on the face, even for concave ones. Also used to check the face is visible.
pub fn face_dot(mesh: &Mesh, face: FaceId) -> [f32; 3] {
    let triangle = mesh.face_triangles(face).unwrap_or_default();
    let Some(first) = triangle.first() else {
        return [0.0; 3];
    };
    let mut sum = [0.0; 3];
    for &vertex in first {
        let p = position(mesh, vertex);
        for i in 0..3 {
            sum[i] += p[i] / 3.0;
        }
    }
    sum
}

fn position(mesh: &Mesh, vertex: VertexId) -> [f32; 3] {
    mesh.vertex(vertex).map_or([0.0; 3], |v| v.position())
}

fn edge_ends(mesh: &Mesh, edge: EdgeId) -> Option<([f32; 3], [f32; 3])> {
    let [a, b] = mesh.edge(edge)?.vertices();
    Some((position(mesh, a), position(mesh, b)))
}

fn sample(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// The nearest face under the pointer, by testing its triangles.
fn pick_face(mesh: &Mesh, view: &ViewInfo, x: f32, y: f32) -> Option<Element> {
    let mut best: Option<(f32, FaceId)> = None;
    for (face, _) in mesh.faces() {
        for triangle in mesh.face_triangles(face).unwrap_or_default() {
            let projected = triangle.map(|v| view.project(position(mesh, v)));
            let [Some(a), Some(b), Some(c)] = projected else {
                continue;
            };
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-6 {
                continue;
            }
            let u = ((b[0] - x) * (c[1] - y) - (b[1] - y) * (c[0] - x)) / area;
            let v = ((c[0] - x) * (a[1] - y) - (c[1] - y) * (a[0] - x)) / area;
            let w = 1.0 - u - v;
            if u >= 0.0 && v >= 0.0 && w >= 0.0 {
                let depth = u * a[2] + v * b[2] + w * c[2];
                if best.is_none_or(|(nearest, _)| depth < nearest) {
                    best = Some((depth, face));
                }
            }
        }
    }
    best.map(|(_, face)| Element::Face(face))
}

/// What a click at (x, y) hits, or `None`. `visible` says whether a point is not hidden.
pub fn pick_point(
    mesh: &Mesh,
    mode: SelectMode,
    view: &ViewInfo,
    x: f32,
    y: f32,
    visible: &dyn Fn([f32; 3]) -> bool,
) -> Option<Element> {
    let mut best: Option<(f32, Element)> = None;
    match mode {
        SelectMode::Vertex => {
            for (id, vertex) in mesh.vertices() {
                let Some(s) = view.project(vertex.position()) else {
                    continue;
                };
                let d = segment_distance([x, y], [s[0], s[1]], [s[0], s[1]]);
                if d <= VERTEX_PICK_PIXELS
                    && best.is_none_or(|(nearest, _)| d < nearest)
                    && visible(vertex.position())
                {
                    best = Some((d, Element::Vertex(id)));
                }
            }
        }
        SelectMode::Edge => {
            return pick_edge(mesh, view, x, y, EDGE_PICK_PIXELS, visible).map(Element::Edge);
        }
        SelectMode::Face => return pick_face(mesh, view, x, y),
    }
    best.map(|(_, element)| element)
}

/// The visible edge nearest to (x, y), if one is within `reach` pixels.
pub fn pick_edge(
    mesh: &Mesh,
    view: &ViewInfo,
    x: f32,
    y: f32,
    reach: f32,
    visible: &dyn Fn([f32; 3]) -> bool,
) -> Option<EdgeId> {
    let mut best: Option<(f32, EdgeId)> = None;
    for (id, _) in mesh.edges() {
        let Some((a, b)) = edge_ends(mesh, id) else {
            continue;
        };
        let (Some(sa), Some(sb)) = (view.project(a), view.project(b)) else {
            continue;
        };
        let d = segment_distance([x, y], [sa[0], sa[1]], [sb[0], sb[1]]);
        if d <= reach && best.is_none_or(|(nearest, _)| d < nearest) {
            let seen =
                (0..=EDGE_SAMPLES).any(|i| visible(sample(a, b, i as f32 / EDGE_SAMPLES as f32)));
            if seen {
                best = Some((d, id));
            }
        }
    }
    best.map(|(_, edge)| edge)
}

/// Everything visible inside a dragged shape.
pub fn pick_region(
    mesh: &Mesh,
    mode: SelectMode,
    view: &ViewInfo,
    region: &Region,
    visible: &dyn Fn([f32; 3]) -> bool,
) -> Vec<Element> {
    let inside = |p: [f32; 3]| {
        view.project(p)
            .is_some_and(|s| region.contains([s[0], s[1]]))
            && visible(p)
    };
    match mode {
        SelectMode::Vertex => mesh
            .vertices()
            .filter(|(_, v)| inside(v.position()))
            .map(|(id, _)| Element::Vertex(id))
            .collect(),
        SelectMode::Edge => mesh
            .edges()
            .filter(|&(id, _)| {
                edge_ends(mesh, id).is_some_and(|(a, b)| {
                    (0..=EDGE_SAMPLES).any(|i| inside(sample(a, b, i as f32 / EDGE_SAMPLES as f32)))
                })
            })
            .map(|(id, _)| Element::Edge(id))
            .collect(),
        SelectMode::Face => mesh
            .faces()
            .filter(|&(id, _)| inside(face_dot(mesh, id)))
            .map(|(id, _)| Element::Face(id))
            .collect(),
    }
}

/// A depth image of the scene, for checking whether a point is hidden.
pub struct DepthImage {
    pub width: i32,
    pub height: i32,
    /// Bottom row first, as OpenGL gives it.
    pub depth: Vec<f32>,
}

impl DepthImage {
    /// True if a point at this screen position and depth is not behind what is drawn,
    /// checking neighboring pixels too, so points on a silhouette are not lost.
    pub fn shows(&self, x: f32, y: f32, depth: f32) -> bool {
        let (px, py) = (x as i32, self.height - 1 - y as i32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (ax, ay) = (px + dx, py + dy);
                if ax < 0 || ay < 0 || ax >= self.width || ay >= self.height {
                    continue;
                }
                let index = ay as usize * self.width as usize + ax as usize;
                if self.depth.get(index).is_some_and(|&d| depth <= d) {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use mixer_mesh::Primitive;

    fn view() -> (ViewInfo, Camera) {
        let mut camera = Camera {
            distance: 10.0,
            ..Camera::default()
        };
        camera.set_view(0.0, 0.0); // Looks along +Y, with X right and Z up.
        camera.orthographic = true;
        (
            ViewInfo {
                matrix: camera.view_projection(800.0 / 600.0),
                width: 800.0,
                height: 600.0,
            },
            camera,
        )
    }

    fn cube() -> Mesh {
        let mut mesh = Mesh::new();
        mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
            .unwrap();
        mesh
    }

    fn all_seen(_: [f32; 3]) -> bool {
        true
    }

    fn screen(view: &ViewInfo, p: [f32; 3]) -> [f32; 2] {
        let s = view.project(p).unwrap();
        [s[0], s[1]]
    }

    #[test]
    fn project_puts_the_target_in_the_middle() {
        let (view, _) = view();
        let s = view.project([0.0; 3]).unwrap();
        assert!((s[0] - 400.0).abs() < 0.01 && (s[1] - 300.0).abs() < 0.01);
        assert!((0.0..=1.0).contains(&s[2]));
        // Up on the model is up on screen, which is a smaller Y.
        assert!(view.project([0.0, 0.0, 1.0]).unwrap()[1] < 300.0);
    }

    #[test]
    fn points_behind_a_perspective_camera_do_not_project() {
        let mut camera = Camera {
            distance: 5.0,
            ..Camera::default()
        };
        camera.set_view(0.0, 0.0);
        let view = ViewInfo {
            matrix: camera.view_projection(1.0),
            width: 800.0,
            height: 800.0,
        };
        assert!(view.project([0.0, -20.0, 0.0]).is_none());
    }

    #[test]
    fn clicks_find_the_nearest_vertex_within_reach() {
        let (view, _) = view();
        let mesh = cube();
        let corner = [1.0, -1.0, 1.0];
        let s = screen(&view, corner);
        let hit = pick_point(
            &mesh,
            SelectMode::Vertex,
            &view,
            s[0] + 3.0,
            s[1] + 3.0,
            &|p| p[1] < 0.0,
        );
        let Some(Element::Vertex(id)) = hit else {
            panic!("expected a vertex, got {hit:?}");
        };
        assert_eq!(mesh.vertex(id).unwrap().position(), corner);
        let far = pick_point(&mesh, SelectMode::Vertex, &view, 5.0, 5.0, &all_seen);
        assert_eq!(far, None);
    }

    #[test]
    fn hidden_vertices_are_skipped() {
        let (view, _) = view();
        let mesh = cube();
        let s = screen(&view, [1.0, -1.0, 1.0]);
        // Front and back corners overlap in this view. Only the back one is "visible".
        let hit = pick_point(&mesh, SelectMode::Vertex, &view, s[0], s[1], &|p| {
            p[1] > 0.0
        });
        let Some(Element::Vertex(id)) = hit else {
            panic!();
        };
        assert_eq!(mesh.vertex(id).unwrap().position(), [1.0, 1.0, 1.0]);
        assert_eq!(
            pick_point(&mesh, SelectMode::Vertex, &view, s[0], s[1], &|_| false),
            None
        );
    }

    #[test]
    fn clicks_find_edges_along_their_length() {
        let (view, _) = view();
        let mesh = cube();
        // The middle of the top front edge, far from both of its ends.
        let s = screen(&view, [0.0, -1.0, 1.0]);
        let hit = pick_point(&mesh, SelectMode::Edge, &view, s[0], s[1] + 4.0, &|p| {
            p[1] < 0.0
        });
        let Some(Element::Edge(id)) = hit else {
            panic!("{hit:?}");
        };
        let [a, b] = mesh.edge(id).unwrap().vertices();
        let (a, b) = (position(&mesh, a), position(&mesh, b));
        assert!(a[2] == 1.0 && b[2] == 1.0 && a[1] == -1.0 && b[1] == -1.0);
        assert_eq!(
            pick_point(&mesh, SelectMode::Edge, &view, 5.0, 5.0, &all_seen),
            None
        );
        assert_eq!(
            pick_point(&mesh, SelectMode::Edge, &view, s[0], s[1], &|_| false),
            None
        );
    }

    #[test]
    fn clicks_find_the_nearest_face() {
        let (view, _) = view();
        let mesh = cube();
        let hit = pick_point(&mesh, SelectMode::Face, &view, 400.0, 300.0, &all_seen);
        let Some(Element::Face(id)) = hit else {
            panic!("{hit:?}");
        };
        assert_eq!(
            mesh.face_normal(id).unwrap(),
            Some([0.0, -1.0, 0.0]),
            "the front face"
        );
        assert_eq!(
            pick_point(&mesh, SelectMode::Face, &view, 5.0, 5.0, &all_seen),
            None
        );
    }

    #[test]
    fn a_face_can_be_clicked_from_behind() {
        let (view, _) = view();
        let mut mesh = Mesh::new();
        let faces = mesh
            .add_primitive(
                Primitive::Circle {
                    radius: 2.0,
                    segments: 8,
                },
                [0.0; 3],
            )
            .unwrap();
        // Facing +Z, so seen edge-on here: make it stand up, facing away from the camera.
        let moved: Vec<_> = mesh.vertices().map(|(id, v)| (id, v.position())).collect();
        for (id, p) in moved {
            mesh.set_vertex_position(id, [p[0], -p[1] * 0.0 + p[2], p[1]])
                .unwrap();
        }
        assert_eq!(
            pick_point(&mesh, SelectMode::Face, &view, 400.0, 300.0, &all_seen),
            Some(Element::Face(faces[0]))
        );
    }

    #[test]
    fn regions_contain_what_they_should() {
        let boxed = Region::Box {
            a: [20.0, 40.0],
            b: [10.0, 30.0],
        };
        assert!(boxed.contains([15.0, 35.0]) && boxed.contains([10.0, 30.0]));
        assert!(!boxed.contains([25.0, 35.0]) && !boxed.contains([15.0, 45.0]));

        let brush = Region::Brush {
            path: vec![[0.0, 0.0], [100.0, 0.0]],
            radius: 10.0,
        };
        assert!(brush.contains([50.0, 9.0]) && brush.contains([105.0, 0.0]));
        assert!(!brush.contains([50.0, 11.0]));
        let dot = Region::Brush {
            path: vec![[0.0, 0.0]],
            radius: 10.0,
        };
        assert!(dot.contains([5.0, 5.0]) && !dot.contains([20.0, 0.0]));
        assert!(
            !Region::Brush {
                path: vec![],
                radius: 10.0
            }
            .contains([0.0, 0.0])
        );

        // An L shape, so a point in its notch must be outside.
        let lasso = Region::Lasso {
            points: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 5.0],
                [5.0, 5.0],
                [5.0, 10.0],
                [0.0, 10.0],
            ],
        };
        assert!(lasso.contains([2.0, 8.0]) && lasso.contains([8.0, 2.0]));
        assert!(!lasso.contains([8.0, 8.0]));
        assert!(
            !Region::Lasso {
                points: vec![[0.0, 0.0], [1.0, 1.0]]
            }
            .contains([0.5, 0.5])
        );
    }

    #[test]
    fn regions_select_the_right_elements_in_each_mode() {
        let (view, _) = view();
        let mesh = cube();
        // Around the upper half of the cube in this view.
        let top_left = screen(&view, [-1.5, 0.0, 1.5]);
        let lower_right = screen(&view, [1.5, 0.0, 0.5]);
        let region = Region::Box {
            a: top_left,
            b: lower_right,
        };

        let picked = pick_region(&mesh, SelectMode::Vertex, &view, &region, &all_seen);
        assert_eq!(picked.len(), 4, "both top front and top back corners");
        let picked = pick_region(&mesh, SelectMode::Vertex, &view, &region, &|p| p[1] < 0.0);
        assert_eq!(
            picked.len(),
            2,
            "only the front ones when the back is hidden"
        );

        let picked = pick_region(&mesh, SelectMode::Edge, &view, &region, &|p| p[1] <= 0.0);
        assert!(picked.len() >= 5, "{}", picked.len());

        let whole = Region::Box {
            a: [0.0, 0.0],
            b: [800.0, 600.0],
        };
        assert_eq!(
            pick_region(&mesh, SelectMode::Face, &view, &whole, &all_seen).len(),
            6
        );
        let empty = Region::Box {
            a: [0.0, 0.0],
            b: [5.0, 5.0],
        };
        assert!(pick_region(&mesh, SelectMode::Face, &view, &empty, &all_seen).is_empty());
    }

    #[test]
    fn a_face_dot_lies_on_a_concave_face() {
        let mut mesh = Mesh::new();
        let outline = [
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 4.0, 0.0],
            [0.0, 4.0, 0.0],
        ];
        let v: Vec<_> = outline
            .iter()
            .map(|&p| mesh.add_vertex(p).unwrap())
            .collect();
        let face = mesh.add_face(&v).unwrap();
        let [x, y, z] = face_dot(&mesh, face);
        assert_eq!(z, 0.0);
        let on_shape = (x <= 4.0 && y <= 1.0) || (x <= 1.0 && y <= 4.0);
        assert!(on_shape, "({x}, {y})");
    }

    #[test]
    fn depth_image_checks_neighbors_and_bounds() {
        // A 3x3 image, all at depth 0.5, except the pixel at screen (0, 0) is far.
        let mut depth = vec![0.5; 9];
        depth[6] = 1.0; // Bottom row first, so index 6 is the top left.
        let image = DepthImage {
            width: 3,
            height: 3,
            depth,
        };
        assert!(image.shows(2.0, 2.0, 0.4), "in front of what is drawn");
        assert!(!image.shows(2.0, 2.0, 0.6), "behind what is drawn");
        assert!(image.shows(1.0, 1.0, 0.8), "a neighboring pixel shows it");
        assert!(!image.shows(50.0, 50.0, 0.1), "off screen");
    }
}
