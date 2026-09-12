//! Elimination-order choices.

/// Which elimination order to run after preprocessing.
///
/// The sampling orders carry one weight per graph vertex. Within a set of
/// vertices tied on the order's score, a smaller weight makes a vertex more
/// likely to be drawn and therefore eliminated earlier. Equal weights give
/// uniform sampling.
///
/// A vertex in the tie set is drawn with mass `u32::MAX - weight + 1`, so the
/// odds are linear in the weight: weight 0 is the likeliest, and `u32::MAX` is
/// drawn about once in four billion against it, which excludes a vertex rather
/// than disfavouring it. Weights within a small factor of each other give a
/// mild bias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Order<'a> {
    /// Preprocess, then repeatedly eliminate a minimum-fill vertex.
    MinFill,
    /// Preprocess, then minimize added fill edges per removed incident edge.
    RelativeFill,
    /// Preprocess, then repeatedly eliminate a minimum-degree vertex.
    MinDegree,
    /// Preprocess, then recursively bisect the graph and eliminate each
    /// separator after its two sides.
    NestedDissection,
    /// Preprocess, then eliminate along an MCS-M numbering, which fills the
    /// residual to a minimal triangulation.
    MinimalTriangulation,
    /// Preprocess, then eliminate along a maximum cardinality search numbering.
    /// The numbering adds no fill on a chordal residual and is a cheaper
    /// relative of [`Order::MinimalTriangulation`] on any other.
    MaximumCardinality,
    /// Min-fill with weighted sampling from the full minimum-fill tie set.
    MinFillSampled {
        /// Per-vertex weights, one entry per input graph vertex.
        weights: &'a [u32],
    },
    /// Min-degree with weighted sampling from the full minimum-degree tie set.
    MinDegreeSampled {
        /// Per-vertex weights, one entry per input graph vertex.
        weights: &'a [u32],
    },
    /// Fill plus `degree_coefficient * degree`, with weighted sampling from
    /// the full minimum-score tie set.
    FillDegreeSampled {
        /// Per-vertex weights, one entry per input graph vertex.
        weights: &'a [u32],
        /// Signed coefficient of the current vertex degree.
        degree_coefficient: i8,
    },
}

impl<'a> Order<'a> {
    /// The sampling weights carried by this order, if any.
    pub(crate) fn tie_weights(self) -> Option<&'a [u32]> {
        match self {
            Order::MinFillSampled { weights }
            | Order::MinDegreeSampled { weights }
            | Order::FillDegreeSampled { weights, .. } => Some(weights),
            Order::MinFill
            | Order::RelativeFill
            | Order::MinDegree
            | Order::NestedDissection
            | Order::MinimalTriangulation
            | Order::MaximumCardinality => None,
        }
    }

    /// Replace the sampling weights after a graph has been reindexed.
    pub(super) fn with_tie_weights<'b>(self, weights: &'b [u32]) -> Order<'b> {
        match self {
            Order::MinFill => Order::MinFill,
            Order::RelativeFill => Order::RelativeFill,
            Order::MinDegree => Order::MinDegree,
            Order::NestedDissection => Order::NestedDissection,
            Order::MinimalTriangulation => Order::MinimalTriangulation,
            Order::MaximumCardinality => Order::MaximumCardinality,
            Order::MinFillSampled { .. } => Order::MinFillSampled { weights },
            Order::MinDegreeSampled { .. } => Order::MinDegreeSampled { weights },
            Order::FillDegreeSampled {
                degree_coefficient, ..
            } => Order::FillDegreeSampled {
                weights,
                degree_coefficient,
            },
        }
    }

    /// Whether the order breaks ties with the per-vertex salt; a sampled
    /// order draws its ties from its own stream instead.
    pub(super) fn uses_salt(self) -> bool {
        matches!(
            self,
            Order::MinFill | Order::RelativeFill | Order::MinDegree | Order::NestedDissection
        )
    }

    /// Whether repeated runs can reuse the residual's initial fill counts.
    pub(super) fn uses_initial_fill_cache(self) -> bool {
        matches!(
            self,
            Order::MinFillSampled { .. } | Order::FillDegreeSampled { .. }
        )
    }
}
