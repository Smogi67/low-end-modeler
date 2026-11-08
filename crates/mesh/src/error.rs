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
    /// The tool works on edges, so it needs vertex or edge mode.
    NeedsEdgeMode,
    /// A loop cut makes from 1 to `MAX_LOOP_CUTS` cuts.
    InvalidCutCount,
    /// A loop cut needs a four-sided face beside the edge.
    CannotLoopCut,
    /// The edges branch: a vertex has more than two of them.
    NotLoops,
    /// No neighboring edge leads away from the slid edges.
    NothingToSlide,
    /// Bridging needs two separate loops with the same number of edges.
    BridgeNeedsTwoLoops,
    /// A bevel has from 1 to `MAX_BEVEL_SEGMENTS` segments.
    InvalidSegments,
    /// A bevel's shape runs from 0 to 1.
    InvalidShape,
    /// An edge to bevel needs a face on each side, and its ends need simple fans of
    /// faces that wind the same way.
    CannotBevel,
    /// Smooth shading's angle runs from 0 (flat) to 180.
    InvalidSmoothAngle,
    /// A cut must cross a face from one edge or corner to another.
    NothingToCut,
    /// Union, subtract and intersect need closed parts.
    NotClosed,
    /// Subtract needs to know which part to cut out.
    NoCutter,
    /// The shapes do not overlap, so nothing would be left.
    EmptyResult,
    /// The result would have too many pieces to handle.
    TooComplex,
    /// Joining needs at least two parts selected.
    NeedsTwoParts,
    /// None of the selected parts are joined to others.
    NotJoined,
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
            Self::NeedsEdgeMode => write!(f, "switch to vertex or edge mode first"),
            Self::InvalidCutCount => write!(f, "a loop cut makes from 1 to 32 cuts"),
            Self::CannotLoopCut => {
                write!(f, "a loop cut needs a four-sided face next to the edge")
            }
            Self::NotLoops => write!(f, "select edge loops or lines that do not branch"),
            Self::NothingToSlide => write!(f, "there is no neighboring edge to slide along"),
            Self::BridgeNeedsTwoLoops => write!(
                f,
                "select two separate edge loops with the same number of edges"
            ),
            Self::InvalidSegments => write!(f, "a bevel has from 1 to 16 segments"),
            Self::InvalidShape => write!(f, "a bevel's shape runs from 0 to 1"),
            Self::CannotBevel => write!(
                f,
                "only edges with a face on each side can be beveled, on a surface whose faces all face the same way"
            ),
            Self::InvalidSmoothAngle => write!(f, "a smoothing angle runs from 0 to 180 degrees"),
            Self::NothingToCut => write!(
                f,
                "nothing was cut: a cut must cross a face from one edge to another"
            ),
            Self::NotClosed => write!(
                f,
                "union, subtract and intersect need closed shapes with no holes"
            ),
            Self::NoCutter => write!(f, "click the part to cut out last"),
            Self::EmptyResult => write!(f, "the shapes do not overlap, so nothing would be left"),
            Self::TooComplex => write!(f, "the result would be too detailed to make"),
            Self::NeedsTwoParts => write!(f, "select at least two parts first"),
            Self::NotJoined => write!(f, "the selected parts are not joined to anything"),
            Self::IdsExhausted => write!(f, "this session has used all available element IDs"),
        }
    }
}

impl std::error::Error for MeshError {}
