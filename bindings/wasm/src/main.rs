//! goatd built for the browser: a PACE `.gr` graph goes in, a PACE `.td` tree
//! decomposition comes out.
//!
//! Emscripten links a program, so `main` has to exist. It does nothing and the
//! page never runs it; `index.html` calls [`goatd_decompose`] through the
//! module's `ccall`.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use goatd::Graph;
use goatd::elimination::{Order, decompose as eliminate};
use goatd::flowcutter::{Budget, decompose as flowcutter};
use goatd::portfolio::decompose_standard as portfolio;

/// Greedy min-fill elimination.
const ORDER_MIN_FILL: u32 = 0;
/// Greedy min-degree elimination.
const ORDER_MIN_DEGREE: u32 = 1;
/// Multilevel nested dissection.
const ORDER_NESTED_DISSECTION: u32 = 2;
/// The vendored FlowCutter solver.
const ORDER_FLOWCUTTER: u32 = 3;
/// Several orders under one budget, keeping the narrowest result.
const ORDER_PORTFOLIO: u32 = 4;

fn main() {}

/// Decompose the PACE `.gr` graph in `gr` and return the `.td` text, or a
/// message beginning with `error: ` when the graph or the settings are
/// rejected. The caller releases the string with [`goatd_string_free`].
///
/// `order` is one of the `ORDER_` values above. `budget_ms` is what the
/// construction may spend, or 0 for no limit; FlowCutter reads 0 as its own
/// default rather than as unlimited. The elimination orders and the portfolio
/// stop for good at twice their soft deadline, so a call can take about twice
/// `budget_ms`. Both are `u32` so that every argument crosses the boundary as
/// a JavaScript number.
///
/// `seed` breaks ties for every order but `ORDER_FLOWCUTTER`, which does not
/// break ties this way and is given a non-zero seed as an error rather than
/// ignoring it.
///
/// # Safety
///
/// `gr` must point to a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn goatd_decompose(
    gr: *const c_char,
    order: u32,
    seed: u32,
    budget_ms: u32,
) -> *mut c_char {
    if gr.is_null() {
        return into_c_string(Err("no graph".to_owned()));
    }
    // SAFETY: the caller guarantees a NUL-terminated string, and `ccall`'s
    // "string" argument type produces one.
    let text = unsafe { CStr::from_ptr(gr) };
    let result = match text.to_str() {
        // A panic reaching Emscripten aborts the module, and every later call
        // on it fails; the panic hook has already written the details to the
        // browser console by the time this reports it.
        Ok(text) => catch_unwind(AssertUnwindSafe(|| {
            run(text, order, u64::from(seed), u64::from(budget_ms))
        }))
        .unwrap_or_else(|_| Err("goatd panicked; please report it as a bug".to_owned())),
        Err(_) => Err("the graph is not UTF-8".to_owned()),
    };
    into_c_string(result)
}

/// Release a string [`goatd_decompose`] returned.
///
/// # Safety
///
/// `text` must be null or a string [`goatd_decompose`] returned and nothing
/// has freed since.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn goatd_string_free(text: *mut c_char) {
    if !text.is_null() {
        // SAFETY: the pointer came from `CString::into_raw` in
        // `into_c_string`, and this is its only release point.
        drop(unsafe { CString::from_raw(text) });
    }
}

fn run(gr: &str, order: u32, seed: u64, budget_ms: u64) -> Result<String, String> {
    if seed != 0 && order == ORDER_FLOWCUTTER {
        return Err(
            "seed is not valid with flowcutter, which does not break ties this way".to_owned(),
        );
    }
    let graph = Graph::from_gr(gr).map_err(|e| e.to_string())?;
    let budget = (budget_ms != 0).then(|| Duration::from_millis(budget_ms));
    let td = match order {
        ORDER_MIN_FILL => eliminate(&graph, Order::MinFill, seed, budget),
        ORDER_MIN_DEGREE => eliminate(&graph, Order::MinDegree, seed, budget),
        ORDER_NESTED_DISSECTION => eliminate(&graph, Order::NestedDissection, seed, budget),
        ORDER_FLOWCUTTER => flowcutter(&graph, Budget::standalone(budget, None)),
        ORDER_PORTFOLIO => portfolio(&graph, seed, budget),
        unknown => return Err(format!("unknown order {unknown}")),
    };
    td.map(|td| td.to_td()).map_err(|e| e.to_string())
}

fn into_c_string(result: Result<String, String>) -> *mut c_char {
    let text = match result {
        Ok(td) => td,
        Err(message) => format!("error: {message}"),
    };
    // A NUL inside a `.td` text or an error message would be a goatd bug, so
    // report it as one rather than inventing a truncated result.
    CString::new(text)
        .unwrap_or_else(|_| c"error: the result contains a NUL byte".to_owned())
        .into_raw()
}
