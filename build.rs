//! Build script: compiles the vendored FlowCutter tree-decomposition builder
//! (`vendor/treedecomp/`, BSD-2) into one static archive. It is always built —
//! there is no configuration in which this crate ships without it.

use std::process::Command;

fn main() {
    // docs.rs builds documentation, not a binary: rustdoc type-checks the crate
    // and never links it, so the `extern "C"` declarations resolve to nothing.
    // `DOCS_RS` is set by docs.rs itself; a normal build never takes this path.
    println!("cargo:rerun-if-env-changed=DOCS_RS");
    if std::env::var_os("DOCS_RS").is_some() {
        return;
    }

    println!("cargo:rerun-if-env-changed=GOATD_CXX");
    let cxx = find_cxx();
    assert!(
        have(&cxx),
        "goatd's vendored FlowCutter needs a C++20 compiler, and `{cxx}` does not run — \
         install one (gcc 12 or newer, or a recent clang), or name another in GOATD_CXX."
    );

    cc::Build::new()
        .cpp(true)
        // The compiler comes from `find_cxx`, not from `cc`'s own `CXX` lookup,
        // so `GOATD_CXX` chooses it whole.
        .compiler(&cxx)
        .std("c++20")
        .opt_level(3)
        .define("NDEBUG", None)
        .warnings(false)
        .include("vendor/treedecomp")
        .include("vendor/treedecomp/upstream")
        .include("vendor/treedecomp/upstream/flow-cutter-pace17/src")
        .file("vendor/treedecomp/ffi.cpp")
        .file("vendor/treedecomp/heap_selftest.cpp")
        .file("vendor/treedecomp/upstream/IFlowCutter.cpp")
        .file("vendor/treedecomp/upstream/TreeDecomposition.cpp")
        .file("vendor/treedecomp/upstream/graph.cpp")
        .file("vendor/treedecomp/upstream/flow-cutter-pace17/src/cell.cpp")
        .file("vendor/treedecomp/upstream/flow-cutter-pace17/src/greedy_order.cpp")
        .file("vendor/treedecomp/upstream/flow-cutter-pace17/src/tree_decomposition.cpp")
        // `pace.cpp` is left out: it holds the upstream binary's `main`.
        .compile("treedecomp");

    println!("cargo:rerun-if-changed=vendor/treedecomp/");
}

/// The upstream sources use C++20; Ubuntu 22.04 still ships gcc-11 as `g++`.
/// Prefer an explicit `GOATD_CXX`, else the newest versioned gcc on PATH, else
/// plain `g++`.
fn find_cxx() -> String {
    if let Ok(cxx) = std::env::var("GOATD_CXX")
        && !cxx.is_empty()
    {
        return cxx;
    }
    for v in ["14", "13", "12"] {
        let candidate = format!("g++-{v}");
        if have(&candidate) {
            return candidate;
        }
    }
    "g++".into()
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
