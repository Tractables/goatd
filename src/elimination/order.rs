//! Elimination-order choices.

/// Which elimination order to run after preprocessing.
///
/// The sampling orders carry one weight per graph vertex. Within a set of
/// vertices tied on the order's score, a smaller weight makes a vertex more
/// likely to be drawn and therefore eliminated earlier. Equal weights give
/// uniform sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Order<'a> {
    /// Preprocess, then repeatedly eliminate a minimum-fill vertex.
    MinFill,
    /// Preprocess, then repeatedly eliminate a minimum-degree vertex.
    MinDegree,
    /// Preprocess, then recursively bisect the graph and eliminate each
    /// separator after its two sides.
    NestedDissection,
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
    /// Degree plus fill with weighted sampling from the full minimum-score tie
    /// set.
    DegreePlusFillSampled {
        /// Per-vertex weights, one entry per input graph vertex.
        weights: &'a [u32],
    },
    /// Fill minus degree with weighted sampling from the full minimum-score
    /// tie set.
    SparsestSubgraphSampled {
        /// Per-vertex weights, one entry per input graph vertex.
        weights: &'a [u32],
    },
}

impl<'a> Order<'a> {
    /// The sampling weights carried by this order, if any.
    pub(super) fn tie_weights(self) -> Option<&'a [u32]> {
        match self {
            Order::MinFillSampled { weights }
            | Order::MinDegreeSampled { weights }
            | Order::DegreePlusFillSampled { weights }
            | Order::SparsestSubgraphSampled { weights } => Some(weights),
            Order::MinFill | Order::MinDegree | Order::NestedDissection => None,
        }
    }

    /// Replace the sampling weights after a graph has been reindexed.
    pub(super) fn with_tie_weights<'b>(self, weights: &'b [u32]) -> Order<'b> {
        match self {
            Order::MinFill => Order::MinFill,
            Order::MinDegree => Order::MinDegree,
            Order::NestedDissection => Order::NestedDissection,
            Order::MinFillSampled { .. } => Order::MinFillSampled { weights },
            Order::MinDegreeSampled { .. } => Order::MinDegreeSampled { weights },
            Order::DegreePlusFillSampled { .. } => Order::DegreePlusFillSampled { weights },
            Order::SparsestSubgraphSampled { .. } => Order::SparsestSubgraphSampled { weights },
        }
    }

    /// Whether repeated runs can reuse the residual's initial fill counts.
    pub(super) fn uses_initial_fill_cache(self) -> bool {
        matches!(
            self,
            Order::MinFillSampled { .. }
                | Order::DegreePlusFillSampled { .. }
                | Order::SparsestSubgraphSampled { .. }
        )
    }
}
