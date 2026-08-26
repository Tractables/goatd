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
        &["--order", "schedule", "--budget", "500"],
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

/// A flag that means nothing under the chosen order is refused, and the
/// message names the flag, the order it was given under, and the order it
/// needs.
#[test]
fn an_inert_flag_is_refused_naming_the_flag_and_the_order() {
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
            &["--order", "schedule", "--ties", "sample"],
            &["--ties sample", "schedule"],
        ),
        (&["--weights", "w.txt"], &["--weights", "--ties sample"]),
        (
            &["--order", "flowcutter", "--steps", "10", "--budget", "10"],
            &["--steps", "--budget"],
        ),
        (&["--ties", "salt"], &["--ties"]),
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
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("usage: goatd"));
}
