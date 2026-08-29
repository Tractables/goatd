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
    let mut build = cc::Build::new();
    if let Some(cxx) = find_cxx() {
        assert!(
            have(&cxx),
            "goatd's vendored FlowCutter needs a C++20 compiler, and `{cxx}` does not run — \
             install one (gcc 12 or newer, or a recent clang), or name another in GOATD_CXX."
        );
        // The compiler comes from `find_cxx`, not from `cc`'s own `CXX` lookup,
        // so `GOATD_CXX` chooses it whole.
        build.compiler(&cxx);
    }

    build
        .cpp(true)
        // `cc` spells this per toolchain: `-std=c++20` GNU-style, `/std:c++20` MSVC.
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

/// An explicit `GOATD_CXX` wins everywhere. On Linux, where the default `g++`
/// can predate C++20 (Ubuntu 22.04 still ships gcc-11), fall back to the
/// newest versioned gcc on PATH, then plain `g++`. Elsewhere `None`: `cc`
/// picks the platform's own compiler (Apple clang on macOS, MSVC on Windows),
/// and a g++ found on PATH there could be one that cannot link with the rest
/// of the build.
fn find_cxx() -> Option<String> {
    if let Ok(cxx) = std::env::var("GOATD_CXX")
        && !cxx.is_empty()
    {
        return Some(cxx);
    }
    // Set by cargo for build scripts; the target's OS, not the host's.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return None;
    }
    for v in ["14", "13", "12"] {
        let candidate = format!("g++-{v}");
        if have(&candidate) {
            return Some(candidate);
        }
    }
    Some("g++".into())
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
