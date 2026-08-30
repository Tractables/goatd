//! Python bindings for goatd: a graph goes in and a tree decomposition comes
//! out.
//!
//! Every construction and every knob here is one the `goatd` solver exposes,
//! under the same name. Nothing in this crate decides anything about a
//! decomposition; it checks arguments, calls the library, and converts the
//! result back.

use std::time::{Duration, Instant};

use goatd::elimination::{Order, decompose as eliminate};
use goatd::flowcutter::{Budget, decompose as flowcutter};
use goatd::portfolio::{PortfolioConfig, decompose as portfolio};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;

create_exception!(
    goatd,
    Error,
    PyException,
    "An invalid input, malformed PACE text, invalid decomposition, oversized \
     problem, or failed FlowCutter construction."
);

fn py_error(error: goatd::Error) -> PyErr {
    Error::new_err(error.to_string())
}

/// An undirected graph over the vertices `0..num_vertices`, as an edge list.
#[pyclass(module = "goatd", frozen)]
pub struct Graph {
    inner: goatd::Graph,
}

#[pymethods]
impl Graph {
    /// The graph over `0..num_vertices` with these edges, given as pairs in
    /// either orientation and any order. Self-loops are dropped and a repeated
    /// undirected edge is kept once.
    #[new]
    fn new(num_vertices: u32, edges: Vec<(u32, u32)>) -> PyResult<Self> {
        goatd::Graph::try_new(num_vertices, edges)
            .map(|inner| Self { inner })
            .map_err(py_error)
    }

    /// Read a PACE `.gr` graph.
    #[staticmethod]
    fn from_gr(text: &str) -> PyResult<Self> {
        goatd::Graph::from_gr(text)
            .map(|inner| Self { inner })
            .map_err(py_error)
    }

    /// Render as a PACE `.gr` graph, whose vertices are 1-indexed.
    fn to_gr(&self) -> String {
        self.inner.to_gr()
    }

    /// Number of vertices, with ids `0..num_vertices`.
    #[getter]
    fn num_vertices(&self) -> u32 {
        self.inner.num_vertices()
    }

    /// The edges, sorted and deduplicated, each with the smaller endpoint
    /// first.
    #[getter]
    fn edges(&self) -> Vec<(u32, u32)> {
        self.inner.edges().to_vec()
    }

    fn __repr__(&self) -> String {
        format!(
            "<goatd.Graph: {} vertices, {} edges>",
            self.inner.num_vertices(),
            self.inner.edges().len()
        )
    }
}

/// A tree decomposition: bags of graph vertices, and acyclic edges between the
/// bags.
#[pyclass(module = "goatd", frozen)]
pub struct TreeDecomposition {
    inner: goatd::TreeDecomposition,
}

#[pymethods]
impl TreeDecomposition {
    /// Build a decomposition of `graph` and check it. `bags` holds the graph
    /// vertices of each bag; `edges` are undirected pairs of positions in
    /// `bags`.
    #[new]
    fn new(graph: &Graph, bags: Vec<Vec<u32>>, edges: Vec<(usize, usize)>) -> PyResult<Self> {
        goatd::TreeDecomposition::new(&graph.inner, bags, edges)
            .map(|inner| Self { inner })
            .map_err(py_error)
    }

    /// Read a PACE `.td` decomposition. The text carries no graph, so call
    /// `validate` to check the result against the graph it should decompose.
    #[staticmethod]
    fn from_td(text: &str) -> PyResult<Self> {
        goatd::TreeDecomposition::from_td(text)
            .map(|inner| Self { inner })
            .map_err(py_error)
    }

    /// Render as a PACE `.td` decomposition, whose bags and vertices are
    /// 1-indexed.
    fn to_td(&self) -> String {
        self.inner.to_td()
    }

    /// Number of vertices in the graph this decomposition was built for.
    #[getter]
    fn num_vertices(&self) -> u32 {
        self.inner.num_vertices()
    }

    /// The bags, as lists of graph vertices, in bag order.
    #[getter]
    fn bags(&self) -> Vec<Vec<u32>> {
        self.inner
            .bags()
            .iter()
            .map(|bag| bag.vertices().to_vec())
            .collect()
    }

    /// The edges between bags, as pairs of positions in `bags` with the
    /// smaller position first.
    #[getter]
    fn edges(&self) -> Vec<(usize, usize)> {
        let mut edges = Vec::new();
        for (bag, neighbours) in self.inner.adjacency().iter().enumerate() {
            for &neighbour in neighbours {
                if bag < neighbour {
                    edges.push((bag, neighbour));
                }
            }
        }
        edges.sort_unstable();
        edges
    }

    /// The vertices in the largest bag, less one: an upper bound on the
    /// treewidth of the decomposed graph.
    #[getter]
    fn treewidth(&self) -> u32 {
        self.inner.treewidth()
    }

    /// Sum of the bag sizes, the second quality signal beside the width.
    #[getter]
    fn total_bag_size(&self) -> usize {
        self.inner.total_bag_size()
    }

    /// Check that this is a tree decomposition of `graph`, raising `Error`
    /// with the first violation found.
    fn validate(&self, graph: &Graph) -> PyResult<()> {
        self.inner.validate(&graph.inner).map_err(py_error)
    }

    fn __repr__(&self) -> String {
        format!(
            "<goatd.TreeDecomposition: {} bags, width {}>",
            self.inner.bags().len(),
            self.inner.treewidth()
        )
    }
}

/// Which construction `order` named.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    MinFill,
    MinDegree,
    NestedDissection,
    FlowCutter,
    Portfolio,
}

impl Method {
    fn parse(name: &str) -> PyResult<Self> {
        match name {
            "minfill" => Ok(Method::MinFill),
            "mindegree" => Ok(Method::MinDegree),
            "nested-dissection" => Ok(Method::NestedDissection),
            "flowcutter" => Ok(Method::FlowCutter),
            "portfolio" => Ok(Method::Portfolio),
            _ => Err(PyValueError::new_err(format!(
                "unknown order {name:?}; use minfill, mindegree, \
                 nested-dissection, flowcutter or portfolio"
            ))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Method::MinFill => "minfill",
            Method::MinDegree => "mindegree",
            Method::NestedDissection => "nested-dissection",
            Method::FlowCutter => "flowcutter",
            Method::Portfolio => "portfolio",
        }
    }
}

/// The arguments after the chosen order has accepted them.
struct Knobs {
    order: Method,
    seed: u64,
    /// One weight per vertex when the order samples its ties, else `None`.
    weights: Option<Vec<u32>>,
    budget: Option<Duration>,
    steps: Option<u64>,
}

/// Reject an argument the chosen order cannot act on, naming both it and the
/// orders that take it.
fn only_with(argument: &str, accepted: bool, order: Method, orders: &str) -> PyResult<()> {
    if accepted {
        return Ok(());
    }
    Err(PyValueError::new_err(format!(
        "{argument} is not valid with order={:?}; it applies to {orders}",
        order.name()
    )))
}

/// Check the arguments against the chosen order and fill in the defaults.
fn knobs(
    num_vertices: u32,
    order: &str,
    seed: Option<u64>,
    ties: Option<&str>,
    weights: Option<Vec<u32>>,
    budget_ms: Option<u64>,
    steps: Option<u64>,
) -> PyResult<Knobs> {
    let order = Method::parse(order)?;
    let greedy = matches!(order, Method::MinFill | Method::MinDegree);

    if let Some(ties) = ties {
        if ties != "sample" {
            return Err(PyValueError::new_err(format!(
                "ties takes only \"sample\", got {ties:?}"
            )));
        }
        only_with("ties", greedy, order, "minfill and mindegree")?;
    }
    if weights.is_some() {
        only_with("weights", greedy, order, "minfill and mindegree")?;
        if ties.is_none() {
            return Err(PyValueError::new_err("weights requires ties=\"sample\""));
        }
    }
    if seed.is_some() {
        only_with(
            "seed",
            order != Method::FlowCutter,
            order,
            "minfill, mindegree, nested-dissection and portfolio",
        )?;
    }
    if let Some(steps) = steps {
        only_with("steps", order == Method::FlowCutter, order, "flowcutter")?;
        if steps == 0 {
            return Err(PyValueError::new_err("steps wants a positive step count"));
        }
        if budget_ms.is_some() {
            return Err(PyValueError::new_err(
                "steps and budget_ms both bound flowcutter; give one",
            ));
        }
    }
    if budget_ms == Some(0) {
        return Err(PyValueError::new_err(
            "budget_ms wants a positive millisecond count",
        ));
    }

    // Equal weights make the sampling uniform, which is what the solver does
    // for `--ties sample` without `--weights`.
    let weights = match (ties, weights) {
        (Some(_), Some(weights)) => Some(weights),
        (Some(_), None) => Some(vec![1; num_vertices as usize]),
        (None, _) => None,
    };

    Ok(Knobs {
        order,
        seed: seed.unwrap_or(0),
        weights,
        budget: budget_ms.map(Duration::from_millis),
        steps,
    })
}

fn construct(
    graph: &goatd::Graph,
    knobs: &Knobs,
) -> Result<goatd::TreeDecomposition, goatd::Error> {
    match knobs.order {
        Method::MinFill | Method::MinDegree => {
            let order = match (knobs.order, knobs.weights.as_deref()) {
                (Method::MinFill, None) => Order::MinFill,
                (Method::MinFill, Some(weights)) => Order::MinFillSampled { weights },
                (_, None) => Order::MinDegree,
                (_, Some(weights)) => Order::MinDegreeSampled { weights },
            };
            eliminate(graph, order, knobs.seed, knobs.budget)
        }
        Method::NestedDissection => {
            eliminate(graph, Order::NestedDissection, knobs.seed, knobs.budget)
        }
        Method::FlowCutter => flowcutter(graph, Budget::standalone(knobs.budget, knobs.steps)),
        Method::Portfolio => {
            let weights = vec![1; graph.num_vertices() as usize];
            let config = knobs
                .budget
                .map_or_else(PortfolioConfig::standard, |budget| {
                    PortfolioConfig::standard().with_soft_budget(budget)
                });
            portfolio(graph, &weights, knobs.seed, config)
        }
    }
}

/// Decompose `graph` and return the result.
///
/// `order` is one of `"minfill"`, `"mindegree"`, `"nested-dissection"`,
/// `"flowcutter"` and `"portfolio"`. `seed` breaks ties for every order but
/// flowcutter. `ties="sample"` makes minfill and mindegree draw from the whole
/// tie set instead of breaking ties by salt, and `weights` then gives one
/// integer per vertex, a smaller weight being eliminated earlier.
///
/// `budget_ms` is the elimination orders' soft deadline, flowcutter's run
/// time, and the portfolio's soft deadline; what is left of it bounds the
/// refinement. `steps` replaces flowcutter's clock with a step count, for a
/// run that repeats exactly. `refine=True` re-cuts the result along FlowCutter
/// separators before returning it.
///
/// An argument the chosen order cannot act on raises `ValueError` naming both.
/// The interpreter lock is released for the whole construction.
#[pyfunction]
#[pyo3(signature = (
    graph,
    *,
    order = "minfill",
    seed = None,
    ties = None,
    weights = None,
    budget_ms = None,
    steps = None,
    refine = false
))]
#[allow(clippy::too_many_arguments)]
fn decompose(
    py: Python<'_>,
    graph: &Graph,
    order: &str,
    seed: Option<u64>,
    ties: Option<&str>,
    weights: Option<Vec<u32>>,
    budget_ms: Option<u64>,
    steps: Option<u64>,
    refine: bool,
) -> PyResult<TreeDecomposition> {
    let graph = &graph.inner;
    let knobs = knobs(
        graph.num_vertices(),
        order,
        seed,
        ties,
        weights,
        budget_ms,
        steps,
    )?;
    let start = Instant::now();
    let inner = py
        .detach(|| -> Result<goatd::TreeDecomposition, goatd::Error> {
            let td = construct(graph, &knobs)?;
            if !refine {
                return Ok(td);
            }
            let left = knobs
                .budget
                .map(|budget| budget.saturating_sub(start.elapsed()));
            goatd::decomposition::refine_with_flowcutter(td, graph, left)
        })
        .map_err(py_error)?;
    Ok(TreeDecomposition { inner })
}

/// Re-cut `td` along FlowCutter separators and return the result.
///
/// `budget_ms` bounds the pass; without one it runs to completion. The pass is
/// anytime, so a decomposition it cannot improve comes back unchanged. The
/// interpreter lock is released while it runs.
#[pyfunction]
#[pyo3(signature = (td, graph, *, budget_ms = None))]
fn refine_with_flowcutter(
    py: Python<'_>,
    td: &TreeDecomposition,
    graph: &Graph,
    budget_ms: Option<u64>,
) -> PyResult<TreeDecomposition> {
    if budget_ms == Some(0) {
        return Err(PyValueError::new_err(
            "budget_ms wants a positive millisecond count",
        ));
    }
    let budget = budget_ms.map(Duration::from_millis);
    let start = td.inner.clone();
    let graph = &graph.inner;
    py.detach(|| goatd::decomposition::refine_with_flowcutter(start, graph, budget))
        .map(|inner| TreeDecomposition { inner })
        .map_err(py_error)
}

// `pymodule` expands this function into a Rust module of the same name, so it
// cannot be called `goatd` without shadowing the crate being wrapped. `name`
// is what Python imports and what the initialisation symbol is built from.
/// Tree decompositions of graphs, with PACE .gr and .td text in and out.
#[pymodule]
#[pyo3(name = "goatd")]
fn python_module(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("Error", module.py().get_type::<Error>())?;
    module.add_class::<Graph>()?;
    module.add_class::<TreeDecomposition>()?;
    module.add_function(wrap_pyfunction!(decompose, module)?)?;
    module.add_function(wrap_pyfunction!(refine_with_flowcutter, module)?)?;
    // The `__init__.py` maturin generates around the extension re-exports
    // exactly this list, `__version__` included.
    module.add(
        "__all__",
        vec![
            "Error",
            "Graph",
            "TreeDecomposition",
            "__version__",
            "decompose",
            "refine_with_flowcutter",
        ],
    )
}
