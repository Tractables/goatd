use std::cell::OnceCell;

use super::*;
use crate::decomposition::polishing::{self, Advance, Budget, Progress, Proposal, Recipient};

/// Separator search and recursion settings for a [`Session`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct Config {
    per_vertex: u64,
    min_steps: u64,
    max_steps: u64,
    iterations: u32,
    min_vertices: usize,
    max_vertices: Option<usize>,
    depth: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            per_vertex: REFINEMENT_STEPS_PER_VERTEX,
            min_steps: MIN_REFINEMENT_STEPS,
            max_steps: MAX_REFINEMENT_STEPS,
            iterations: FLOWCUTTER_REFINEMENT_ITERATIONS,
            min_vertices: MIN_REFINEMENT_VERTICES,
            max_vertices: None,
            depth: MAX_RECURSION_DEPTH,
        }
    }
}

impl Config {
    /// Set the per-region separator effort to `vertices * per_vertex`, clamped
    /// to `[minimum, maximum]`. These are separator work units, distinct from
    /// the scheduling operations counted by [`Budget`].
    pub const fn with_search_steps(mut self, per_vertex: u64, minimum: u64, maximum: u64) -> Self {
        self.per_vertex = per_vertex;
        self.min_steps = minimum;
        self.max_steps = maximum;
        self
    }

    /// Cap source/sink restarts within each region's separator search.
    pub const fn with_iterations(mut self, iterations: u32) -> Self {
        self.iterations = iterations;
        self
    }

    /// Search regions within these vertex-count bounds; `None` has no upper bound.
    pub const fn with_vertex_limits(mut self, minimum: usize, maximum: Option<usize>) -> Self {
        self.min_vertices = minimum;
        self.max_vertices = maximum;
        self
    }

    /// Limit nested separator replacements. Zero leaves the input unchanged.
    pub const fn with_max_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// Check effort, iteration and vertex limits without starting a search.
    ///
    /// # Errors
    /// Returns an error for zero minimum effort or iterations, unrepresentable
    /// effort or iteration counts, or reversed effort or vertex bounds.
    pub fn validate(self) -> Result<(), Error> {
        if self.min_steps == 0
            || self.max_steps < self.min_steps
            || self.max_steps > i64::MAX as u64
            || self.iterations == 0
            || self.iterations > i32::MAX as u32
            || self.max_vertices.is_some_and(|max| max < self.min_vertices)
        {
            return Err(Error::InvalidInput(
                "invalid separator effort, iteration or vertex limits".into(),
            ));
        }
        Ok(())
    }

    fn eligible(self, vertices: usize, depth: u32) -> bool {
        depth < self.depth
            && vertices >= self.min_vertices
            && self.max_vertices.is_none_or(|max| vertices <= max)
    }
}

/// Resumable separator refinement with complete original-graph proposals.
///
/// One scheduling step prepares a region, initializes a source/sink restart,
/// advances its cut trajectory, or finishes a recursive replacement. Search
/// state survives pauses, including an unfinished cutter trajectory. A completed
/// restart offers its candidate before width or bag-size acceptance; rejecting
/// it continues the same search. Accepting descends into the proposed sides.
///
/// Recursive candidates are glued into their surrounding decomposition before
/// [`Proposal::candidate`] exposes them. This assembly is lazy and can be more
/// expensive than the local rewrite. Account for it and scoring in the caller's
/// total budget. Constructor validation and individual graph/cutter operations
/// are indivisible; real deadlines are cooperative rather than preemptive.
pub struct Session<'g> {
    graph: &'g Graph,
    config: Config,
    nodes: Vec<Option<Node>>,
    free: Vec<usize>,
    tasks: Vec<Task>,
    active: Option<Active>,
    pending: Option<Pending>,
    current: OnceCell<TreeDecomposition>,
    progress: Progress,
}

struct Node {
    vertices: Vec<u32>,
    depth: u32,
    kind: Kind,
}
enum Kind {
    Leaf(TreeDecomposition),
    Branch {
        left: usize,
        right: usize,
        separator: Vec<u32>,
        fallback: TreeDecomposition,
    },
}
#[derive(Clone, Copy)]
enum Task {
    Visit(usize),
    Finish(usize),
}
struct Active {
    node: usize,
    graph: Graph,
    search: separator::Search,
}
struct Split {
    node: usize,
    left: TreeDecomposition,
    right: TreeDecomposition,
    left_vertices: Vec<u32>,
    right_vertices: Vec<u32>,
    separator: Vec<u32>,
}
enum Pending {
    Split {
        split: Split,
        finish_on_reject: bool,
    },
    Fallback {
        node: usize,
        refined: TreeDecomposition,
    },
}

impl<'g> Session<'g> {
    /// Validate the graph/decomposition pair and configure its search.
    ///
    /// # Errors
    /// Returns an error for an invalid decomposition, invalid settings or a
    /// searched graph too large for FlowCutter's expanded index space.
    pub fn new(graph: &'g Graph, tree: TreeDecomposition, config: Config) -> Result<Self, Error> {
        let start = Instant::now();
        let units = crate::meter::units_spent();
        tree.validate(graph)?;
        let mut session = Self::trusted(graph, tree, config)?;
        session.progress.elapsed = start.elapsed();
        session.progress.setup_elapsed = session.progress.elapsed;
        session.progress.work_units = crate::meter::units_spent().saturating_sub(units);
        Ok(session)
    }

    pub(super) fn trusted(
        graph: &'g Graph,
        tree: TreeDecomposition,
        config: Config,
    ) -> Result<Self, Error> {
        config.validate()?;
        if config.eligible(graph.num_vertices() as usize, 0) {
            separator::validate_graph_size(graph.num_vertices(), graph.edges().len())?;
        }
        Ok(Self {
            graph,
            config,
            nodes: vec![Some(Node {
                vertices: (0..graph.num_vertices()).collect(),
                depth: 0,
                kind: Kind::Leaf(tree),
            })],
            free: Vec::new(),
            tasks: vec![Task::Visit(0)],
            active: None,
            pending: None,
            current: OnceCell::new(),
            progress: Progress::default(),
        })
    }

    /// The complete accepted incumbent. Assembly after recursive acceptance is lazy.
    pub fn current(&self) -> &TreeDecomposition {
        self.current.get_or_init(|| self.materialize(0, None))
    }

    /// Search measurements, including `offer_current`. Lazy tree assembly,
    /// proposal acceptance and final extraction are outside these measurements;
    /// time the whole session lifecycle when comparing end-to-end cost.
    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// Original-graph vertices of the next region, when the next task is a search.
    pub fn region_vertices(&self) -> Option<&[u32]> {
        match self.tasks.last()? {
            Task::Visit(node) => Some(&self.node(*node).vertices),
            Task::Finish(_) => None,
        }
    }

    /// Change settings for subsequent searches without restarting an active one.
    ///
    /// # Errors
    /// Returns an error while a separator search is active, for invalid settings,
    /// or when the graph exceeds a newly enabled search's index space.
    pub fn set_config(&mut self, config: Config) -> Result<(), Error> {
        if self.active.is_some() {
            return Err(Error::InvalidInput(
                "finish or accept the active separator search before changing its settings".into(),
            ));
        }
        config.validate()?;
        if config.eligible(self.graph.num_vertices() as usize, 0) {
            separator::validate_graph_size(self.graph.num_vertices(), self.graph.edges().len())?;
        }
        self.config = config;
        Ok(())
    }

    /// Continue until a proposal, a budget pause or exhaustion.
    pub fn advance(&mut self, budget: Budget) -> Advance<'_> {
        self.advance_inner(budget, false, None)
    }

    /// Offer the best separator found so far without advancing or restarting it.
    /// Deriving this proposal has a cost, recorded as one scheduling step. A
    /// repeated call can offer the same candidate; acceptance remains the caller's.
    pub fn offer_current(&mut self) -> Option<Proposal<'_>> {
        let mut slice = Budget::new(1).begin();
        let active = self.active.as_ref()?;
        let vertices = active.search.best_vertices()?.to_vec();
        let derived = self.make_split(active.node, &active.graph, vertices);
        slice.step();
        slice.record(&mut self.progress);
        let (candidate, split, recommended) = derived?;
        self.pending = Some(Pending::Split {
            split,
            finish_on_reject: false,
        });
        self.progress.proposed += 1;
        Some(polishing::new_proposal(self, candidate, recommended))
    }

    /// Consume the session and transfer its accepted complete tree. Unfinished
    /// searches and their tree-dependent state are discarded.
    pub fn into_tree(mut self) -> TreeDecomposition {
        if let Some(tree) = self.current.take() {
            return tree;
        }
        if matches!(self.node(0).kind, Kind::Leaf(_)) {
            let node = self.nodes[0].take().expect("the root remains live");
            if let Kind::Leaf(tree) = node.kind {
                return tree;
            }
            unreachable!();
        }
        self.materialize(0, None)
    }

    pub(super) fn advance_legacy(&mut self, deadline: Option<Instant>) -> Advance<'_> {
        self.advance_inner(Budget::new(u64::MAX), true, deadline)
    }

    fn advance_inner(
        &mut self,
        budget: Budget,
        legacy: bool,
        deadline: Option<Instant>,
    ) -> Advance<'_> {
        let mut slice = budget.begin();
        let _wall = slice.wall_guard();
        loop {
            let Some(task) = self.tasks.last().copied() else {
                slice.record(&mut self.progress);
                return Advance::Exhausted;
            };
            if !legacy && let Some(reason) = slice.pause() {
                slice.record(&mut self.progress);
                return Advance::Paused(reason);
            }
            match task {
                Task::Finish(node) => {
                    self.tasks.pop();
                    let refined = self.materialize(node, None);
                    let Kind::Branch { fallback, .. } = &self.node(node).kind else {
                        unreachable!();
                    };
                    let candidate =
                        (refined.quality_key() > fallback.quality_key()).then(|| fallback.clone());
                    slice.step();
                    if let Some(candidate) = candidate {
                        self.pending = Some(Pending::Fallback { node, refined });
                        self.progress.proposed += 1;
                        slice.record(&mut self.progress);
                        return polishing::proposal(self, candidate, true);
                    }
                    self.collapse(node, refined);
                }
                Task::Visit(node) => {
                    if (legacy && expired(deadline))
                        || !self
                            .config
                            .eligible(self.node(node).vertices.len(), self.node(node).depth)
                    {
                        self.active = None;
                        self.tasks.pop();
                        slice.step();
                        continue;
                    }
                    if self.active.is_none() {
                        let start = Instant::now();
                        let graph = self
                            .graph
                            .induced_subgraph(&self.node(node).vertices)
                            .expect("regions keep unique original-graph vertices");
                        let steps = self
                            .config
                            .per_vertex
                            .saturating_mul(graph.num_vertices() as u64)
                            .clamp(self.config.min_steps, self.config.max_steps);
                        let search = separator::Search::new(
                            graph.num_vertices() as usize,
                            graph.edges(),
                            steps as i64,
                            self.config.iterations as i32,
                        );
                        self.progress.setup_elapsed += start.elapsed();
                        slice.step();
                        if let Some(search) = search {
                            self.active = Some(Active {
                                node,
                                graph,
                                search,
                            });
                        } else {
                            self.tasks.pop();
                        }
                        continue;
                    }
                    let active = self.active.as_mut().expect("search was prepared");
                    if active.search.exhausted() {
                        let active = self
                            .active
                            .take()
                            .expect("completed search remains available");
                        let best = active.search.into_vertices();
                        if legacy
                            && let Some(vertices) = best
                            && let Some((candidate, split, recommended)) =
                                self.make_split(node, &active.graph, vertices)
                        {
                            self.pending = Some(Pending::Split {
                                split,
                                finish_on_reject: true,
                            });
                            self.progress.proposed += 1;
                            slice.step();
                            slice.record(&mut self.progress);
                            return polishing::proposal(self, candidate, recommended);
                        }
                        self.tasks.pop();
                        slice.step();
                        continue;
                    }
                    let before = active.search.attempts();
                    let candidate = active.search.step(!legacy);
                    self.progress.attempted += active.search.attempts() - before;
                    slice.step();
                    if let Some(vertices) = candidate {
                        let active = self
                            .active
                            .as_ref()
                            .expect("a search owns its restart result");
                        if let Some((candidate, split, recommended)) =
                            self.make_split(node, &active.graph, vertices)
                        {
                            self.pending = Some(Pending::Split {
                                split,
                                finish_on_reject: false,
                            });
                            self.progress.proposed += 1;
                            slice.record(&mut self.progress);
                            return polishing::proposal(self, candidate, recommended);
                        }
                    }
                }
            }
        }
    }

    fn node(&self, id: usize) -> &Node {
        self.nodes[id]
            .as_ref()
            .expect("tasks reference live regions")
    }

    fn insert(&mut self, node: Node) -> usize {
        if let Some(id) = self.free.pop() {
            self.nodes[id] = Some(node);
            id
        } else {
            let id = self.nodes.len();
            self.nodes.push(Some(node));
            id
        }
    }

    fn collapse(&mut self, node: usize, tree: TreeDecomposition) {
        let old = std::mem::replace(
            &mut self.nodes[node].as_mut().expect("live region").kind,
            Kind::Leaf(tree),
        );
        if let Kind::Branch { left, right, .. } = old {
            self.nodes[left] = None;
            self.nodes[right] = None;
            self.free.extend([left, right]);
        }
    }

    fn make_split(
        &self,
        node: usize,
        graph: &Graph,
        vertices: Vec<u32>,
    ) -> Option<(TreeDecomposition, Split, bool)> {
        let separator = separator::with_sides(graph, vertices)?;
        let region = self.node(node);
        let Kind::Leaf(tree) = &region.kind else {
            unreachable!();
        };
        let sep = to_global_vertices(separator.vertices(), &region.vertices);
        let mut left_vertices = to_global_vertices(separator.side_a(), &region.vertices);
        let mut right_vertices = to_global_vertices(separator.side_b(), &region.vertices);
        left_vertices.extend_from_slice(&sep);
        right_vertices.extend_from_slice(&sep);
        let left = project_td_keeping_global_ids(tree, &left_vertices)?;
        let right = project_td_keeping_global_ids(tree, &right_vertices)?;
        let candidate = glue_at_separator(left.clone(), right.clone(), &sep)?;
        let recommended = candidate.quality_key() < tree.quality_key();
        Some((
            candidate,
            Split {
                node,
                left,
                right,
                left_vertices,
                right_vertices,
                separator: sep,
            },
            recommended,
        ))
    }

    fn materialize(
        &self,
        root: usize,
        replacement: Option<(usize, &TreeDecomposition)>,
    ) -> TreeDecomposition {
        let mut tasks = vec![(root, false)];
        let mut trees = Vec::new();
        while let Some((id, finish)) = tasks.pop() {
            if let Some((node, tree)) = replacement
                && id == node
            {
                trees.push(tree.clone());
                continue;
            }
            match &self.node(id).kind {
                Kind::Leaf(tree) => trees.push(tree.clone()),
                Kind::Branch {
                    left,
                    right,
                    separator,
                    ..
                } => {
                    if finish {
                        let b = trees.pop().expect("right subtree assembled");
                        let a = trees.pop().expect("left subtree assembled");
                        trees.push(
                            glue_at_separator(a, b, separator)
                                .expect("accepted regions share their separator"),
                        );
                    } else {
                        tasks.extend([(id, true), (*right, false), (*left, false)]);
                    }
                }
            }
        }
        trees
            .pop()
            .expect("the region has a complete decomposition")
    }
}

impl Recipient for Session<'_> {
    fn current(&self) -> &TreeDecomposition {
        Session::current(self)
    }
    fn candidate_is_complete(&self) -> bool {
        false
    }
    fn complete(&self, candidate: &TreeDecomposition) -> TreeDecomposition {
        let node = match self
            .pending
            .as_ref()
            .expect("proposal has a pending replacement")
        {
            Pending::Split { split, .. } => split.node,
            Pending::Fallback { node, .. } => *node,
        };
        self.materialize(0, Some((node, candidate)))
    }

    fn accept(&mut self, candidate: TreeDecomposition) {
        match self
            .pending
            .take()
            .expect("proposal has a pending replacement")
        {
            Pending::Split { split, .. } => {
                self.active = None;
                self.tasks.pop();
                let depth = self.node(split.node).depth + 1;
                let left = self.insert(Node {
                    vertices: split.left_vertices,
                    depth,
                    kind: Kind::Leaf(split.left),
                });
                let right = self.insert(Node {
                    vertices: split.right_vertices,
                    depth,
                    kind: Kind::Leaf(split.right),
                });
                self.nodes[split.node].as_mut().expect("live region").kind = Kind::Branch {
                    left,
                    right,
                    separator: split.separator,
                    fallback: candidate,
                };
                self.tasks.extend([
                    Task::Finish(split.node),
                    Task::Visit(right),
                    Task::Visit(left),
                ]);
            }
            Pending::Fallback { node, .. } => self.collapse(node, candidate),
        }
        self.current.take();
        self.progress.accepted += 1;
    }

    fn reject(&mut self) {
        match self
            .pending
            .take()
            .expect("proposal has a pending replacement")
        {
            Pending::Split {
                finish_on_reject: true,
                ..
            } => {
                self.active = None;
                self.tasks.pop();
            }
            Pending::Split {
                finish_on_reject: false,
                ..
            } => {}
            Pending::Fallback { node, refined } => self.collapse(node, refined),
        }
    }
}
