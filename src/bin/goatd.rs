//! `goatd`: a PACE `.gr` graph in, a `.td` tree decomposition out, by any of
//! the library's routes.
//!
//! Usage and every option are in [`USAGE`]. Order-specific flags are rejected
//! when used with another construction.

use std::io::{BufWriter, Read, Write};
use std::process::exit;
use std::time::{Duration, Instant};

use goatd::decomposition::refine_with_flowcutter;
use goatd::elimination::{Order, decompose as eliminate};
use goatd::flowcutter::{Budget, decompose as flowcutter};
use goatd::portfolio::{PortfolioConfig, decompose as portfolio};
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
                        soft deadline, sampling effort, and trailing
                        FlowCutter slot, and the refinement's deadline
  --hard-budget <ms>    portfolio only: hard wall-clock cutoff; defaults to
                        twice --budget
  --steps <n>           flowcutter only: a step budget in place of a clock,
                        for a run that repeats exactly
  --refine              re-cut the decomposition along FlowCutter separators
                        before writing it
  -h, --help            this text
";

/// Which construction `--order` named.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    MinFill,
    MinDegree,
    NestedDissection,
    FlowCutter,
    Portfolio,
}

impl Method {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "minfill" => Method::MinFill,
            "mindegree" => Method::MinDegree,
            "nested-dissection" => Method::NestedDissection,
            "flowcutter" => Method::FlowCutter,
            "portfolio" => Method::Portfolio,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Method::MinFill => "minfill",
            Method::MinDegree => "mindegree",
            Method::NestedDissection => "nested-dissection",
            Method::FlowCutter => "flowcutter",
            Method::Portfolio => "portfolio",
        }
    }
}

/// The command line, parsed and checked for flags that the chosen order
/// cannot act on.
struct Args {
    input: String,
    out: Option<String>,
    order: Method,
    seed: Option<u64>,
    sample: bool,
    weights: Option<String>,
    budget: Option<Duration>,
    hard_budget: Option<Duration>,
    steps: Option<u64>,
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
    let mut budget = None;
    let mut hard_budget = None;
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
                    Method::parse(&name)
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
            "--budget" => {
                let milliseconds = number(&mut i, arg);
                if milliseconds == 0 {
                    usage_error("--budget wants a positive millisecond count");
                }
                budget = Some(Duration::from_millis(milliseconds));
            }
            "--hard-budget" => {
                let milliseconds = number(&mut i, arg);
                if milliseconds == 0 {
                    usage_error("--hard-budget wants a positive millisecond count");
                }
                hard_budget = Some(Duration::from_millis(milliseconds));
            }
            "--steps" => {
                let n = number(&mut i, arg);
                if n == 0 {
                    usage_error("--steps wants a positive step count");
                }
                steps = Some(n);
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
    let order = order.unwrap_or(Method::MinFill);

    // Validate order-specific flags after the order and all flags are known.
    let needs = |flag: &str, ok: bool, orders: &str| {
        if !ok {
            usage_error(&format!(
                "{flag} is not valid with --order {}; use --order {orders}",
                order.name()
            ));
        }
    };
    let greedy = matches!(order, Method::MinFill | Method::MinDegree);
    if sample {
        needs("--ties sample", greedy, "minfill or mindegree");
    }
    if weights.is_some() {
        needs("--weights", greedy, "minfill or mindegree");
        if !sample {
            usage_error("--weights requires --ties sample");
        }
    }
    if seed.is_some() {
        needs(
            "--seed",
            order != Method::FlowCutter,
            "minfill, mindegree, nested-dissection or portfolio",
        );
    }
    if steps.is_some() {
        needs("--steps", order == Method::FlowCutter, "flowcutter");
        if budget.is_some() {
            usage_error("--steps and --budget both bound flowcutter; give one");
        }
    }
    if let Some(hard) = hard_budget {
        needs("--hard-budget", order == Method::Portfolio, "portfolio");
        let Some(soft) = budget else {
            usage_error("--hard-budget requires --budget");
        };
        if hard < soft {
            usage_error("--hard-budget must be at least --budget");
        }
    }

    Args {
        input,
        out,
        order,
        seed,
        sample,
        weights,
        budget,
        hard_budget,
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

fn portfolio_config(args: &Args, process_elapsed: Duration) -> PortfolioConfig {
    let Some(soft_budget) = args.budget else {
        return PortfolioConfig::standard();
    };
    let hard_budget = args
        .hard_budget
        .unwrap_or_else(|| soft_budget.saturating_mul(2));
    PortfolioConfig::standard_with_budget(soft_budget)
        .with_soft_budget(soft_budget.saturating_sub(process_elapsed))
        .with_hard_budget(hard_budget.saturating_sub(process_elapsed))
}

fn construct(args: &Args, graph: &Graph, process_elapsed: Duration) -> TreeDecomposition {
    let seed = args.seed.unwrap_or(0);
    let budget = args.budget;
    match args.order {
        Method::MinFill | Method::MinDegree => {
            let weights = args.sample.then(|| match &args.weights {
                Some(path) => read_weights(path, graph.num_vertices()),
                None => vec![1; graph.num_vertices() as usize],
            });
            let config = match (args.order, args.sample) {
                (Method::MinFill, false) => Order::MinFill,
                (Method::MinFill, true) => Order::MinFillSampled {
                    weights: weights.as_deref().expect("sampling creates weights"),
                },
                (_, false) => Order::MinDegree,
                (_, true) => Order::MinDegreeSampled {
                    weights: weights.as_deref().expect("sampling creates weights"),
                },
            };
            eliminate(graph, config, seed, budget).unwrap_or_else(|error| fail(&error.to_string()))
        }
        Method::NestedDissection => eliminate(graph, Order::NestedDissection, seed, budget)
            .unwrap_or_else(|error| fail(&error.to_string())),
        Method::FlowCutter => flowcutter(graph, Budget::standalone(budget, args.steps))
            .unwrap_or_else(|e| fail(&e.to_string())),
        Method::Portfolio => {
            let weights = vec![1; graph.num_vertices() as usize];
            portfolio(
                graph,
                &weights,
                seed,
                portfolio_config(args, process_elapsed),
            )
            .unwrap_or_else(|error| fail(&error.to_string()))
        }
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);

    let start = Instant::now();
    let graph = Graph::from_gr(&read_input(&args.input))
        .unwrap_or_else(|e| fail(&format!("{}: {e}", args.input)));

    let mut td = construct(&args, &graph, start.elapsed());
    if args.refine {
        let remaining = args
            .budget
            .map(|budget| budget.saturating_sub(start.elapsed()));
        td = refine_with_flowcutter(td, &graph, remaining)
            .unwrap_or_else(|error| fail(&error.to_string()));
    }

    let written = match &args.out {
        Some(path) => std::fs::File::create(path)
            .and_then(|file| write_decomposition(&td, file))
            .map_err(|e| format!("cannot write {path}: {e}")),
        None => write_decomposition(&td, std::io::stdout().lock())
            .map_err(|e| format!("cannot write to stdout: {e}")),
    };
    if let Err(e) = written {
        fail(&e);
    }
}

fn write_decomposition(td: &TreeDecomposition, writer: impl Write) -> std::io::Result<()> {
    let mut writer = BufWriter::new(writer);
    td.write_td(&mut writer)?;
    writer.flush()
}

#[cfg(test)]
#[path = "goatd/tests/mod.rs"]
mod tests;
