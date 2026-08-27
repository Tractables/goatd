//! Tree-decomposition data, validation, projection, and refinement.
//!
//! Partition refinement changes a temporary two-way partition. This module
//! instead operates on complete tree decompositions: [`refine_with_flowcutter`]
//! finds separators and rewrites bags, while [`TreeDecomposition::project`]
//! restricts a decomposition to a vertex subset.

mod model;
mod ops;
mod refine;

pub use model::{TdBag, TreeDecomposition};
pub use ops::{Projection, RootedForest};
pub use refine::refine_with_flowcutter;
