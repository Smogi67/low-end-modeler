//! Errors returned by mesh queries and edits.

use std::fmt;

use crate::id::{EdgeId, FaceId, VertexId};

/// Why a query or edit was refused. A refused edit leaves the mesh unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshError {
    /// The vertex was deleted, or the ID came from an undone edit.
    StaleVertex(VertexId),
    /// The edge was deleted, or the ID came from an undone edit.
    StaleEdge(EdgeId),
    /// The face was deleted, or the ID came from an undone edit.
    StaleFace(FaceId),
    /// A position contained NaN or infinity.
    NonFinitePosition,
    /// An edge split factor was outside 0.0 to 1.0, or was NaN.
    InvalidFactor,
    /// An edge needs two different vertices.
    SameVertex(VertexId),
    /// An edge already joins these two vertices.
    EdgeExists(EdgeId),
    /// A face needs at least 3 vertices.
    FaceTooSmall {
        /// How many vertices were given.
        count: usize,
    },
    /// A vertex appeared more than once in a face.
    RepeatedVertex(VertexId),
    /// A face already uses exactly this set of vertices.
    FaceExists(FaceId),
    /// The vertex is not a corner of the face.
    VertexNotInFace {
        /// The vertex that was looked for.
        vertex: VertexId,
        /// The face that was searched.
        face: FaceId,
    },
    /// A face can only be split between corners that are not already neighbors.
    CornersAdjacent {
        /// The face that was to be split.
        face: FaceId,
    },
    /// Joining faces across an edge needs exactly two faces on that edge.
    NotTwoFaces {
        /// The edge that was to be removed.
        edge: EdgeId,
        /// How many faces use that edge.
        faces: usize,
    },
    /// The two faces wind in the same direction along the edge. Flip one first.
    InconsistentWinding(EdgeId),
    /// The faces also touch at another vertex, so joining them would produce a
    /// face that repeats that vertex.
    JoinWouldRepeatVertex(VertexId),
    /// An edit is already running on the document.
    EditInProgress,
    /// The operation needs an open edit, but none is running.
    NoEditInProgress,
    /// A primitive's size or segment count is out of range.
    InvalidPrimitive,
    /// An export scale must be a positive number.
    InvalidScale,
    /// The elements given do not match the selection mode.
    WrongSelectMode,
    /// An axis must be 0, 1 or 2.
    InvalidAxis,
    /// The tool needs something selected, or more than is selected.
    NothingSelected,
    /// The tool needs at least one selected face.
    NoFacesSelected,
    /// The selection cannot make a new edge or face.
    CannotFill,
    /// A merge distance must be a positive number.
    InvalidDistance,
    /// All IDs for an element type have been used. This takes about four billion
    /// element creations in one session, so it should never occur in practice.
    IdsExhausted,
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleVertex(id) => write!(f, "vertex {id} no longer exists"),
            Self::StaleEdge(id) => write!(f, "edge {id} no longer exists"),
            Self::StaleFace(id) => write!(f, "face {id} no longer exists"),
            Self::NonFinitePosition => write!(f, "position contains NaN or infinity"),
            Self::InvalidFactor => write!(f, "split factor must be between 0 and 1"),
            Self::SameVertex(id) => write!(f, "an edge cannot join vertex {id} to itself"),
            Self::EdgeExists(id) => write!(f, "edge {id} already joins these vertices"),
            Self::FaceTooSmall { count } => {
                write!(f, "a face needs at least 3 vertices, got {count}")
            }
            Self::RepeatedVertex(id) => write!(f, "vertex {id} is repeated in the face"),
            Self::FaceExists(id) => write!(f, "face {id} already uses these vertices"),
            Self::VertexNotInFace { vertex, face } => {
                write!(f, "vertex {vertex} is not a corner of face {face}")
            }
            Self::CornersAdjacent { face } => {
                write!(f, "these corners of face {face} are already neighbors")
            }
            Self::NotTwoFaces { edge, faces } => {
                write!(f, "edge {edge} has {faces} faces; joining needs exactly 2")
            }
            Self::InconsistentWinding(id) => {
                write!(f, "the faces on edge {id} have opposite winding")
            }
            Self::JoinWouldRepeatVertex(id) => {
                write!(
                    f,
                    "the faces also touch at vertex {id}, so they cannot join"
                )
            }
            Self::EditInProgress => write!(f, "another edit is already running"),
            Self::NoEditInProgress => write!(f, "no edit is running"),
            Self::InvalidPrimitive => write!(f, "that shape's size or detail is out of range"),
            Self::InvalidScale => write!(f, "the export scale must be a positive number"),
            Self::WrongSelectMode => write!(f, "those elements do not match the selection mode"),
            Self::InvalidAxis => write!(f, "the axis must be X, Y or Z"),
            Self::NothingSelected => write!(f, "select more first"),
            Self::NoFacesSelected => write!(f, "select at least one face first"),
            Self::CannotFill => write!(f, "these already form an edge or face"),
            Self::InvalidDistance => write!(f, "the merge distance must be a positive number"),
            Self::IdsExhausted => write!(f, "this session has used all available element IDs"),
        }
    }
}

impl std::error::Error for MeshError {}
