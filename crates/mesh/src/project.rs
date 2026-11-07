//! Saving and opening: the modeler's own project file, and importing OBJ files.
//!
//! A project file (`.lem`) is plain text in the style of OBJ, so it can be read and
//! repaired by hand, and it keeps exactly what the editor has: faces with any number
//! of corners, loose edges, and loose vertices, Z up, untriangulated.
//!
//! ```text
//! # Low-End Modeler project
//! lem 1
//! v 0 0 0
//! v 1 0 0
//! v 1 1 0
//! f 1 2 3
//! l 3 1
//! ```
//!
//! OBJ files from other programs read with the same parser. They are Y up, so they are
//! turned to Z up, the reverse of export. Texture coordinates, normals, groups and
//! materials are skipped, since the modeler has no use for them yet.

use std::collections::HashSet;
use std::fmt::{self, Write};

use crate::error::MeshError;
use crate::id::VertexId;
use crate::mesh::Mesh;

/// The project format's version. A newer file is refused rather than misread.
pub const PROJECT_VERSION: u32 = 1;

/// Why a file could not be read. Nothing is changed when this is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The line the problem is on, counting from 1, or 0 for the whole file.
    pub line: usize,
    /// What is wrong, in plain words.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for ParseError {}

/// What was read, and what had to be left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// Faces left out because they repeated a vertex, had fewer than three corners, or
    /// repeated another face.
    pub skipped_faces: usize,
}

/// Which kind of file is being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// The modeler's own `.lem` project.
    Project,
    /// An OBJ file, Y up.
    Obj,
}

fn bad(line: usize, message: impl Into<String>) -> ParseError {
    ParseError {
        line,
        message: message.into(),
    }
}

impl Mesh {
    /// The mesh as project file text. Selection is not saved.
    pub fn to_project(&self) -> String {
        let mut out = String::with_capacity(self.corner_count() * 12 + self.vertex_count() * 40);
        let _ = writeln!(out, "# Low-End Modeler project\nlem {PROJECT_VERSION}");
        let slots = self
            .vertices()
            .map(|(id, _)| id.index() as usize + 1)
            .max()
            .unwrap_or(0);
        let mut numbers = vec![0usize; slots];
        for (n, (id, vertex)) in self.vertices().enumerate() {
            numbers[id.index() as usize] = n + 1;
            let [x, y, z] = vertex.position.map(|c| c + 0.0);
            // Rust prints the shortest text that reads back as the same number.
            let _ = writeln!(out, "v {x} {y} {z}");
        }
        let number = |v: VertexId| numbers[v.index() as usize];
        for (face, _) in self.faces() {
            out.push('f');
            for corner in self.face_walk(face) {
                let _ = write!(out, " {}", number(self.corners[corner].vertex));
            }
            out.push('\n');
        }
        for (id, edge) in self.edges() {
            if self.edges[id].first_corner.is_none() {
                let [a, b] = edge.vertices;
                let _ = writeln!(out, "l {} {}", number(a), number(b));
            }
        }
        out
    }

    /// Reads a project or OBJ file into a new mesh.
    pub fn from_text(text: &str, kind: FileKind) -> Result<(Mesh, LoadReport), ParseError> {
        let mut mesh = Mesh::new();
        let mut report = LoadReport::default();
        let mut vertices: Vec<VertexId> = Vec::new();
        let mut seen_header = false;
        let mut seen_faces: HashSet<Vec<VertexId>> = HashSet::new();

        // OBJ numbers from 1, and negative numbers count back from the latest vertex.
        let index = |token: &str, line: usize, count: usize| -> Result<usize, ParseError> {
            let first = token.split('/').next().unwrap_or("");
            let n: i64 = first
                .parse()
                .map_err(|_| bad(line, format!("\"{token}\" is not a vertex number")))?;
            let at = if n > 0 {
                n - 1
            } else if n < 0 {
                count as i64 + n
            } else {
                -1
            };
            if at < 0 || at as usize >= count {
                return Err(bad(line, format!("there is no vertex {n}")));
            }
            Ok(at as usize)
        };

        for (number, raw) in text.lines().enumerate() {
            let line = number + 1;
            let content = raw.split('#').next().unwrap_or("").trim();
            let mut words = content.split_whitespace();
            let Some(keyword) = words.next() else {
                continue;
            };
            let rest: Vec<&str> = words.collect();
            match keyword {
                "lem" => {
                    let version: u32 = rest
                        .first()
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| bad(line, "the version number is missing"))?;
                    if version > PROJECT_VERSION {
                        return Err(bad(
                            line,
                            "this project was saved by a newer version of the modeler",
                        ));
                    }
                    seen_header = true;
                }
                "v" => {
                    if rest.len() < 3 {
                        return Err(bad(line, "a vertex needs three numbers"));
                    }
                    let mut p = [0.0f32; 3];
                    for (slot, token) in p.iter_mut().zip(&rest) {
                        *slot = token
                            .parse::<f32>()
                            .ok()
                            .filter(|c| c.is_finite())
                            .ok_or_else(|| bad(line, format!("\"{token}\" is not a number")))?;
                    }
                    // OBJ is Y up: (x, y, z) there is (x, -z, y) here.
                    let position = match kind {
                        FileKind::Project => p,
                        FileKind::Obj => [p[0], -p[2] + 0.0, p[1]],
                    };
                    let id = mesh
                        .add_vertex(position)
                        .map_err(|e| bad(line, e.to_string()))?;
                    vertices.push(id);
                }
                "f" => {
                    let mut corners = Vec::with_capacity(rest.len());
                    for token in &rest {
                        corners.push(vertices[index(token, line, vertices.len())?]);
                    }
                    let mut key = corners.clone();
                    key.sort_unstable();
                    let repeats = key.windows(2).any(|w| w[0] == w[1]);
                    if corners.len() < 3 || repeats || !seen_faces.insert(key) {
                        report.skipped_faces += 1;
                        continue;
                    }
                    match mesh.add_face(&corners) {
                        Ok(_) => {}
                        Err(MeshError::FaceExists(_) | MeshError::RepeatedVertex(_)) => {
                            report.skipped_faces += 1;
                        }
                        Err(error) => return Err(bad(line, error.to_string())),
                    }
                }
                "l" => {
                    let mut previous: Option<VertexId> = None;
                    for token in &rest {
                        let v = vertices[index(token, line, vertices.len())?];
                        if let Some(p) = previous
                            && p != v
                            && mesh.find_edge(p, v).is_none()
                        {
                            mesh.add_edge(p, v).map_err(|e| bad(line, e.to_string()))?;
                        }
                        previous = Some(v);
                    }
                }
                // Parts of OBJ files the modeler does not use.
                "vt" | "vn" | "vp" | "o" | "g" | "s" | "usemtl" | "mtllib" => {}
                other if kind == FileKind::Obj => {
                    // Other programs write more kinds of line; they hold nothing needed.
                    let _ = other;
                }
                other => {
                    return Err(bad(
                        line,
                        format!("\"{other}\" is not part of a project file"),
                    ));
                }
            }
        }
        if kind == FileKind::Project && !seen_header {
            return Err(bad(0, "this is not a Low-End Modeler project file"));
        }
        Ok((mesh, report))
    }
}
