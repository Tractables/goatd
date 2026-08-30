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
use std::time::{Duration, Instant};

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

/// The limits the goatd command line gives FlowCutter. They are repeated here
/// because the library takes them as arguments and the command line does not
/// export its own.
const FLOWCUTTER_PATIENCE: Duration = Duration::from_millis(150);
const FLOWCUTTER_TIMED_ITERATIONS: u32 = 100_000;
const FLOWCUTTER_STEP_ITERATIONS: u32 = 900;

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
    /// FlowCutter's run time, and the refinement's deadline.
    pub budget_ms: u64,
    /// `GOATD_ORDER_FLOWCUTTER` only: a step budget in place of a clock, for a
    /// run that repeats exactly. 0 leaves it unset. Give either this or
    /// `budget_ms`, not both.
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
    /// it. Accepted with every order.
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
    if options.order == GOATD_ORDER_FLOWCUTTER && options.steps == 0 && options.budget_ms == 0 {
        return Err(invalid(
            "GOATD_ORDER_FLOWCUTTER needs a budget_ms or a steps limit",
        ));
    }
    Ok(())
}

fn construct(
    graph: &Graph,
    options: &GoatdOptions,
    weights: Option<&[u32]>,
) -> Result<TreeDecomposition, Error> {
    let start = Instant::now();
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
            let limit = match budget {
                Some(budget) => Budget::timed(
                    budget,
                    Some(FLOWCUTTER_PATIENCE),
                    FLOWCUTTER_TIMED_ITERATIONS,
                ),
                None => Budget::steps(options.steps, FLOWCUTTER_STEP_ITERATIONS),
            };
            flowcutter(graph, limit)?
        }
        GOATD_ORDER_PORTFOLIO => {
            let weights = vec![1; graph.num_vertices() as usize];
            let config = budget.map_or_else(PortfolioConfig::standard, |budget| {
                PortfolioConfig::standard().with_soft_budget(budget)
            });
            portfolio(graph, &weights, options.seed, config)?
        }
        unknown => return Err(unknown_order(unknown)),
    };
    if !options.refine {
        return Ok(td);
    }
    let remaining = budget.map(|budget| budget.saturating_sub(start.elapsed()));
    refine_with_flowcutter(td, graph, remaining)
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
