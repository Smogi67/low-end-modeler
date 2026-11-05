//! Typed generational IDs.

use std::fmt;
use std::num::NonZeroU32;

/// Internal access to the two parts of an ID. Not public, so code outside this
/// crate can only get IDs from the mesh and cannot forge them.
pub(crate) trait RawId: Copy + Eq {
    fn from_parts(index: u32, generation: NonZeroU32) -> Self;
    fn slot(self) -> u32;
    fn generation(self) -> NonZeroU32;
}

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        ///
        /// Copying an ID does not keep the element alive. If the element is
        /// deleted, lookups with this ID report an error rather than returning
        /// a different element.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name {
            index: u32,
            generation: NonZeroU32,
        }

        impl $name {
            /// Array slot of this element. Useful for side tables such as GPU
            /// buffers. The same slot can be reused later by another element,
            /// so this number alone does not identify an element over time.
            pub fn index(self) -> u32 {
                self.index
            }
        }

        impl RawId for $name {
            fn from_parts(index: u32, generation: NonZeroU32) -> Self {
                Self { index, generation }
            }
            fn slot(self) -> u32 {
                self.index
            }
            fn generation(self) -> NonZeroU32 {
                self.generation
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}#{}"), self.index, self.generation)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(self, f)
            }
        }
    };
}

define_id!(
    /// Identifies a [`Vertex`](crate::Vertex).
    VertexId, "V"
);
define_id!(
    /// Identifies an [`Edge`](crate::Edge).
    EdgeId, "E"
);
define_id!(
    /// Identifies a [`Corner`](crate::Corner).
    CornerId, "C"
);
define_id!(
    /// Identifies a [`Face`](crate::Face).
    FaceId, "F"
);

impl CornerId {
    /// Placeholder used only while a new face is being linked. It is always
    /// overwritten before the edit returns, and it never refers to a real slot
    /// because slot `u32::MAX` is never allocated.
    pub(crate) fn dangling() -> Self {
        Self::from_parts(u32::MAX, NonZeroU32::MIN)
    }
}
