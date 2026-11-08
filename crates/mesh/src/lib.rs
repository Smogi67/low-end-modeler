//! Editable polygon mesh for Mixer. Spec section 5 sets the
//! requirements; this file records how they are met.
//!
//! # Structure
//!
//! The mesh stores four kinds of element, each in its own array, linked
//! by typed IDs instead of references:
//!
//! - [`Vertex`]: a position, plus one edge from its *disk cycle*.
//! - [`Edge`]: two vertices. It is a member of a disk cycle at each end (all
//!   edges around that vertex), and it holds one corner from its *radial cycle*
//!   (all face corners using that edge).
//! - [`Corner`]: one corner of one face. This is the half-edge: it runs from its
//!   vertex to the next corner's vertex along its edge. The *face cycle* links
//!   the corners of a face in winding order.
//! - [`Face`]: an n-gon with three or more corners.
//!
//! This is the same layout as Blender's BMesh, chosen over a classic twin-pointer
//! half-edge because modeling routinely creates shapes that a twin pointer cannot
//! store: wire edges, loose vertices, three faces on one edge (extruding an inner
//! edge), two fans meeting at one vertex, and neighboring faces with opposite
//! winding. Here they are all valid states. Neighbor lookups cost only as much as
//! the number of neighbors.
//!
//! "Corner" is Blender's own newer name for what BMesh calls a loop. It avoids
//! confusion with edge loops and loop cuts, which are a separate idea.
//!
//! # Rules that always hold
//!
//! [`Mesh::validate`] checks all of these; [`validate`](mod@validate) lists them.
//! The most important ones:
//!
//! - An edge joins two different vertices. At most one edge joins any pair.
//! - A face has at least 3 corners and never repeats a vertex.
//! - No two faces use exactly the same set of vertices.
//! - Winding need not agree between neighboring faces. Fixing it is the job of
//!   the normals tool and export, not of this structure.
//!
//! Every public edit either succeeds and keeps these rules, or returns an error
//! and leaves the mesh untouched.
//!
//! # Stale IDs
//!
//! An ID is an array index plus a generation number. Each array hands out each
//! generation once and never reuses or rewinds it, not even on undo. An ID for a
//! deleted element can therefore never match a later element placed in the same
//! slot, and looking it up reports an error. Redo restores the original
//! generation, so IDs from before the undo become valid again.
//!
//! # Undo and redo
//!
//! Undo uses saved copies of only the changed elements. While an edit runs, each
//! array saves an element's old value the first time it is changed. The array's
//! fields are private and the only way to change an element is through code that
//! does this saving, so **a new tool gets correct undo without writing any undo
//! code**. Undoing swaps the saved values with the live ones, and that swap is
//! also what makes redo work.
//!
//! - Memory use follows the size of the edit, not the size of the mesh.
//! - History is capped by bytes and by step count. See [`HistoryLimits`].
//! - A failed or cancelled edit restores the exact previous state.
//! - [`Document`] owns the mesh and hands out `&mut Mesh` only during an edit, so
//!   the compiler rejects any change that bypasses history.
//!
//! Planned extensions that this design leaves room for:
//!
//! - **Selection:** stored as fields on the elements, so undo restores it like anything
//!   else. See `selection.rs`.
//! - **Several objects (Stage 3 and later):** a history step can grow to hold one
//!   change per mesh, without changing how a single mesh records changes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bevel;
mod boolean;
mod cut;
mod error;
mod gltf;
mod groups;
mod history;
mod id;
mod loops;
mod mesh;
mod obj;
mod ops;
mod pool;
mod primitives;
mod project;
mod selection;
mod shading;
mod tools;
mod triangulate;
pub mod validate;

pub use bevel::{Bevel, BevelSettings, MAX_BEVEL_SEGMENTS};
pub use boolean::BooleanOp;
pub use cut::CutPoint;
pub use error::MeshError;
pub use history::{DEFAULT_MAX_BYTES, DEFAULT_MAX_STEPS, Document, HistoryLimits};
pub use id::{CornerId, EdgeId, FaceId, VertexId};
pub use loops::{MAX_LOOP_CUTS, SlideRail};
pub use mesh::{Corner, Edge, Face, Mesh, Vertex};
pub use obj::ROBLOX_MAX_TRIANGLES;
pub use primitives::{MAX_SEGMENTS, MAX_SUBDIVISIONS, Primitive};
pub use project::{FileKind, LoadReport, PROJECT_VERSION, ParseError, ProjectSettings};
pub use selection::{Element, SelectMode, SelectOp};
pub use shading::{AUTO_SMOOTH_ANGLE, ShadingNormals};
pub use validate::ValidationError;
