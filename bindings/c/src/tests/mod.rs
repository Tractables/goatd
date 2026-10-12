use super::*;

/// A path on four vertices, small enough to decompose in a test.
fn graph() -> Graph {
    Graph::try_new(4, [(0, 1), (1, 2), (2, 3)]).expect("a path is a graph")
}

/// The same path as an edge array, for the calls that go through
/// `goatd_decompose`.
const PATH_EDGES: [u32; 6] = [0, 1, 1, 2, 2, 3];

fn options(order: u32) -> GoatdOptions {
    GoatdOptions {
        order,
        ..goatd_options_default()
    }
}

/// `check_options` refused these options, with a message naming `what`.
fn refused(options: &GoatdOptions, what: &str) {
    let error = check_options(options).expect_err("these options are inert");
    let message = error.to_string();
    assert!(
        matches!(error, Error::InvalidInput(_)),
        "not an input error: {message}"
    );
    assert!(message.contains(what), "{what} is not named in: {message}");
}

#[test]
fn the_defaults_are_accepted() {
    check_options(&goatd_options_default()).expect("the defaults are min-fill");
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
    // The count is the sampled elimination's own check, so the refusal
    // comes back from the construction rather than from `check_options`.
    let weights = [1u32; 3];
    let mut short = options(GOATD_ORDER_MIN_FILL);
    short.sample_ties = true;
    short.tie_weights = weights.as_ptr();
    short.tie_weights_len = weights.len();
    let edges = PATH_EDGES;
    let mut td = GoatdDecomposition::empty();
    let status = unsafe { goatd_decompose(4, edges.as_ptr(), 3, &raw const short, &raw mut td) };
    assert_eq!(status, GOATD_ERROR_INVALID_INPUT);
    assert!(
        last_error().contains("3 weights for 4 vertices"),
        "{}",
        last_error()
    );
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
    check_options(&options(GOATD_ORDER_FLOWCUTTER)).expect("FlowCutter has a default of its own");
}

#[test]
fn refinement_runs_after_every_order_but_the_budgeted_portfolio() {
    for order in [
        GOATD_ORDER_MIN_FILL,
        GOATD_ORDER_MIN_DEGREE,
        GOATD_ORDER_NESTED_DISSECTION,
        GOATD_ORDER_FLOWCUTTER,
        GOATD_ORDER_PORTFOLIO,
    ] {
        let mut refined = options(order);
        refined.refine = true;
        check_options(&refined).expect("an unbudgeted construction leaves cuts to find");

        refined.budget_ms = 200;
        if order == GOATD_ORDER_PORTFOLIO {
            refused(&refined, "refine");
        } else {
            check_options(&refined).expect("refinement runs after a budgeted construction");
        }
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
    let status = unsafe { goatd_decompose(4, std::ptr::null(), 0, std::ptr::null(), &raw mut td) };
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
    let edges = PATH_EDGES;
    let mut td = GoatdDecomposition::empty();
    let defaults = goatd_options_default();
    let status = unsafe { goatd_decompose(4, edges.as_ptr(), 3, &raw const defaults, &raw mut td) };
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
    let edges = PATH_EDGES;
    let mut td = GoatdDecomposition::empty();
    let defaults = goatd_options_default();
    let status = unsafe { goatd_decompose(4, edges.as_ptr(), 3, &raw const defaults, &raw mut td) };
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
    let pairs = tree_edges.as_chunks::<2>().0;
    // The header promises the smaller bag first and the pairs in
    // ascending order, which a consumer may index on.
    assert!(pairs.iter().all(|pair| pair[0] < pair[1]));
    assert!(pairs.windows(2).all(|two| two[0] < two[1]));

    let shared = pairs
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
