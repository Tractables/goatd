//! An elimination as it went: the order the vertices left the graph in and
//! the neighbours each had when it left, rather than the decomposition built
//! from them.

use super::execution::{self, ElimExit};
use super::graph::EliminationGraph;
use super::prepared::{RunConfig, validate_order};
use super::{Cutoff, Order, engine};
use crate::{Error, Graph, TreeDecomposition, deadline, meter};

/// The record of one elimination of every vertex of a graph.
///
/// The first [`neighbours`](Self::neighbours)`.len()` vertices of
/// [`order`](Self::order) were eliminated one at a time, each with the
/// neighbours listed for it. Where a cutoff stopped the elimination, the
/// vertices after them were left, and they follow by rank (the order's
/// ranks, where it has them) and then by number: a decomposition puts them
/// in one bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    pub(super) order: Vec<u32>,
    pub(super) neighbours: Vec<Vec<u32>>,
}

/// How a traced elimination ended.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceOutcome {
    /// Every vertex was eliminated by the order.
    Completed(Trace),
    /// A cutoff stopped the order; the vertices left follow in one group.
    CompletedAtDeadline(Cutoff, Trace),
    /// A cutoff stopped a run configured not to complete partial work.
    DeadlineAborted(Cutoff),
    /// A vertex had more neighbours than the caller's width bound.
    WidthAborted,
}

impl Trace {
    /// Every vertex, in the order eliminated, those a cutoff left last.
    pub fn order(&self) -> &[u32] {
        &self.order
    }

    /// The neighbours of each vertex the order eliminated, when it went, in
    /// the order eliminated.
    pub fn neighbours(&self) -> &[Vec<u32>] {
        &self.neighbours
    }

    /// The order and the neighbours, as [`order`](Self::order) and
    /// [`neighbours`](Self::neighbours) give them.
    pub fn into_parts(self) -> (Vec<u32>, Vec<Vec<u32>>) {
        (self.order, self.neighbours)
    }

    /// The tree decomposition of `graph`, the graph eliminated, that the
    /// elimination gives, its bags numbered from the last to go: a bag for
    /// each vertex eliminated, holding it and its neighbours when it went,
    /// whose parent is the bag of the first of those neighbours to go, and a
    /// bag for the vertices a cutoff left. Bag 0, the last vertex's or the
    /// one of the vertices left, is a root, and a parent's number is below
    /// its children's. The decompositions [`decompose`](super::decompose)
    /// returns number their bags from the first to go.
    ///
    /// # Errors
    ///
    /// Returns an error when the trace names a vertex outside `graph`.
    pub fn decomposition(&self, graph: &Graph) -> Result<TreeDecomposition, Error> {
        let n = self.order.len();
        if n != graph.num_vertices() as usize {
            return Err(Error::InvalidInput(format!(
                "a trace of {n} vertices for a graph of {}",
                graph.num_vertices()
            )));
        }
        let exact = self.neighbours.len();
        let mut position = vec![0usize; n];
        for (i, &v) in self.order.iter().enumerate() {
            position[v as usize] = i;
        }
        let num_bags = exact + usize::from(exact < n);
        let last = num_bags.saturating_sub(1);
        let bag_of = |u: u32| position[u as usize].min(exact);
        let mut bags = Vec::with_capacity(num_bags);
        let mut tree_edges = Vec::with_capacity(num_bags);
        for (i, (&v, around)) in self.order.iter().zip(&self.neighbours).enumerate() {
            if let Some(&first) = around.iter().min_by_key(|&&u| position[u as usize]) {
                tree_edges.push((last - i, last - bag_of(first)));
            }
            bags.push(around.iter().copied().chain([v]).collect::<Vec<u32>>());
        }
        if exact < n {
            bags.push(self.order[exact..].to_vec());
        }
        bags.reverse();
        meter::charge(n as u64 + self.neighbours.iter().map(|a| a.len() as u64).sum::<u64>());
        TreeDecomposition::new_trusted(graph, bags, tree_edges)
    }
}

/// Run one elimination order over every vertex of `graph`, without the
/// reductions ([`Prepared::unreduced`](super::Prepared::unreduced)) and
/// without splitting the graph into its connected components, and return the
/// order it took and each vertex's neighbours when it went ([`Trace`]).
///
/// The ranked orders ([`Order::MinFillRanked`], [`Order::MinFillWeighted`],
/// [`Order::Ranked`]) then decide every step: the same graph and ranks give
/// the same trace. Budgets, the width bound and deadline completion are read
/// from `config` as [`Prepared::run`](super::Prepared::run) reads them;
/// `seed` as [`decompose`](super::decompose) reads it.
///
/// # Errors
///
/// Returns an error for an order whose weights or ranks do not fit the graph,
/// and for budgets [`Prepared::run`](super::Prepared::run) refuses.
pub fn trace(
    graph: &Graph,
    order: Order<'_>,
    seed: u64,
    config: RunConfig,
) -> Result<TraceOutcome, Error> {
    validate_order(graph, order)?;
    let deadlines = deadline::staged(
        meter::now(),
        config.soft_budget,
        config.hard_budget,
        "elimination",
    )?;
    let mut eliminated = EliminationGraph::from_edges(graph.num_vertices, &graph.edges);
    let (bags, exit, mut rest) = engine::run_traced(
        &mut eliminated,
        engine::RunSpec {
            order,
            seed,
            sample_band: 0,
            update_order_ties: false,
            stop: execution::ElimStop {
                soft_deadline: deadlines.soft,
                hard_deadline: deadlines.hard,
                width_bound: config.width_bound,
            },
            complete_on_deadline: config.complete_on_deadline,
            setup_deadline: if config.complete_on_deadline {
                None
            } else {
                deadlines.hard
            },
        },
    );
    let cutoff = match exit {
        ElimExit::WidthLimitExceeded => return Ok(TraceOutcome::WidthAborted),
        ElimExit::DeadlineReached(cutoff) if !config.complete_on_deadline => {
            return Ok(TraceOutcome::DeadlineAborted(cutoff));
        }
        ElimExit::DeadlineReached(cutoff) => Some(cutoff),
        ElimExit::Complete => None,
    };
    if config
        .width_bound
        .is_some_and(|bound| rest.len() > bound as usize + 1)
    {
        return Ok(TraceOutcome::WidthAborted);
    }
    match order.tie_ranks() {
        Some(ranks) => rest.sort_unstable_by_key(|&v| (ranks[v as usize], v)),
        None => rest.sort_unstable(),
    }
    let mut trace = Trace {
        order: Vec::with_capacity(graph.num_vertices() as usize),
        neighbours: Vec::with_capacity(bags.len()),
    };
    meter::charge(bags.iter().map(|bag| bag.len() as u64).sum::<u64>());
    for mut bag in bags {
        trace.order.push(bag[0]);
        bag.remove(0);
        trace.neighbours.push(bag);
    }
    trace.order.extend(rest);
    Ok(match cutoff {
        None => TraceOutcome::Completed(trace),
        Some(cutoff) => TraceOutcome::CompletedAtDeadline(cutoff, trace),
    })
}
