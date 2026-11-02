//! The `goatd` binary, driven over PACE files: every order decomposes a
//! small graph into a valid `.td`, the output goes where it is asked to go,
//! and a flag the chosen order cannot act on is refused with a message that
//! names both.

use std::path::PathBuf;
use std::process::{Command, Output};

use goatd::{Graph, TreeDecomposition};

/// The budget the tests that read the whole schedule give it. They pair it
/// with `--capped-restarts`, so the run returns when the schedule is done
/// rather than when the budget is: on this graph the eight weighted stages and
/// their diverse passes cost about a third of a second in a debug build, and
/// the rest of the budget is margin for a machine that stalls under a parallel
/// build. Over a 4.75-second soft budget the trailing FlowCutter candidate
/// runs to the end of its window instead of stopping early, and the run then
/// spends the whole budget: 0.28 s at 4,000 against 5.3 s at 5,000.
const SCHEDULE_BUDGET: &str = "2000";

/// The 3×3 grid: nine vertices, twelve edges, treewidth 3.
fn grid_gr() -> String {
    let mut edges = Vec::new();
    for r in 0..3u32 {
        for c in 0..3u32 {
            let v = r * 3 + c;
            if c + 1 < 3 {
                edges.push((v, v + 1));
            }
            if r + 1 < 3 {
                edges.push((v, v + 3));
            }
        }
    }
    Graph::new(9, edges).to_gr()
}

/// A scratch directory of this test's own, under the target directory so a
/// parallel test run never sees another test's files.
fn scratch(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("cli_{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the scratch directory");
    dir
}

fn goatd(args: &[&str], stdin: Option<&str>) -> Output {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_goatd"))
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn goatd");
    if let Some(text) = stdin {
        let write_result = child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(text.as_bytes());
        if let Err(error) = write_result
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            panic!("write the graph to stdin: {error}");
        }
    }
    child.wait_with_output().expect("goatd runs to completion")
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn every_order_writes_a_valid_decomposition_of_the_grid() {
    let dir = scratch("every_order");
    let gr = dir.join("grid.gr");
    std::fs::write(&gr, grid_gr()).expect("write the graph");
    let graph = Graph::from_gr(&grid_gr()).expect("the grid parses");

    let orders: &[&[&str]] = &[
        &["--order", "minfill"],
        &["--order", "minfill", "--ties", "sample", "--seed", "7"],
        &["--order", "mindegree"],
        &["--order", "mindegree", "--ties", "sample"],
        &["--order", "nested-dissection"],
        &["--order", "flowcutter", "--steps", "2000"],
        &[
            "--order",
            "portfolio",
            "--budget",
            "500",
            "--hard-budget",
            "750",
        ],
        &["--order", "minfill", "--refine"],
    ];
    for (i, order) in orders.iter().enumerate() {
        let td_path = dir.join(format!("{i}.td"));
        let mut args: Vec<&str> = vec![gr.to_str().unwrap(), "--out", td_path.to_str().unwrap()];
        args.extend_from_slice(order);
        let out = goatd(&args, None);
        assert!(
            out.status.success(),
            "{order:?} failed: {}",
            stderr_of(&out)
        );
        assert!(
            out.stdout.is_empty(),
            "{order:?} wrote to stdout although --out was given"
        );
        let text = std::fs::read_to_string(&td_path).expect("the .td was written");
        let td = TreeDecomposition::from_td(&text).expect("goatd wrote a well-formed .td");
        td.validate(&graph)
            .unwrap_or_else(|error| panic!("{order:?}: {error}"));
        assert!(
            td.treewidth() <= 4,
            "{order:?}: width {} on a treewidth-3 grid",
            td.treewidth()
        );
    }
}

#[test]
fn stdin_in_and_stdout_out_by_default() {
    let out = goatd(&["-"], Some(&grid_gr()));
    assert!(out.status.success(), "{}", stderr_of(&out));
    let text = String::from_utf8(out.stdout).expect("utf-8");
    assert!(text.starts_with("s td "), "not a .td: {text}");
    let td = TreeDecomposition::from_td(&text).expect("a well-formed .td on stdout");
    td.validate(&Graph::from_gr(&grid_gr()).unwrap())
        .expect("the decomposition written to stdout is valid");
}

#[test]
fn disconnected_and_empty_graphs_are_written_as_one_pace_bag_tree() {
    for graph in [Graph::new(4, [(0, 1), (2, 3)]), Graph::new(0, [])] {
        let out = goatd(&["-"], Some(&graph.to_gr()));
        assert!(out.status.success(), "{}", stderr_of(&out));
        let text = String::from_utf8(out.stdout).expect("utf-8");
        let td = TreeDecomposition::from_td(&text).expect("one PACE bag tree");
        td.validate(&graph)
            .expect("the serialized decomposition remains valid");
    }
}

#[test]
fn a_seeded_sampling_run_repeats_itself() {
    let run = |seed: &str| {
        let args = [
            "-", "--order", "minfill", "--ties", "sample", "--seed", seed,
        ];
        let out = goatd(&args, Some(&grid_gr()));
        assert!(out.status.success(), "{}", stderr_of(&out));
        out.stdout
    };
    assert_eq!(run("11"), run("11"));
}

#[test]
fn a_weights_file_biases_the_sample_and_is_checked_against_the_graph() {
    let dir = scratch("weights");
    let weights = dir.join("w.txt");
    std::fs::write(&weights, "1\n2\n3\n4\n5\n6\n7\n8\n9\n").expect("write weights");
    let out = goatd(
        &[
            "-",
            "--order",
            "minfill",
            "--ties",
            "sample",
            "--weights",
            weights.to_str().unwrap(),
        ],
        Some(&grid_gr()),
    );
    assert!(out.status.success(), "{}", stderr_of(&out));

    std::fs::write(&weights, "1\n2\n").expect("write too few weights");
    let out = goatd(
        &[
            "-",
            "--order",
            "minfill",
            "--ties",
            "sample",
            "--weights",
            weights.to_str().unwrap(),
        ],
        Some(&grid_gr()),
    );
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_of(&out);
    assert!(
        err.contains("2 weights") && err.contains("9 vertices"),
        "the error must give both counts: {err}"
    );
}

fn assert_refused_with(flags: &[&str], expected: &[&str]) {
    let mut args: Vec<&str> = if flags.is_empty() { vec![] } else { vec!["-"] };
    args.extend_from_slice(flags);
    let out = goatd(&args, Some(&grid_gr()));
    assert_eq!(
        out.status.code(),
        Some(2),
        "{flags:?} must be a usage error; stderr: {}",
        stderr_of(&out)
    );
    let err = stderr_of(&out);
    for want in expected {
        assert!(
            err.contains(want),
            "{flags:?}: must name {want:?}, got: {err}"
        );
    }
}

/// An order-specific flag is refused under another order, and the message
/// names the flag and both orders.
#[test]
fn an_unsupported_flag_is_refused_naming_the_flag_and_the_order() {
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["--order", "minfill", "--steps", "10"],
            &["--steps", "minfill", "flowcutter"],
        ),
        (
            &["--order", "flowcutter", "--seed", "3"],
            &["--seed", "flowcutter"],
        ),
        (
            &["--order", "nested-dissection", "--ties", "sample"],
            &["--ties sample", "nested-dissection", "minfill or mindegree"],
        ),
        (
            &["--order", "portfolio", "--ties", "sample"],
            &["--ties sample", "portfolio"],
        ),
        (
            &["--order", "flowcutter", "--weights", "w.txt"],
            &["--weights", "flowcutter", "minfill or mindegree"],
        ),
        (
            &["--order", "portfolio", "--refine"],
            &["--refine", "portfolio"],
        ),
        (
            &["--order", "flowcutter", "--no-sampling-patience"],
            &["--no-sampling-patience", "flowcutter", "portfolio"],
        ),
        (
            &["--order", "flowcutter", "--expensive-orders-up-to", "50000"],
            &["--expensive-orders-up-to", "flowcutter", "portfolio"],
        ),
        (
            &["--order", "flowcutter", "--trace"],
            &["--trace", "flowcutter", "portfolio"],
        ),
    ];
    for &(flags, expected) in cases {
        assert_refused_with(flags, expected);
    }
    assert_portfolio_only_flags_refused_under_minfill();
}

fn assert_portfolio_only_flags_refused_under_minfill() {
    let cases: &[(&[&str], &[&str])] = &[
        (&["--hard-budget", "10"], &["--hard-budget", "minfill"]),
        (&["--no-hedge"], &["--no-hedge", "minfill"]),
        (
            &["--no-bipartite-lift"],
            &["--no-bipartite-lift", "minfill"],
        ),
        (
            &["--bipartite-lift-rate", "150"],
            &["--bipartite-lift-rate", "minfill"],
        ),
        (&["--mcs-up-to", "500"], &["--mcs-up-to"]),
        (&["--no-mcs"], &["--no-mcs", "minfill"]),
        (&["--mcsm-up-to", "500"], &["--mcsm-up-to"]),
        (&["--no-mcsm"], &["--no-mcsm", "minfill"]),
        (&["--drop-fill-up-to", "500"], &["--drop-fill-up-to"]),
        (&["--no-drop-fill"], &["--no-drop-fill", "minfill"]),
        (&["--hedge-dims", "1,2"], &["--hedge-dims"]),
        (&["--hedge-random", "2"], &["--hedge-random"]),
        (&["--hedge-reserve", "0.5"], &["--hedge-reserve"]),
        (&["--capped-restarts"], &["--capped-restarts", "minfill"]),
        (
            &["--sampling-patience", "50"],
            &["--sampling-patience", "minfill"],
        ),
        (
            &["--expensive-orders-up-to", "50000"],
            &["--expensive-orders-up-to", "minfill"],
        ),
    ];
    for &(tail, expected) in cases {
        let mut flags = vec!["--order", "minfill"];
        flags.extend_from_slice(tail);
        let mut want = expected.to_vec();
        want.push("portfolio");
        assert_refused_with(&flags, &want);
    }
}

#[test]
fn conflicting_and_out_of_range_hedge_or_stage_flags_are_refused() {
    assert_conflicting_stage_and_sampling_flags_refused();
    assert_invalid_hedge_flags_refused();
}

fn assert_conflicting_stage_and_sampling_flags_refused() {
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["--order", "portfolio", "--mcs-up-to", "500", "--no-mcs"],
            &["--mcs-up-to", "--no-mcs", "give one"],
        ),
        (
            &["--order", "portfolio", "--mcsm-up-to", "500", "--no-mcsm"],
            &["--mcsm-up-to", "--no-mcsm", "give one"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--drop-fill-up-to",
                "500",
                "--no-drop-fill",
            ],
            &["--drop-fill-up-to", "--no-drop-fill", "give one"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--sampling-patience",
                "50",
                "--no-sampling-patience",
            ],
            &["--sampling-patience", "--no-sampling-patience"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--sample-band",
                "0",
                "--sample-band-alternate",
            ],
            &["--sample-band-alternate", "--sample-band 0"],
        ),
        (
            &["--order", "flowcutter", "--steps", "10", "--budget", "10"],
            &["--steps", "--budget"],
        ),
    ];
    for &(flags, expected) in cases {
        assert_refused_with(flags, expected);
    }
}

fn assert_invalid_hedge_flags_refused() {
    let cases: &[(&[&str], &[&str])] = &[
        (&["--hedge-dims", "1,9"], &["--hedge-dims", "1..=8"]),
        (&["--hedge-dims", "2,2"], &["--hedge-dims", "twice"]),
        (&["--hedge-dims", ""], &["--hedge-dims", "1..=8"]),
        (
            &["--hedge-dims", "1,2", "--hedge-random", "2"],
            &["--hedge-dims", "--hedge-random", "give one"],
        ),
        (
            &["--no-hedge", "--hedge-dims", "1,2"],
            &["--hedge-dims", "--no-hedge", "give one"],
        ),
        (&["--hedge-random", "9"], &["--hedge-random", "1..=8"]),
        (
            &["--no-hedge", "--hedge-random", "2"],
            &["--hedge-random", "--no-hedge", "give one"],
        ),
        (
            &["--hedge-dims", "3", "--hedge-reserve", "0.5"],
            &["--hedge-reserve", "after the first", "two or more stages"],
        ),
        (
            &["--hedge-random", "1", "--hedge-reserve", "0.5"],
            &["--hedge-reserve", "two or more stages"],
        ),
        (
            &["--hedge-dims", "1,2", "--hedge-reserve", "0"],
            &["--hedge-reserve", "0 < f <= 1"],
        ),
        (
            &["--hedge-random", "2", "--hedge-reserve", "1.5"],
            &["--hedge-reserve", "0 < f <= 1"],
        ),
        (
            &["--hedge-dims", "1,2", "--hedge-reserve", "half"],
            &["--hedge-reserve", "such as 0.5"],
        ),
        (
            &["--no-hedge", "--hedge-reserve", "0.5"],
            &["--hedge-reserve", "--no-hedge", "give one"],
        ),
    ];
    for &(tail, expected) in cases {
        let mut flags = vec!["--order", "portfolio"];
        flags.extend_from_slice(tail);
        assert_refused_with(&flags, expected);
    }
}

#[test]
fn budget_dependent_and_malformed_cli_flags_are_refused() {
    let portfolio_budget_flags: &[(&[&str], &str)] = &[
        (&["--hard-budget", "10"], "--hard-budget"),
        (&["--capped-restarts"], "--capped-restarts"),
        (&["--no-bipartite-lift"], "--no-bipartite-lift"),
        (&["--bipartite-lift-rate", "150"], "--bipartite-lift-rate"),
        (&["--no-recombine"], "--no-recombine"),
        (&["--no-merge"], "--no-merge"),
        (
            &["--hedge-dims", "1,2", "--hedge-reserve", "0.5"],
            "--hedge-reserve",
        ),
    ];
    for &(tail, name) in portfolio_budget_flags {
        let mut flags = vec!["--order", "portfolio"];
        flags.extend_from_slice(tail);
        assert_refused_with(&flags, &[name, "--budget"]);
    }

    let cases: &[(&[&str], &[&str])] = &[
        (
            &["--order", "minfill", "--weights", "w.txt"],
            &["--weights", "--ties sample"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--budget",
                "20",
                "--hard-budget",
                "10",
            ],
            &["--hard-budget", "--budget", "at least"],
        ),
        (
            &["--order", "portfolio", "--sampling-patience", "0"],
            &["--sampling-patience", "positive"],
        ),
        (&["--order", "minfill", "--ties", "salt"], &["--ties"]),
        (
            &["--order", "minfill", "--budget", "0"],
            &["--budget", "positive"],
        ),
        (&["--order", "treewidth"], &["--order"]),
        (&[], &["no input graph"]),
    ];
    for &(flags, expected) in cases {
        assert_refused_with(flags, expected);
    }
}

/// A gate wider than the vertex counts the library takes is a usage error,
/// not a value quietly cut down to the largest it holds.
#[test]
fn a_vertex_gate_past_the_range_is_refused() {
    for flag in [
        "--mcs-up-to",
        "--mcsm-up-to",
        "--drop-fill-up-to",
        "--recombine-up-to",
        "--merge-up-to",
        "--local-merge-up-to",
    ] {
        assert_refused_with(
            &["--order", "portfolio", flag, "5000000000"],
            &[flag, "vertex count"],
        );
    }
}

/// The library's own band is not zero, so alternating without `--sample-band`
/// alternates between the minimum and that band and is a run, not a usage
/// error.
#[test]
fn alternating_without_a_band_of_its_own_runs() {
    let out = goatd(
        &["-", "--order", "portfolio", "--sample-band-alternate"],
        Some(&grid_gr()),
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_of(&out));
}

#[test]
fn the_trace_names_the_candidate_the_decomposition_came_from() {
    let out = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--trace",
        ],
        Some(&grid_gr()),
    );

    assert!(out.status.success(), "{}", stderr_of(&out));
    let err = stderr_of(&out);
    let mut candidates: Vec<&str> = Vec::new();
    let mut winner = None;
    for line in err.lines() {
        if let Some(rest) = line.strip_prefix("c trace winner candidate=") {
            winner = Some(rest);
        } else if let Some(rest) = line.strip_prefix("c trace candidate=") {
            candidates.push(rest);
        }
    }
    assert!(candidates.len() > 3, "one line per candidate: {err}");
    assert!(
        candidates.iter().any(|line| line.starts_with("min-fill ")),
        "{err}"
    );
    // The restarts run once the schedule admits them, and the cap stops them
    // at their count rather than at the deadline.
    assert!(
        candidates.iter().any(|line| line.starts_with("sample ")),
        "{err}"
    );
    for line in &candidates {
        assert!(
            line.contains(" width=") || line.contains(" outcome="),
            "a candidate line says what it produced: {line}"
        );
        // A candidate that produced a decomposition also carries its shape.
        if line.contains(" width=") {
            assert!(
                line.contains(" bag-mass=") && line.contains(" max-separator="),
                "a produced candidate carries its shape numbers: {line}"
            );
        }
    }
    let winner = winner.expect("a winner line");
    assert!(
        candidates.iter().any(|line| line.starts_with(winner)),
        "the winner must be one of the candidates: {err}"
    );

    let out = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
        ],
        Some(&grid_gr()),
    );
    let err = stderr_of(&out);
    assert!(!err.contains("c trace"), "no trace without the flag: {err}");
}

/// The grid is bipartite, so the lift would take a share of the window and
/// leave the hedge fewer stages than it counts here; this is about the hedge,
/// so the lift is off.
#[test]
fn the_default_portfolio_hedges_and_no_hedge_turns_that_off() {
    let out = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--no-bipartite-lift",
            "--trace",
        ],
        Some(&grid_gr()),
    );

    assert!(out.status.success(), "{}", stderr_of(&out));
    let err = stderr_of(&out);
    let mut modified: Vec<&str> = Vec::new();
    let mut diverse = 0usize;
    for line in err.lines() {
        let Some(rest) = line.strip_prefix("c trace candidate=") else {
            continue;
        };
        if rest.starts_with("sample ") {
            assert!(
                rest.contains(" pass=plain"),
                "every restart stays plain: {line}"
            );
        } else if rest.contains(" pass=modified") {
            let stage = rest.split(' ').next().expect("a stage name");
            if stage.starts_with("diverse:") {
                diverse += 1;
            } else {
                modified.push(stage);
            }
        }
    }
    assert_eq!(
        modified,
        ["min-degree", "min-fill", "min-degree"].repeat(8),
        "each of the eight default stages runs the fixed orders that read \
         weights again on its own ranking: {err}"
    );
    assert_eq!(
        diverse,
        8 * 46,
        "and the diverse pass again on that ranking: {err}"
    );

    let out = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--no-hedge",
            "--no-bipartite-lift",
            "--trace",
        ],
        Some(&grid_gr()),
    );

    assert!(out.status.success(), "{}", stderr_of(&out));
    let err = stderr_of(&out);
    assert!(err.contains("c trace candidate="), "{err}");
    assert!(
        !err.contains(" pass="),
        "one pass, so no pass to name: {err}"
    );
}

/// The trace lines of a run, without the times, so two runs' candidates can be
/// compared.
fn trace_candidates(out: &Output) -> Vec<String> {
    stderr_of(out)
        .lines()
        .filter(|line| line.starts_with("c trace"))
        .map(|line| {
            line.split(' ')
                .filter(|field| !field.starts_with("ms="))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// The stage names of the fixed orders weighted stage `index` repeated, in
/// order. The diverse candidates of the stage are left out, since there are 46
/// of them and they say nothing the three fixed orders do not;
/// `the_default_portfolio_hedges_and_no_hedge_turns_that_off` counts them.
fn modified_fixed_orders(out: &Output, index: usize) -> Vec<String> {
    let label = if index == 0 {
        " pass=modified ".to_string()
    } else {
        format!(" pass=modified:{index} ")
    };
    stderr_of(out)
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("c trace candidate=")?;
            let (stage, tail) = rest.split_once(' ')?;
            (format!(" {tail} ").contains(&label) && !stage.starts_with("diverse:"))
                .then(|| stage.to_string())
        })
        .collect()
}

/// The seeds of the ordinary restarts, in the order they ran.
fn restart_seeds(out: &Output) -> Vec<String> {
    stderr_of(out)
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("c trace candidate=sample seed=")?;
            Some(rest.split(' ').next()?.to_string())
        })
        .collect()
}

/// Assert that `out` ran `stages` weighted stages with the three fixed orders
/// each, and no stage at `stages`.
fn assert_modified_stage_count(out: &Output, stages: usize) {
    let err = stderr_of(out);
    for index in 0..stages {
        assert_eq!(
            modified_fixed_orders(out, index),
            ["min-degree", "min-fill", "min-degree"],
            "stage {index}: {err}"
        );
    }
    assert!(
        modified_fixed_orders(out, stages).is_empty(),
        "expected {stages} stages: {err}"
    );
}

#[test]
fn hedge_dims_spelling_the_default_series_runs_the_default_hedge() {
    // --hedge-dims replaces the series the portfolio would have run, so
    // spelling out the default dimensions has to be the default candidate for
    // candidate.
    let default = goatd(
        &["-", "--order", "portfolio", "--seed", "7", "--trace"],
        Some(&grid_gr()),
    );
    let spelled = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--seed",
            "7",
            "--trace",
            "--hedge-dims",
            "3,1,2,4,8,5,6,7",
        ],
        Some(&grid_gr()),
    );

    assert!(default.status.success(), "{}", stderr_of(&default));
    assert!(spelled.status.success(), "{}", stderr_of(&spelled));
    assert!(!spelled.stdout.is_empty(), "no decomposition on stdout");
    assert_eq!(default.stdout, spelled.stdout);
    assert_eq!(trace_candidates(&default), trace_candidates(&spelled));
    assert_modified_stage_count(&default, 8);
}

#[test]
fn one_dimension_in_hedge_dims_runs_that_dimension_and_nothing_else() {
    let three = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--seed",
            "7",
            "--trace",
            "--hedge-dims",
            "3",
        ],
        Some(&grid_gr()),
    );

    assert!(three.status.success(), "{}", stderr_of(&three));
    assert_modified_stage_count(&three, 1);
}

#[test]
fn hedge_dims_runs_one_weighted_stage_per_dimension() {
    let out = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--trace",
            "--hedge-dims",
            "1,2,3",
        ],
        Some(&grid_gr()),
    );

    assert!(out.status.success(), "{}", stderr_of(&out));
    let err = stderr_of(&out);
    // Every stage repeats the fixed orders that read weights, and the restarts
    // stay plain whatever the stages do.
    assert_modified_stage_count(&out, 3);
    for line in err.lines() {
        let Some(rest) = line.strip_prefix("c trace candidate=sample ") else {
            continue;
        };
        assert!(
            rest.contains(" pass=plain"),
            "a restart went modified: {line}"
        );
    }
}

#[test]
fn hedge_random_runs_one_stage_per_draw_and_costs_what_the_rankings_cost() {
    // Under a budget the restarts run to the deadline and two runs draw
    // different counts; capped, both stop at the count and the candidate
    // lists compare.
    let random = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--trace",
            "--hedge-random",
            "2",
        ],
        Some(&grid_gr()),
    );
    let ranked = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--trace",
            "--hedge-dims",
            "1,2",
        ],
        Some(&grid_gr()),
    );

    assert!(random.status.success(), "{}", stderr_of(&random));
    assert!(ranked.status.success(), "{}", stderr_of(&ranked));
    assert_modified_stage_count(&random, 2);
    assert_eq!(
        trace_candidates(&random).len(),
        trace_candidates(&ranked).len(),
        "the control runs as many candidates as the rankings do",
    );
}

#[test]
fn a_reserve_the_stages_cannot_fit_in_leaves_the_budget_to_the_restarts() {
    // A reserve of a billionth of what the plain pass left is less than any
    // stage costs, so the rule refuses every stage after the first.
    let reserved = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            "500",
            "--capped-restarts",
            "--trace",
            "--hedge-dims",
            "1,2,3",
            "--hedge-reserve",
            "0.000000001",
        ],
        Some(&grid_gr()),
    );
    let unhedged = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            "500",
            "--capped-restarts",
            "--trace",
            "--no-hedge",
        ],
        Some(&grid_gr()),
    );

    assert!(reserved.status.success(), "{}", stderr_of(&reserved));
    assert!(unhedged.status.success(), "{}", stderr_of(&unhedged));
    let err = stderr_of(&reserved);
    let skipped: Vec<&str> = err
        .lines()
        .filter(|line| line.starts_with("c trace candidate=weighted-stage "))
        .collect();

    // One line per stage left unrun, each naming its stage and the numbers the
    // rule read. The first stage is not one of them: it runs on any reserve.
    assert_eq!(skipped.len(), 2, "one line per stage left unrun: {err}");
    for (index, line) in skipped.iter().enumerate() {
        let pass = format!(" pass=modified:{} ", index + 1);
        assert!(
            format!("{line} ").contains(&pass),
            "the line names its stage: {line}"
        );
        assert!(
            line.contains(" outcome=skipped projected=")
                && line.contains(" spent=")
                && line.contains(" allowance="),
            "the line carries the numbers the rule read: {line}"
        );
    }
    assert_eq!(
        modified_fixed_orders(&reserved, 0),
        ["min-degree", "min-fill", "min-degree"],
        "the first stage ran its candidates: {err}"
    );
    for index in [1, 2] {
        assert!(
            modified_fixed_orders(&reserved, index)
                .iter()
                .all(|stage| stage == "weighted-stage"),
            "a refused stage ran a candidate: {err}"
        );
    }

    // What the refused stages did not take, the restarts get: the seeds a
    // portfolio that hedges nothing runs, from the start and in order.
    let seeds = restart_seeds(&reserved);
    assert!(!seeds.is_empty(), "no restart ran: {err}");
    let plain = restart_seeds(&unhedged);
    assert!(
        plain.starts_with(&seeds),
        "the restarts left the sequence a portfolio without a hedge runs: \
         {seeds:?} against {plain:?}",
    );
}

#[test]
fn the_reserve_applies_to_the_default_series_without_a_dimension_flag() {
    // The default series has eight stages, so the reserve has stages to refuse
    // and the flag is accepted on its own.
    let reserved = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--trace",
            "--hedge-reserve",
            "1.0",
        ],
        Some(&grid_gr()),
    );

    assert!(reserved.status.success(), "{}", stderr_of(&reserved));
    assert_modified_stage_count(&reserved, 8);
}

#[test]
fn a_reserve_that_holds_the_stages_runs_all_of_them() {
    // Capped restarts, so the two runs' candidate lists compare; see
    // hedge_random_runs_one_stage_per_draw_and_costs_what_the_rankings_cost.
    let reserved = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--trace",
            "--hedge-dims",
            "1,2,3",
            "--hedge-reserve",
            "1.0",
        ],
        Some(&grid_gr()),
    );
    let default = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            SCHEDULE_BUDGET,
            "--capped-restarts",
            "--trace",
            "--hedge-dims",
            "1,2,3",
        ],
        Some(&grid_gr()),
    );

    assert!(reserved.status.success(), "{}", stderr_of(&reserved));
    assert_modified_stage_count(&reserved, 3);
    assert!(
        !stderr_of(&reserved).contains("outcome=skipped"),
        "nothing was refused: {}",
        stderr_of(&reserved)
    );
    // On a graph whose passes cost nothing, the default reserve runs the same
    // candidates as the whole of it.
    assert_eq!(
        trace_candidates(&reserved),
        trace_candidates(&default),
        "the default reserve holds three stages on a graph this size",
    );
}

#[test]
fn a_bad_graph_is_an_error_naming_the_input() {
    let out = goatd(&["-"], Some("p tw 2 1\n1 3\n"));
    assert_eq!(out.status.code(), Some(1));
    let err = stderr_of(&out);
    assert!(err.contains("vertex 3"), "{err}");

    let out = goatd(&["/nonexistent/graph.gr"], None);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_of(&out).contains("/nonexistent/graph.gr"));
}

#[test]
fn help_prints_the_usage_and_exits_zero() {
    let out = goatd(&["--help"], None);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.starts_with("usage: goatd"));
    assert!(help.contains("portfolio"));
}

fn traced_portfolio(extra: &[&str]) -> String {
    let mut args = vec!["-", "--order", "portfolio", "--budget", "500", "--trace"];
    args.extend_from_slice(extra);
    let out = goatd(&args, Some(&grid_gr()));
    assert!(out.status.success(), "{}", stderr_of(&out));
    stderr_of(&out)
}

#[test]
fn the_portfolio_runs_an_mcs_m_candidate_and_no_mcsm_turns_it_off() {
    let with = traced_portfolio(&[]);
    assert!(with.contains("candidate=minimal-triangulation"), "{with}");

    let without = traced_portfolio(&["--no-mcsm"]);
    assert!(
        !without.contains("candidate=minimal-triangulation"),
        "{without}"
    );
}

#[test]
fn a_zero_gate_leaves_the_mcs_m_candidate_unrun() {
    let err = traced_portfolio(&["--mcsm-up-to", "0"]);
    assert!(!err.contains("candidate=minimal-triangulation"), "{err}");
}

#[test]
fn the_fill_dropping_pass_leaves_a_valid_decomposition() {
    for extra in [&["--no-drop-fill"][..], &["--drop-fill-up-to", "1000"][..]] {
        let mut args = vec!["-", "--order", "portfolio", "--budget", "500"];
        args.extend_from_slice(extra);
        let out = goatd(&args, Some(&grid_gr()));
        assert!(out.status.success(), "{}", stderr_of(&out));
        let td = TreeDecomposition::from_td(&String::from_utf8_lossy(&out.stdout))
            .expect("the CLI writes a .td");
        td.validate(&Graph::from_gr(&grid_gr()).expect("the fixture parses"))
            .expect("the CLI writes a decomposition of its input");
    }
}

#[test]
fn the_portfolio_runs_a_maximum_cardinality_candidate_and_no_mcs_turns_it_off() {
    let with = traced_portfolio(&[]);
    assert!(with.contains("candidate=maximum-cardinality"), "{with}");

    let without = traced_portfolio(&["--no-mcs"]);
    assert!(
        !without.contains("candidate=maximum-cardinality"),
        "{without}"
    );
    assert!(
        without.contains("candidate=minimal-triangulation"),
        "--no-mcs leaves the MCS-M candidate alone: {without}",
    );
}

#[test]
fn a_zero_gate_leaves_the_maximum_cardinality_candidate_unrun() {
    let err = traced_portfolio(&["--mcs-up-to", "0"]);
    assert!(!err.contains("candidate=maximum-cardinality"), "{err}");
}
