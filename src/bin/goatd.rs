//! `goatd`: a PACE `.gr` graph in, a `.td` tree decomposition out, by any of
//! the library's routes.
//!
//! Usage and every option are in [`USAGE`]. A flag that means nothing under
//! the chosen order is an error naming both, never silently ignored.

use std::io::{Read, Write};
use std::process::exit;
use std::time::{Duration, Instant};

use goatd::elimination::{
    Config, PortfolioConfig, elimination_td, five_slot_portfolio, refine_td_with_flowcutter_cut,
};
use goatd::flowcutter::{
    FC_BARE_TIMEOUT_MS, FC_DEFAULT_ITERS, FC_DEFAULT_STEPS_ITERS, FC_PATIENCE_MS_PARAMETRIZED,
    FcBudget, flowcutter_td,
};
use goatd::{Graph, TreeDecomposition};

const USAGE: &str = "\
usage: goatd <graph.gr | -> [options]

Reads a PACE .gr graph (- for stdin) and writes a PACE .td tree
decomposition to stdout, or to --out.

options:
  --out <file.td>       write the decomposition here instead of stdout
  --order <name>        which construction runs (default: minfill)
                          minfill             greedy min-fill order
                          mindegree           greedy min-degree order
                          nested-dissection   multilevel nested dissection
                          flowcutter          the FlowCutter solver
                          portfolio           several orders under one budget,
                                              keeping the narrowest
  --seed <n>            tie-breaking seed for every order but flowcutter
                        (default: 0)
  --ties sample         minfill / mindegree only: break ties by weighted
                        sampling from the whole tie set instead of by salt
  --weights <file>      with --ties sample: one integer per vertex, one per
                        line; a smaller weight is eliminated earlier
                        (default: every vertex weighs the same)
  --budget <ms>         wall-clock budget: the elimination orders' soft
                        deadline, flowcutter's run time, the portfolio's
                        soft deadline, and the refinement's deadline
  --steps <n>           flowcutter only: a step budget in place of a clock,
                        for a run that repeats exactly
  --refine              re-cut the decomposition along FlowCutter separators
                        before writing it
  -h, --help            this text
";

/// Which construction `--order` named.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Order {
    MinFill,
    MinDegree,
    NestedDissection,
    FlowCutter,
    Portfolio,
}

impl Order {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "minfill" => Order::MinFill,
            "mindegree" => Order::MinDegree,
            "nested-dissection" => Order::NestedDissection,
            "flowcutter" => Order::FlowCutter,
            "portfolio" => Order::Portfolio,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Order::MinFill => "minfill",
            Order::MinDegree => "mindegree",
            Order::NestedDissection => "nested-dissection",
            Order::FlowCutter => "flowcutter",
            Order::Portfolio => "portfolio",
        }
    }
}

/// The command line, parsed and checked for flags that the chosen order
/// cannot act on.
struct Args {
    input: String,
    out: Option<String>,
    order: Order,
    seed: Option<u64>,
    sample: bool,
    weights: Option<String>,
    budget_ms: Option<u64>,
    steps: Option<i64>,
    refine: bool,
}

fn usage_error(msg: &str) -> ! {
    eprintln!("goatd: {msg}\n\n{USAGE}");
    exit(2)
}

fn fail(msg: &str) -> ! {
    eprintln!("goatd: {msg}");
    exit(1)
}

fn parse_args(argv: &[String]) -> Args {
    let mut input: Option<String> = None;
    let mut out = None;
    let mut order = None;
    let mut seed = None;
    let mut sample = false;
    let mut weights = None;
    let mut budget_ms = None;
    let mut steps = None;
    let mut refine = false;

    let mut i = 0;
    let value = |i: &mut usize, flag: &str| -> String {
        *i += 1;
        argv.get(*i)
            .cloned()
            .unwrap_or_else(|| usage_error(&format!("{flag} needs a value")))
    };
    let number = |i: &mut usize, flag: &str| -> u64 {
        let v = value(i, flag);
        v.parse().unwrap_or_else(|_| {
            usage_error(&format!("{flag} wants a non-negative integer, got {v:?}"))
        })
    };
    while i < argv.len() {
        let arg = argv[i].as_str();
        match arg {
            "-h" | "--help" => {
                print!("{USAGE}");
                exit(0)
            }
            "--out" => out = Some(value(&mut i, arg)),
            "--order" => {
                let name = value(&mut i, arg);
                order = Some(
                    Order::parse(&name)
                        .unwrap_or_else(|| usage_error(&format!("unknown --order {name:?}"))),
                );
            }
            "--seed" => seed = Some(number(&mut i, arg)),
            "--ties" => {
                let how = value(&mut i, arg);
                if how != "sample" {
                    usage_error(&format!("--ties takes only `sample`, got {how:?}"));
                }
                sample = true;
            }
            "--weights" => weights = Some(value(&mut i, arg)),
            "--budget" => budget_ms = Some(number(&mut i, arg)),
            "--steps" => {
                let n = number(&mut i, arg);
                steps = Some(
                    i64::try_from(n)
                        .ok()
                        .filter(|&n| n > 0)
                        .unwrap_or_else(|| usage_error("--steps wants a positive step count")),
                );
            }
            "--refine" => refine = true,
            _ if arg.starts_with('-') && arg != "-" => {
                usage_error(&format!("unknown option {arg:?}"))
            }
            _ => {
                if input.replace(arg.to_string()).is_some() {
                    usage_error("more than one input graph given");
                }
            }
        }
        i += 1;
    }

    let Some(input) = input else {
        usage_error("no input graph given");
    };
    let order = order.unwrap_or(Order::MinFill);

    // Every flag the chosen order cannot act on is an error naming both.
    let needs = |flag: &str, ok: bool, orders: &str| {
        if !ok {
            usage_error(&format!(
                "{flag} means nothing under --order {}; it needs --order {orders}",
                order.name()
            ));
        }
    };
    let greedy = matches!(order, Order::MinFill | Order::MinDegree);
    if sample {
        needs("--ties sample", greedy, "minfill or mindegree");
    }
    if weights.is_some() && !sample {
        usage_error("--weights means nothing without --ties sample");
    }
    if seed.is_some() {
        needs(
            "--seed",
            order != Order::FlowCutter,
            "minfill, mindegree, nested-dissection or portfolio",
        );
    }
    if steps.is_some() {
        needs("--steps", order == Order::FlowCutter, "flowcutter");
        if budget_ms.is_some() {
            usage_error("--steps and --budget both bound flowcutter; give one");
        }
    }

    Args {
        input,
        out,
        order,
        seed,
        sample,
        weights,
        budget_ms,
        steps,
        refine,
    }
}

fn read_input(path: &str) -> String {
    let mut text = String::new();
    let result = if path == "-" {
        std::io::stdin().read_to_string(&mut text)
    } else {
        std::fs::File::open(path).and_then(|mut f| f.read_to_string(&mut text))
    };
    if let Err(e) = result {
        fail(&format!("cannot read {path}: {e}"));
    }
    text
}

/// One weight per vertex, one per line, in vertex order.
fn read_weights(path: &str, num_vertices: u32) -> Vec<u32> {
    let text = read_input(path);
    let weights: Vec<u32> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('c'))
        .map(|l| {
            l.parse()
                .unwrap_or_else(|_| fail(&format!("{path}: not a vertex weight: {l:?}")))
        })
        .collect();
    if weights.len() != num_vertices as usize {
        fail(&format!(
            "{path}: {} weights for a graph of {num_vertices} vertices",
            weights.len()
        ));
    }
    weights
}

fn decompose(args: &Args, graph: &Graph) -> TreeDecomposition {
    let seed = args.seed.unwrap_or(0);
    let budget = args.budget_ms.map(Duration::from_millis);
    let n = graph.num_vertices as usize;
    let weight: Vec<u32> = match &args.weights {
        Some(path) => read_weights(path, graph.num_vertices),
        None => vec![1; n],
    };
    match args.order {
        Order::MinFill | Order::MinDegree => {
            let config = match (args.order, args.sample) {
                (Order::MinFill, false) => Config::MinFill,
                (Order::MinFill, true) => Config::MinFillSampled { weight: &weight },
                (_, false) => Config::MinDegree,
                (_, true) => Config::MinDegreeSampled { weight: &weight },
            };
            elimination_td(graph, config, seed, budget)
        }
        Order::NestedDissection => elimination_td(graph, Config::NestedDissection, seed, budget),
        Order::FlowCutter => {
            let fc_budget = match (args.steps, args.budget_ms) {
                (Some(steps), _) => FcBudget::Steps {
                    steps,
                    iters: FC_DEFAULT_STEPS_ITERS,
                },
                (None, Some(ms)) => FcBudget::timed(
                    i64::try_from(ms).unwrap_or(i64::MAX),
                    FC_PATIENCE_MS_PARAMETRIZED,
                    FC_DEFAULT_ITERS,
                ),
                (None, None) => FcBudget::timed(
                    FC_BARE_TIMEOUT_MS,
                    FC_PATIENCE_MS_PARAMETRIZED,
                    FC_DEFAULT_ITERS,
                ),
            };
            flowcutter_td(graph, fc_budget).unwrap_or_else(|e| fail(&e.to_string()))
        }
        Order::Portfolio => {
            let tds = five_slot_portfolio(
                graph,
                &weight,
                seed,
                PortfolioConfig::five_slot(args.budget_ms),
            );
            tds.into_iter()
                .next()
                .unwrap_or_else(|| fail("the portfolio produced no decomposition"))
        }
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);

    let start = Instant::now();
    let graph = Graph::from_gr(&read_input(&args.input))
        .unwrap_or_else(|e| fail(&format!("{}: {e}", args.input)));

    let mut td = decompose(&args, &graph);
    if args.refine {
        let deadline = args.budget_ms.map(|ms| start + Duration::from_millis(ms));
        let all_vertices: Vec<u32> = (0..graph.num_vertices).collect();
        td = refine_td_with_flowcutter_cut(td, &all_vertices, &graph.edges, deadline);
    }

    let text = td.to_td(graph.num_vertices);
    let written = match &args.out {
        Some(path) => std::fs::write(path, text).map_err(|e| format!("cannot write {path}: {e}")),
        None => std::io::stdout()
            .write_all(text.as_bytes())
            .map_err(|e| format!("cannot write to stdout: {e}")),
    };
    if let Err(e) = written {
        fail(&e);
    }
}
