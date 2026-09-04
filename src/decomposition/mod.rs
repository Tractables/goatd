//! Tree-decomposition data, validation, projection, and refinement.
//!
//! Partition refinement changes a temporary two-way partition. This module
//! instead operates on complete tree decompositions: [`refine_with_flowcutter`]
//! finds separators and rewrites bags, [`minimalize_triangulation`] drops the
//! fill edges the bags do not need, and [`TreeDecomposition::project`]
//! restricts a decomposition to a vertex subset.

mod minimal;
mod model;
mod ops;
mod refine;

pub use minimal::minimalize_triangulation;
pub(crate) use minimal::{minimalize_at, minimalize_fits};
pub use model::{TdBag, TreeDecomposition};
pub(crate) use ops::SubsumedBagCompaction;
pub use ops::{Projection, RootedForest};
pub use refine::refine_with_flowcutter;
