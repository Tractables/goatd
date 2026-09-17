//! The C API for goatd: a graph goes in, a tree decomposition comes out.
//!
//! `include/goatd.h` is generated from this file with cbindgen, so the
//! documentation comments below are what a C caller reads. Building and
//! linking are covered in `bindings/c/README.md`.
//!
//! Every entry point catches a panic and reports it as
//! `GOATD_ERROR_PANIC` rather than unwinding into the caller.

use std::any::Any;
use std::cell::RefCell;
use std::ffi::{CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::time::Duration;

use goatd::decomposition::refine_with_flowcutter;
use goatd::elimination::{Order, decompose as eliminate};
use goatd::flowcutter::{Budget, decompose as flowcutter};
use goatd::portfolio::{PortfolioConfig, decompose as portfolio};
use goatd::{Error, Graph, TreeDecomposition};

/// What a goatd call returned: `GOATD_OK`, or one of the `GOATD_ERROR_`
/// values. `goatd_last_error_message` describes the failure in words.
pub type GoatdStatus = i32;

/// The call succeeded.
pub const GOATD_OK: i32 = 0;
/// The arguments broke a documented contract: a null pointer, a vertex id
/// outside the graph, or an option the chosen order cannot act on.
pub const GOATD_ERROR_INVALID_INPUT: i32 = 1;
/// The decomposition handed to `goatd_validate` is not a tree decomposition
/// of the given graph.
pub const GOATD_ERROR_INVALID_DECOMPOSITION: i32 = 2;
/// The graph exceeds a limit of the chosen construction.
pub const GOATD_ERROR_TOO_LARGE: i32 = 3;
/// The FlowCutter backend returned nothing.
pub const GOATD_ERROR_NO_DECOMPOSITION: i32 = 4;
/// goatd panicked. The panic did not cross into the caller, but the library
/// state behind it is no longer trustworthy; report it as a bug.
pub const GOATD_ERROR_PANIC: i32 = 5;
/// An error this version of the bindings has no code for. The message says
/// what happened.
pub const GOATD_ERROR_OTHER: i32 = 6;

/// Greedy min-fill elimination.
pub const GOATD_ORDER_MIN_FILL: u32 = 0;
/// Greedy min-degree elimination.
pub const GOATD_ORDER_MIN_DEGREE: u32 = 1;
/// Multilevel nested dissection.
pub const GOATD_ORDER_NESTED_DISSECTION: u32 = 2;
/// The vendored FlowCutter solver.
pub const GOATD_ORDER_FLOWCUTTER: u32 = 3;
/// Several orders under one budget, keeping the narrowest result. This is the
/// strongest setting; give it a `budget_ms`.
pub const GOATD_ORDER_PORTFOLIO: u32 = 4;

/// How a decomposition is constructed. Start from `goatd_options_default` and
/// change what you need: a field the chosen order cannot act on is an error,
/// not a silently ignored value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GoatdOptions {
    /// One of the `GOATD_ORDER_` values.
    pub order: u32,
    /// Tie-breaking seed. One seed gives one decomposition. Not accepted by
    /// `GOATD_ORDER_FLOWCUTTER`, which does not break ties this way.
    pub seed: u64,
    /// Milliseconds the construction may spend, or 0 for no limit. It is the
    /// soft deadline of the elimination orders and of the portfolio,
    /// FlowCutter's run time, and the refinement's deadline. The elimination
    /// orders and the portfolio stop for good at twice their soft deadline,
    /// and the refinement's deadline is its own, so a call can take about
    /// `2 * budget_ms`, or `3 * budget_ms` with `refine`.
    pub budget_ms: u64,
    /// `GOATD_ORDER_FLOWCUTTER` only: a step budget in place of a clock, for a
    /// run that repeats exactly. 0 leaves it unset. Give either this or
    /// `budget_ms`, not both; with neither, FlowCutter runs for its own
    /// default of 200 ms.
    pub steps: u64,
    /// `GOATD_ORDER_MIN_FILL` and `GOATD_ORDER_MIN_DEGREE` only: break ties by
    /// weighted sampling from the whole tie set instead of by salt.
    pub sample_ties: bool,
    /// With `sample_ties`, one weight per vertex; a smaller weight is
    /// eliminated earlier. Null weighs every vertex the same.
    pub tie_weights: *const u32,
    /// Number of entries in `tie_weights`, which must be the graph's vertex
    /// count.
    pub tie_weights_len: usize,
    /// Re-cut the decomposition along FlowCutter separators before returning
    /// it. Accepted with every order. With `budget_ms` set the pass is bounded
    /// and skips subgraphs over 100 000 vertices; with 0 it runs to completion,
    /// ungated, and one uninterruptible separator search on a large graph can
    /// take minutes.
    pub refine: bool,
}

/// A tree decomposition, flattened into arrays.
///
/// Bag `i` holds the vertices `bag_vertices[bag_offsets[i]]` up to but not
/// including `bag_vertices[bag_offsets[i + 1]]`, so `bag_offsets` has
/// `num_bags + 1` entries and its last entry is the length of `bag_vertices`.
/// `tree_edges` holds `2 * num_tree_edges` bag indices, one undirected edge
/// per pair.
///
/// `goatd_decompose` fills the struct the caller supplies and takes ownership
/// of nothing; the three arrays inside belong to the caller and are released
/// together by `goatd_decomposition_free`.
///
/// `treewidth`, `max_separator` and `bag_mass` describe the decomposition that
/// was built. They are written, never read: a struct the caller filled in for
/// `goatd_validate` is checked on its arrays alone.
#[repr(C)]
pub struct GoatdDecomposition {
    /// Vertices in the graph this decomposition was built for.
    pub num_vertices: u32,
    /// Number of bags.
    pub num_bags: usize,
    /// `num_bags + 1` offsets into `bag_vertices`.
    pub bag_offsets: *const usize,
    /// Bag contents, concatenated in bag order.
    pub bag_vertices: *const u32,
    /// Number of edges between bags.
    pub num_tree_edges: usize,
    /// `2 * num_tree_edges` bag indices.
    pub tree_edges: *const usize,
    /// Vertices in the largest bag, less one. An upper bound on the graph's
    /// treewidth.
    pub treewidth: u32,
    /// The most vertices two adjacent bags share, which is what a consumer
    /// joining two bags carries between them. No bags is `0`.
    pub max_separator: u32,
    /// `log2` of the sum over bags of `2^(bag size)`: what a consumer
    /// compiling over the bags pays in the worst case, on the same scale as
    /// the width. No bags is `0`.
    ///
    /// The sum is scaled by the largest bag before the logarithm, so a bag of
    /// a few thousand vertices still gives an answer where `2^(bag size)` on
    /// its own is already infinite. The plain sum of the bag sizes is
    /// `bag_offsets[num_bags]` and is not repeated here.
    pub bag_mass: f64,
}

impl GoatdDecomposition {
    const fn empty() -> Self {
        Self {
            num_vertices: 0,
            num_bags: 0,
            bag_offsets: std::ptr::null(),
            bag_vertices: std::ptr::null(),
            num_tree_edges: 0,
            tree_edges: std::ptr::null(),
            treewidth: 0,
            max_separator: 0,
            bag_mass: 0.0,
        }
    }
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

/// The version of these bindings, which is the version of the goatd release
/// they wrap. The string is static and outlives every other call.
#[unsafe(no_mangle)]
pub extern "C" fn goatd_version() -> *const c_char {
    const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr().cast()
}

/// Why the last call on this thread failed, as a NUL-terminated string, or
/// the empty string if it succeeded. Never null.
///
/// The message belongs to goatd and is replaced by the next call on the same
/// thread; copy it if you need to keep it. Errors are recorded per thread, so
/// a message never crosses from one thread to another.
#[unsafe(no_mangle)]
pub extern "C" fn goatd_last_error_message() -> *const c_char {
    const EMPTY: &str = "\0";
    LAST_ERROR
        .try_with(|slot| slot.borrow().as_ptr())
        .unwrap_or_else(|_| EMPTY.as_ptr().cast())
}

/// The defaults: min-fill, seed 0, no budget, no sampling, no refinement.
#[unsafe(no_mangle)]
pub extern "C" fn goatd_options_default() -> GoatdOptions {
    GoatdOptions {
        order: GOATD_ORDER_MIN_FILL,
        seed: 0,
        budget_ms: 0,
        steps: 0,
        sample_ties: false,
        tie_weights: std::ptr::null(),
        tie_weights_len: 0,
        refine: false,
    }
}

/// Decompose the graph on vertices `0..num_vertices` whose `num_edges`
/// undirected edges are the pairs in `edges`.
///
/// On `GOATD_OK`, `*out` describes the decomposition and the caller releases
/// it with `goatd_decomposition_free`; on any other status `*out` is
/// untouched. `*out` is overwritten rather than merged, so free an earlier
/// result before reusing the storage.
///
/// # Safety
///
/// `edges` must point to `2 * num_edges` vertex ids, or be null when
/// `num_edges` is zero; `options` and `out` must each point to storage for one
/// value of their type; and `options->tie_weights`, when it is not null, must
/// point to `options->tie_weights_len` weights.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn goatd_decompose(
    num_vertices: u32,
    edges: *const u32,
    num_edges: usize,
    options: *const GoatdOptions,
    out: *mut GoatdDecomposition,
) -> GoatdStatus {
    guard(|| {
        if options.is_null() {
            return Err(invalid("options must not be null"));
        }
        if out.is_null() {
            return Err(invalid("out must not be null"));
        }
        let options = unsafe { &*options };
        let graph = unsafe { graph_from_edges(num_vertices, edges, num_edges) }?;
        check_options(options, &graph)?;
        let weights = unsafe { tie_weights(options) };
        let td = construct(&graph, options, weights)?;
        unsafe { out.write(flatten(&td)) };
        Ok(())
    })
}

/// Release the arrays in a decomposition `goatd_decompose` produced and leave
/// the struct empty, so calling this twice is harmless. The struct itself
/// belongs to the caller.
///
/// # Safety
///
/// `decomposition` must be null or point to a value `goatd_decompose` filled
/// in and nothing has freed since.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn goatd_decomposition_free(decomposition: *mut GoatdDecomposition) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if decomposition.is_null() {
            return;
        }
        let td = unsafe { &mut *decomposition };
        // The vertex array's length is the last offset, so read it first.
        let total = if td.bag_offsets.is_null() {
            0
        } else {
            unsafe { *td.bag_offsets.add(td.num_bags) }
        };
        unsafe {
            reclaim(td.bag_offsets, td.num_bags + 1);
            reclaim(td.bag_vertices, total);
            reclaim(td.tree_edges, td.num_tree_edges * 2);
        }
        *td = GoatdDecomposition::empty();
    }));
}

/// Check a decomposition against its graph with goatd's own validator: bag
/// contents, an acyclic bag tree, vertex and edge coverage, and the running
/// intersection property.
///
/// Returns `GOATD_OK` when it holds and `GOATD_ERROR_INVALID_DECOMPOSITION`
/// with a message naming the first violation when it does not. The
/// decomposition need not have come from `goatd_decompose`.
///
/// # Safety
///
/// `edges` must point to `2 * num_edges` vertex ids, or be null when
/// `num_edges` is zero, and `decomposition` must point to one
/// `GoatdDecomposition` whose arrays have the lengths its fields describe.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn goatd_validate(
    num_vertices: u32,
    edges: *const u32,
    num_edges: usize,
    decomposition: *const GoatdDecomposition,
) -> GoatdStatus {
    guard(|| {
        if decomposition.is_null() {
            return Err(invalid("decomposition must not be null"));
        }
        let td = unsafe { &*decomposition };
        let graph = unsafe { graph_from_edges(num_vertices, edges, num_edges) }?;
        let (bags, tree_edges) = unsafe { unflatten(td) }?;
        TreeDecomposition::new(&graph, bags, tree_edges).map(|_| ())
    })
}

/// Run one entry point's body, turning an error or a panic into a status and a
/// message the caller can fetch.
fn guard(body: impl FnOnce() -> Result<(), Error>) -> GoatdStatus {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(())) => {
            set_error("");
            GOATD_OK
        }
        Ok(Err(error)) => {
            set_error(&error.to_string());
            match error {
                Error::InvalidInput(_) => GOATD_ERROR_INVALID_INPUT,
                Error::InvalidDecomposition(_) => GOATD_ERROR_INVALID_DECOMPOSITION,
                Error::TooLarge(_) => GOATD_ERROR_TOO_LARGE,
                Error::NoDecomposition => GOATD_ERROR_NO_DECOMPOSITION,
                _ => GOATD_ERROR_OTHER,
            }
        }
        Err(payload) => {
            set_error(&panic_message(&*payload));
            GOATD_ERROR_PANIC
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    let what = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    match what {
        Some(what) => format!("goatd panicked: {what}"),
        None => "goatd panicked".to_string(),
    }
}

fn set_error(message: &str) {
    // An embedded NUL would truncate the sentence a C caller reads.
    let message = message.replace('\0', " ");
    let _ = LAST_ERROR.try_with(|slot| {
        *slot.borrow_mut() = CString::new(message).unwrap_or_default();
    });
}

fn invalid(what: &str) -> Error {
    Error::InvalidInput(what.to_string())
}

fn unknown_order(order: u32) -> Error {
    Error::InvalidInput(format!(
        "order {order} is not one of the GOATD_ORDER_ values"
    ))
}

/// # Safety
///
/// As `goatd_decompose`, for `edges` and `num_edges`.
unsafe fn graph_from_edges(
    num_vertices: u32,
    edges: *const u32,
    num_edges: usize,
) -> Result<Graph, Error> {
    if num_edges == 0 {
        return Graph::try_new(num_vertices, []);
    }
    if edges.is_null() {
        return Err(invalid("edges is null but num_edges is not zero"));
    }
    let Some(len) = num_edges.checked_mul(2) else {
        return Err(invalid(
            "num_edges is too large to address as endpoint pairs",
        ));
    };
    let flat = unsafe { slice::from_raw_parts(edges, len) };
    let (pairs, _) = flat.as_chunks::<2>();
    Graph::try_new(num_vertices, pairs.iter().map(|pair| (pair[0], pair[1])))
}

/// # Safety
///
/// As `goatd_decompose`, for `options->tie_weights`.
unsafe fn tie_weights(options: &GoatdOptions) -> Option<&[u32]> {
    if options.tie_weights.is_null() {
        return None;
    }
    Some(unsafe { slice::from_raw_parts(options.tie_weights, options.tie_weights_len) })
}

/// Reject the option combinations that cannot mean anything for the chosen
/// order, naming the field and the orders that accept it.
fn check_options(options: &GoatdOptions, graph: &Graph) -> Result<(), Error> {
    let order = match options.order {
        GOATD_ORDER_MIN_FILL => "GOATD_ORDER_MIN_FILL",
        GOATD_ORDER_MIN_DEGREE => "GOATD_ORDER_MIN_DEGREE",
        GOATD_ORDER_NESTED_DISSECTION => "GOATD_ORDER_NESTED_DISSECTION",
        GOATD_ORDER_FLOWCUTTER => "GOATD_ORDER_FLOWCUTTER",
        GOATD_ORDER_PORTFOLIO => "GOATD_ORDER_PORTFOLIO",
        unknown => return Err(unknown_order(unknown)),
    };
    let inert = |field: &str, accepts: &str| -> Result<(), Error> {
        Err(Error::InvalidInput(format!(
            "{field} is not valid with {order}; it applies to {accepts}"
        )))
    };
    let greedy = options.order == GOATD_ORDER_MIN_FILL || options.order == GOATD_ORDER_MIN_DEGREE;

    if options.sample_ties && !greedy {
        return inert(
            "sample_ties",
            "GOATD_ORDER_MIN_FILL and GOATD_ORDER_MIN_DEGREE",
        );
    }
    if !options.tie_weights.is_null() {
        if !greedy {
            return inert(
                "tie_weights",
                "GOATD_ORDER_MIN_FILL and GOATD_ORDER_MIN_DEGREE",
            );
        }
        if !options.sample_ties {
            return Err(invalid("tie_weights is only read with sample_ties"));
        }
        // The sampled orders check the count against the graph themselves;
        // this only rules out reading past the array the caller described.
        if options.tie_weights_len != graph.num_vertices() as usize {
            return Err(Error::InvalidInput(format!(
                "tie_weights_len is {} for a graph of {} vertices",
                options.tie_weights_len,
                graph.num_vertices()
            )));
        }
    }
    if options.seed != 0 && options.order == GOATD_ORDER_FLOWCUTTER {
        return inert(
            "seed",
            "GOATD_ORDER_MIN_FILL, GOATD_ORDER_MIN_DEGREE, \
             GOATD_ORDER_NESTED_DISSECTION and GOATD_ORDER_PORTFOLIO",
        );
    }
    if options.steps != 0 {
        if options.order != GOATD_ORDER_FLOWCUTTER {
            return inert("steps", "GOATD_ORDER_FLOWCUTTER");
        }
        if options.budget_ms != 0 {
            return Err(invalid(
                "steps and budget_ms both bound FlowCutter; give one",
            ));
        }
    }
    Ok(())
}

fn construct(
    graph: &Graph,
    options: &GoatdOptions,
    weights: Option<&[u32]>,
) -> Result<TreeDecomposition, Error> {
    let budget = (options.budget_ms != 0).then(|| Duration::from_millis(options.budget_ms));
    let td = match options.order {
        GOATD_ORDER_MIN_FILL | GOATD_ORDER_MIN_DEGREE => {
            // Sampling without caller weights gives every vertex the same one.
            let uniform = match (options.sample_ties, weights) {
                (true, None) => vec![1; graph.num_vertices() as usize],
                _ => Vec::new(),
            };
            let sampled = options
                .sample_ties
                .then(|| weights.unwrap_or(uniform.as_slice()));
            let order = match (options.order, sampled) {
                (GOATD_ORDER_MIN_FILL, None) => Order::MinFill,
                (GOATD_ORDER_MIN_FILL, Some(weights)) => Order::MinFillSampled { weights },
                (_, None) => Order::MinDegree,
                (_, Some(weights)) => Order::MinDegreeSampled { weights },
            };
            eliminate(graph, order, options.seed, budget)?
        }
        GOATD_ORDER_NESTED_DISSECTION => {
            eliminate(graph, Order::NestedDissection, options.seed, budget)?
        }
        GOATD_ORDER_FLOWCUTTER => {
            // Validation above has left exactly one of the two set.
            let steps = (options.steps != 0).then_some(options.steps);
            flowcutter(graph, Budget::standalone(budget, steps))?
        }
        GOATD_ORDER_PORTFOLIO => {
            let weights = vec![1; graph.num_vertices() as usize];
            let config = budget.map_or_else(
                PortfolioConfig::standard,
                PortfolioConfig::standard_with_budget,
            );
            portfolio(graph, &weights, options.seed, config)?
        }
        unknown => return Err(unknown_order(unknown)),
    };
    if !options.refine {
        return Ok(td);
    }
    // The budget is a deadline per phase, as the header says it is for every
    // phase it lists. Giving the pass what the construction left of one shared
    // budget made it a no-op on every graph the construction did not finish
    // early, which is the graph it is wanted on.
    refine_with_flowcutter(td, graph, budget)
}

fn flatten(td: &TreeDecomposition) -> GoatdDecomposition {
    let bags = td.bags();
    let mut offsets = Vec::with_capacity(bags.len() + 1);
    let mut vertices = Vec::with_capacity(td.total_bag_size());
    offsets.push(0);
    for bag in bags {
        vertices.extend_from_slice(bag.vertices());
        offsets.push(vertices.len());
    }
    let mut tree_edges = Vec::new();
    for (bag, neighbours) in td.adjacency().iter().enumerate() {
        for &neighbour in neighbours {
            if bag < neighbour {
                tree_edges.push(bag);
                tree_edges.push(neighbour);
            }
        }
    }
    GoatdDecomposition {
        num_vertices: td.num_vertices(),
        num_bags: bags.len(),
        bag_offsets: release(offsets),
        bag_vertices: release(vertices),
        num_tree_edges: tree_edges.len() / 2,
        tree_edges: release(tree_edges),
        treewidth: td.treewidth(),
        // A separator is a set of the graph's vertices, so it counts in the
        // same type the vertices do.
        max_separator: td.max_separator() as u32,
        bag_mass: td.bag_mass(),
    }
}

/// Read a flattened decomposition back into the shapes
/// `TreeDecomposition::new` takes. The offsets are checked here because a
/// caller may have built the arrays itself.
///
/// # Safety
///
/// As `goatd_validate`, for the arrays `td` describes.
#[allow(clippy::type_complexity)]
unsafe fn unflatten(
    td: &GoatdDecomposition,
) -> Result<(Vec<Vec<u32>>, Vec<(usize, usize)>), Error> {
    if td.num_bags != 0 && td.bag_offsets.is_null() {
        return Err(invalid("bag_offsets is null but num_bags is not zero"));
    }
    let offsets = if td.bag_offsets.is_null() {
        &[][..]
    } else {
        unsafe { slice::from_raw_parts(td.bag_offsets, td.num_bags + 1) }
    };
    let total = match offsets.first().copied() {
        None => 0,
        Some(0) => offsets[td.num_bags],
        Some(_) => return Err(invalid("bag_offsets does not start at 0")),
    };
    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(invalid("bag_offsets does not increase"));
    }
    if total != 0 && td.bag_vertices.is_null() {
        return Err(invalid("bag_vertices is null but the bags are not empty"));
    }
    let vertices = if td.bag_vertices.is_null() {
        &[][..]
    } else {
        unsafe { slice::from_raw_parts(td.bag_vertices, total) }
    };
    let bags = (0..td.num_bags)
        .map(|bag| vertices[offsets[bag]..offsets[bag + 1]].to_vec())
        .collect();

    if td.num_tree_edges != 0 && td.tree_edges.is_null() {
        return Err(invalid("tree_edges is null but num_tree_edges is not zero"));
    }
    let Some(len) = td.num_tree_edges.checked_mul(2) else {
        return Err(invalid("num_tree_edges is too large to address as pairs"));
    };
    let flat = if td.tree_edges.is_null() {
        &[][..]
    } else {
        unsafe { slice::from_raw_parts(td.tree_edges, len) }
    };
    let (pairs, _) = flat.as_chunks::<2>();
    let tree_edges = pairs.iter().map(|pair| (pair[0], pair[1])).collect();
    Ok((bags, tree_edges))
}

/// Hand an array to the caller. `goatd_decomposition_free` takes it back.
fn release<T>(values: Vec<T>) -> *const T {
    Box::into_raw(values.into_boxed_slice()) as *const T
}

/// # Safety
///
/// `ptr` must be null, or an array of `len` values from [`release`] that
/// nothing has freed since.
unsafe fn reclaim<T>(ptr: *const T, len: usize) {
    if !ptr.is_null() {
        drop(unsafe { Vec::from_raw_parts(ptr.cast_mut(), len, len) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path on four vertices: enough graph for the option checks, which
    /// only read its vertex count, and small enough to decompose in a test.
    fn graph() -> Graph {
        Graph::try_new(4, [(0, 1), (1, 2), (2, 3)]).expect("a path is a graph")
    }

    fn options(order: u32) -> GoatdOptions {
        GoatdOptions {
            order,
            ..goatd_options_default()
        }
    }

    /// `check_options` refused these options, with a message naming `what`.
    fn refused(options: &GoatdOptions, what: &str) {
        let error = check_options(options, &graph()).expect_err("these options are inert");
        let message = error.to_string();
        assert!(
            matches!(error, Error::InvalidInput(_)),
            "not an input error: {message}"
        );
        assert!(message.contains(what), "{what} is not named in: {message}");
    }

    #[test]
    fn the_defaults_are_accepted() {
        check_options(&goatd_options_default(), &graph()).expect("the defaults are min-fill");
    }

    #[test]
    fn an_order_outside_the_list_is_refused_by_number() {
        refused(&options(99), "99");
    }

    #[test]
    fn sampling_ties_is_for_the_two_greedy_orders() {
        for order in [
            GOATD_ORDER_NESTED_DISSECTION,
            GOATD_ORDER_FLOWCUTTER,
            GOATD_ORDER_PORTFOLIO,
        ] {
            let mut inert = options(order);
            inert.sample_ties = true;
            refused(&inert, "sample_ties");
        }
    }

    #[test]
    fn tie_weights_are_only_read_with_sampling() {
        let weights = [1u32; 4];
        let mut inert = options(GOATD_ORDER_MIN_FILL);
        inert.tie_weights = weights.as_ptr();
        inert.tie_weights_len = weights.len();
        refused(&inert, "sample_ties");
    }

    #[test]
    fn tie_weights_are_counted_against_the_graph() {
        let weights = [1u32; 3];
        let mut short = options(GOATD_ORDER_MIN_FILL);
        short.sample_ties = true;
        short.tie_weights = weights.as_ptr();
        short.tie_weights_len = weights.len();
        refused(&short, "tie_weights_len is 3 for a graph of 4 vertices");
    }

    #[test]
    fn flowcutter_does_not_break_ties_by_seed() {
        let mut inert = options(GOATD_ORDER_FLOWCUTTER);
        inert.seed = 1;
        inert.steps = 100;
        refused(&inert, "seed");
    }

    #[test]
    fn a_step_budget_is_flowcutters_alone() {
        for order in [
            GOATD_ORDER_MIN_FILL,
            GOATD_ORDER_MIN_DEGREE,
            GOATD_ORDER_NESTED_DISSECTION,
            GOATD_ORDER_PORTFOLIO,
        ] {
            let mut inert = options(order);
            inert.steps = 100;
            refused(&inert, "steps");
        }
    }

    #[test]
    fn flowcutter_takes_one_bound_or_neither() {
        let mut both = options(GOATD_ORDER_FLOWCUTTER);
        both.steps = 100;
        both.budget_ms = 100;
        refused(&both, "give one");
        // With neither, FlowCutter runs on its own default, as it does from
        // the command line and from the other two bindings.
        check_options(&options(GOATD_ORDER_FLOWCUTTER), &graph())
            .expect("FlowCutter has a default of its own");
    }

    #[test]
    fn every_order_accepts_refinement() {
        for order in [
            GOATD_ORDER_MIN_FILL,
            GOATD_ORDER_MIN_DEGREE,
            GOATD_ORDER_NESTED_DISSECTION,
            GOATD_ORDER_FLOWCUTTER,
            GOATD_ORDER_PORTFOLIO,
        ] {
            let mut refined = options(order);
            refined.refine = true;
            check_options(&refined, &graph()).expect("refinement runs after every order");
        }
    }

    /// A decomposition the caller built itself, pointing at the arrays given,
    /// which have to outlive it.
    fn described(offsets: &[usize], vertices: &[u32], tree_edges: &[usize]) -> GoatdDecomposition {
        GoatdDecomposition {
            num_vertices: 4,
            num_bags: offsets.len().saturating_sub(1),
            bag_offsets: offsets.as_ptr(),
            bag_vertices: vertices.as_ptr(),
            num_tree_edges: tree_edges.len() / 2,
            tree_edges: tree_edges.as_ptr(),
            treewidth: 0,
            max_separator: 0,
            bag_mass: 0.0,
        }
    }

    /// `unflatten` refused this description, with a message naming `what`.
    fn malformed(td: &GoatdDecomposition, what: &str) {
        let error = unsafe { unflatten(td) }.expect_err("this description is malformed");
        let message = error.to_string();
        assert!(
            matches!(error, Error::InvalidInput(_)),
            "not an input error: {message}"
        );
        assert!(message.contains(what), "{what} is not named in: {message}");
    }

    #[test]
    fn the_offsets_start_at_zero_and_increase() {
        let vertices = [0u32, 1];
        malformed(&described(&[1, 2], &vertices, &[]), "does not start at 0");
        malformed(&described(&[0, 2, 1], &vertices, &[]), "does not increase");
    }

    #[test]
    fn an_array_the_counts_ask_for_may_not_be_null() {
        let (offsets, vertices, tree_edges) = ([0usize, 1, 2], [0u32, 1], [0usize, 1]);

        let mut no_offsets = described(&offsets, &vertices, &tree_edges);
        no_offsets.bag_offsets = std::ptr::null();
        malformed(&no_offsets, "bag_offsets is null");

        let mut no_vertices = described(&offsets, &vertices, &tree_edges);
        no_vertices.bag_vertices = std::ptr::null();
        malformed(&no_vertices, "bag_vertices is null");

        let mut no_edges = described(&offsets, &vertices, &tree_edges);
        no_edges.tree_edges = std::ptr::null();
        malformed(&no_edges, "tree_edges is null");
    }

    #[test]
    fn a_decomposition_with_no_bags_reads_back_empty() {
        let empty = GoatdDecomposition::empty();
        let (bags, tree_edges) = unsafe { unflatten(&empty) }.expect("no bags is a description");
        assert!(bags.is_empty());
        assert!(tree_edges.is_empty());
    }

    #[test]
    fn flattening_and_reading_back_gives_the_same_decomposition() {
        let graph = graph();
        let td = eliminate(&graph, Order::MinFill, 0, None).expect("a path decomposes");
        let mut flat = flatten(&td);
        let (bags, tree_edges) = unsafe { unflatten(&flat) }.expect("goatd's own arrays");
        assert_eq!(bags.len(), td.bags().len());
        for (read, bag) in bags.iter().zip(td.bags()) {
            assert_eq!(read.as_slice(), bag.vertices());
        }
        TreeDecomposition::new(&graph, bags, tree_edges).expect("and it is still a decomposition");
        unsafe { goatd_decomposition_free(&raw mut flat) };
        assert_eq!(flat.num_bags, 0);
        assert!(flat.bag_offsets.is_null());
    }

    #[test]
    fn the_entry_points_name_the_null_argument() {
        let mut td = GoatdDecomposition::empty();
        let status =
            unsafe { goatd_decompose(4, std::ptr::null(), 0, std::ptr::null(), &raw mut td) };
        assert_eq!(status, GOATD_ERROR_INVALID_INPUT);
        assert!(last_error().contains("options"));

        let defaults = goatd_options_default();
        let status = unsafe {
            goatd_decompose(
                4,
                std::ptr::null(),
                0,
                &raw const defaults,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(status, GOATD_ERROR_INVALID_INPUT);
        assert!(last_error().contains("out"));

        let status = unsafe { goatd_validate(4, std::ptr::null(), 0, std::ptr::null()) };
        assert_eq!(status, GOATD_ERROR_INVALID_INPUT);
        assert!(last_error().contains("decomposition"));
    }

    #[test]
    fn an_edge_array_the_count_asks_for_may_not_be_null() {
        let mut td = GoatdDecomposition::empty();
        let defaults = goatd_options_default();
        let status =
            unsafe { goatd_decompose(4, std::ptr::null(), 1, &raw const defaults, &raw mut td) };
        assert_eq!(status, GOATD_ERROR_INVALID_INPUT);
        assert!(last_error().contains("edges is null"));
    }

    #[test]
    fn a_run_that_succeeded_leaves_no_message_behind() {
        let edges = [0u32, 1, 1, 2, 2, 3];
        let mut td = GoatdDecomposition::empty();
        let defaults = goatd_options_default();
        let status =
            unsafe { goatd_decompose(4, edges.as_ptr(), 3, &raw const defaults, &raw mut td) };
        assert_eq!(status, GOATD_OK, "{}", last_error());
        assert_eq!(last_error(), "");
        assert_eq!(td.num_vertices, 4);
        // The bags of a tree decomposition are joined into a tree, and the
        // last offset is the length of the vertex array.
        assert_eq!(td.num_tree_edges, td.num_bags - 1);
        let offsets = unsafe { slice::from_raw_parts(td.bag_offsets, td.num_bags + 1) };
        assert_eq!(offsets.first(), Some(&0));
        assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            unsafe { goatd_validate(4, edges.as_ptr(), 3, &raw const td) },
            GOATD_OK
        );
        unsafe { goatd_decomposition_free(&raw mut td) };
        // Freeing leaves the struct empty, so a second call has nothing to do.
        unsafe { goatd_decomposition_free(&raw mut td) };
        unsafe { goatd_decomposition_free(std::ptr::null_mut()) };
    }

    #[test]
    fn the_shape_numbers_describe_the_arrays_beside_them() {
        // A path of four: whatever order wins, the bags are small enough to
        // re-derive all three numbers from the arrays directly, which is what
        // a consumer would otherwise have to write for itself.
        let edges = [0u32, 1, 1, 2, 2, 3];
        let mut td = GoatdDecomposition::empty();
        let defaults = goatd_options_default();
        let status =
            unsafe { goatd_decompose(4, edges.as_ptr(), 3, &raw const defaults, &raw mut td) };
        assert_eq!(status, GOATD_OK, "{}", last_error());
        let offsets = unsafe { slice::from_raw_parts(td.bag_offsets, td.num_bags + 1) };
        let vertices = unsafe { slice::from_raw_parts(td.bag_vertices, offsets[td.num_bags]) };
        let bag = |index: usize| &vertices[offsets[index]..offsets[index + 1]];

        let widest = (0..td.num_bags).map(|i| bag(i).len()).max().expect("bags");
        assert_eq!(td.treewidth, widest as u32 - 1);

        let direct: f64 = (0..td.num_bags).map(|i| (bag(i).len() as f64).exp2()).sum();
        assert!(
            (td.bag_mass - direct.log2()).abs() < 1e-9,
            "{} against {}",
            td.bag_mass,
            direct.log2()
        );

        let tree_edges = unsafe { slice::from_raw_parts(td.tree_edges, 2 * td.num_tree_edges) };
        let shared = tree_edges
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                bag(pair[0])
                    .iter()
                    .filter(|vertex| bag(pair[1]).contains(vertex))
                    .count()
            })
            .max()
            .unwrap_or(0);
        assert_eq!(td.max_separator, shared as u32);

        unsafe { goatd_decomposition_free(&raw mut td) };
    }

    #[test]
    fn the_version_is_the_package_version() {
        let version = unsafe { std::ffi::CStr::from_ptr(goatd_version()) };
        assert_eq!(version.to_str().expect("ascii"), env!("CARGO_PKG_VERSION"));
    }

    fn last_error() -> String {
        unsafe { std::ffi::CStr::from_ptr(goatd_last_error_message()) }
            .to_str()
            .expect("the message is made from a Rust string")
            .to_string()
    }
}
