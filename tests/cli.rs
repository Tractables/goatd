//! The `goatd` binary, driven over PACE files: every order decomposes a
//! small graph into a valid `.td`, the output goes where it is asked to go,
//! and a flag the chosen order cannot act on is refused with a message that
//! names both.

use std::path::PathBuf;
use std::process::{Command, Output};

use goatd::{Graph, TreeDecomposition};

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
        let out = goatd(&["-", "--ties", "sample", "--seed", seed], Some(&grid_gr()));
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

/// An order-specific flag is refused under another order, and the message
/// names the flag and both orders.
#[test]
fn an_unsupported_flag_is_refused_naming_the_flag_and_the_order() {
    let cases: &[(&[&str], &[&str])] = &[
        (&["--steps", "10"], &["--steps", "minfill", "flowcutter"]),
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
        (&["--weights", "w.txt"], &["--weights", "--ties sample"]),
        (
            &["--order", "flowcutter", "--weights", "w.txt"],
            &["--weights", "flowcutter", "minfill or mindegree"],
        ),
        (
            &["--order", "flowcutter", "--steps", "10", "--budget", "10"],
            &["--steps", "--budget"],
        ),
        (
            &["--hard-budget", "10"],
            &["--hard-budget", "minfill", "portfolio"],
        ),
        (
            &["--order", "portfolio", "--hard-budget", "10"],
            &["--hard-budget", "--budget"],
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
        (&["--no-hedge"], &["--no-hedge", "minfill", "portfolio"]),
        (&["--hedge-dims", "1,2"], &["--hedge-dims", "portfolio"]),
        (
            &["--order", "portfolio", "--hedge-dims", "1,9"],
            &["--hedge-dims", "1..=8"],
        ),
        (
            &["--order", "portfolio", "--hedge-dims", "2,2"],
            &["--hedge-dims", "twice"],
        ),
        (
            &["--order", "portfolio", "--hedge-dims", ""],
            &["--hedge-dims", "1..=8"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--hedge-dims",
                "1,2",
                "--hedge-random",
                "2",
            ],
            &["--hedge-dims", "--hedge-random", "give one"],
        ),
        (
            &["--order", "portfolio", "--no-hedge", "--hedge-dims", "1,2"],
            &["--hedge-dims", "--no-hedge", "give one"],
        ),
        (&["--hedge-random", "2"], &["--hedge-random", "portfolio"]),
        (
            &["--order", "portfolio", "--hedge-random", "9"],
            &["--hedge-random", "1..=8"],
        ),
        (
            &["--order", "portfolio", "--no-hedge", "--hedge-random", "2"],
            &["--hedge-random", "--no-hedge", "give one"],
        ),
        (
            &["--hedge-reserve", "0.5"],
            &["--hedge-reserve", "portfolio"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--hedge-dims",
                "3",
                "--hedge-reserve",
                "0.5",
            ],
            &["--hedge-reserve", "after the first", "two or more stages"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--hedge-random",
                "1",
                "--hedge-reserve",
                "0.5",
            ],
            &["--hedge-reserve", "two or more stages"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--hedge-dims",
                "1,2",
                "--hedge-reserve",
                "0",
            ],
            &["--hedge-reserve", "0 < f <= 1"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--hedge-random",
                "2",
                "--hedge-reserve",
                "1.5",
            ],
            &["--hedge-reserve", "0 < f <= 1"],
        ),
        (
            &[
                "--order",
                "portfolio",
                "--hedge-dims",
                "1,2",
                "--hedge-reserve",
                "half",
            ],
            &["--hedge-reserve", "such as 0.5"],
        ),
        (
            &["--order", "flowcutter", "--trace"],
            &["--trace", "flowcutter", "portfolio"],
        ),
        (&["--ties", "salt"], &["--ties"]),
        (&["--budget", "0"], &["--budget", "positive"]),
        (&["--order", "treewidth"], &["--order"]),
        (&[], &["no input graph"]),
    ];
    for &(flags, expected) in cases {
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
}

#[test]
fn the_trace_names_the_candidate_the_decomposition_came_from() {
    let out = goatd(
        &["-", "--order", "portfolio", "--budget", "500", "--trace"],
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
    // A 500 ms budget is below the extended-sampling threshold, so the
    // schedule is the fixed candidates and ordinary restarts.
    assert!(
        candidates.iter().any(|line| line.starts_with("sample ")),
        "{err}"
    );
    for line in &candidates {
        assert!(
            line.contains(" width=") || line.contains(" outcome="),
            "a candidate line says what it produced: {line}"
        );
    }
    let winner = winner.expect("a winner line");
    assert!(
        candidates.iter().any(|line| line.starts_with(winner)),
        "the winner must be one of the candidates: {err}"
    );

    let out = goatd(
        &["-", "--order", "portfolio", "--budget", "500"],
        Some(&grid_gr()),
    );
    let err = stderr_of(&out);
    assert!(!err.contains("c trace"), "no trace without the flag: {err}");
}

#[test]
fn the_default_portfolio_hedges_and_no_hedge_turns_that_off() {
    let out = goatd(
        &["-", "--order", "portfolio", "--budget", "500", "--trace"],
        Some(&grid_gr()),
    );

    assert!(out.status.success(), "{}", stderr_of(&out));
    let err = stderr_of(&out);
    let mut modified: Vec<&str> = Vec::new();
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
            modified.push(rest.split(' ').next().expect("a stage name"));
        }
    }
    assert_eq!(
        modified,
        ["min-degree", "min-fill", "min-degree"].repeat(8),
        "each of the eight default stages runs the fixed orders that read \
         weights again on its own ranking: {err}"
    );

    let out = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            "500",
            "--no-hedge",
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

/// The stage names of the candidates of weighted stage `index`, in order.
fn modified_stages(out: &Output, index: usize) -> Vec<String> {
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
            format!(" {tail} ")
                .contains(&label)
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
    for index in 0..8 {
        assert_eq!(
            modified_stages(&default, index),
            ["min-degree", "min-fill", "min-degree"],
            "stage {index} of the default series: {}",
            stderr_of(&default)
        );
    }
    assert!(
        modified_stages(&default, 8).is_empty(),
        "the default series is eight stages: {}",
        stderr_of(&default)
    );
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
    assert_eq!(
        modified_stages(&three, 0),
        ["min-degree", "min-fill", "min-degree"],
        "one dimension runs its weighted stage: {}",
        stderr_of(&three)
    );
    assert!(
        modified_stages(&three, 1).is_empty(),
        "one dimension is one weighted stage: {}",
        stderr_of(&three)
    );
}

#[test]
fn hedge_dims_runs_one_weighted_stage_per_dimension() {
    let out = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            "500",
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
    for index in 0..3 {
        assert_eq!(
            modified_stages(&out, index),
            ["min-degree", "min-fill", "min-degree"],
            "stage {index} of the series: {err}"
        );
    }
    assert!(
        modified_stages(&out, 3).is_empty(),
        "three dimensions are three stages: {err}"
    );
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
    let random = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            "500",
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
            "500",
            "--trace",
            "--hedge-dims",
            "1,2",
        ],
        Some(&grid_gr()),
    );

    assert!(random.status.success(), "{}", stderr_of(&random));
    assert!(ranked.status.success(), "{}", stderr_of(&ranked));
    for index in 0..2 {
        assert_eq!(
            modified_stages(&random, index),
            ["min-degree", "min-fill", "min-degree"],
            "stage {index} of the control: {}",
            stderr_of(&random)
        );
    }
    assert!(
        modified_stages(&random, 2).is_empty(),
        "two draws are two stages: {}",
        stderr_of(&random)
    );
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
        modified_stages(&reserved, 0),
        ["min-degree", "min-fill", "min-degree"],
        "the first stage ran its candidates: {err}"
    );
    for index in 1..3 {
        assert!(
            modified_stages(&reserved, index)
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
            "500",
            "--trace",
            "--hedge-reserve",
            "1.0",
        ],
        Some(&grid_gr()),
    );

    assert!(reserved.status.success(), "{}", stderr_of(&reserved));
    for index in 0..8 {
        assert_eq!(
            modified_stages(&reserved, index),
            ["min-degree", "min-fill", "min-degree"],
            "stage {index} of the default series: {}",
            stderr_of(&reserved)
        );
    }
}

#[test]
fn a_reserve_that_holds_the_stages_runs_all_of_them() {
    let reserved = goatd(
        &[
            "-",
            "--order",
            "portfolio",
            "--budget",
            "500",
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
            "500",
            "--trace",
            "--hedge-dims",
            "1,2,3",
        ],
        Some(&grid_gr()),
    );

    assert!(reserved.status.success(), "{}", stderr_of(&reserved));
    for index in 0..3 {
        assert_eq!(
            modified_stages(&reserved, index),
            ["min-degree", "min-fill", "min-degree"],
            "stage {index} ran under the whole reserve: {}",
            stderr_of(&reserved)
        );
    }
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
