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
use goatd::embedding::MAX_DIM;
use goatd::flowcutter::{Budget, decompose as flowcutter};
use goatd::portfolio::{
    CandidateOutcome, CandidateTrace, DEFAULT_HEDGE_DIMS, Hedge, HedgeSeries, MAX_HEDGE_PASSES,
    Pass, PortfolioConfig, decompose_traced as portfolio,
};
use goatd::{Graph, TreeDecomposition, stop_flag};

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
  --hedge-dims <list>   portfolio only: run the hedge's weighted stage once per
                        dimension of a comma-separated list, in the order
                        given, each on a ranking from its own placement, such
                        as 1,2,3, in place of the default 3,1,2,4,8,5,6,7.
                        Dimensions run 1 to 8 and no dimension repeats. The
                        restarts stay plain and the incumbent width bounds the
                        stages that follow. Not with --hedge-random or
                        --no-hedge
  --hedge-random <k>    portfolio only: k weighted stages on random weights
                        instead, the control for --hedge-dims. Each stage draws
                        from a seed of its own, --seed + 6151 + i * 104729 for
                        stage i. Same restrictions as --hedge-dims
  --hedge-reserve <f>   portfolio only: the share of the budget left after the
                        plain pass the hedge's weighted stages may spend
                        between them, 0 < f <= 1 (default 0.5). The rest is
                        kept for the ordinary restarts: a stage costs about
                        what the plain pass cost, and the portfolio runs one
                        more only while that fits. The first stage runs on any
                        budget, so this needs a series of two or more stages —
                        the default one, or --hedge-dims or --hedge-random
                        asking for that many
  --mcs-up-to <n>       portfolio only: run the maximum cardinality search
                        candidate while the preprocessed residual has at most n
                        vertices, in place of the built-in gate. The search
                        numbers the residual by numbered-neighbour count and the
                        candidate eliminates along that numbering reversed; it
                        is one deterministic candidate, and it costs a scan of
                        the unnumbered vertices per vertex, which is what the
                        gate bounds
  --no-mcs              portfolio only: run no maximum cardinality search
                        candidate
  --mcsm-up-to <n>      portfolio only: run the MCS-M candidate while the
                        preprocessed residual has at most n vertices, in place
                        of the built-in gate. MCS-M eliminates along a minimal
                        triangulation of the residual; it is one deterministic
                        candidate, and it costs a traversal of the residual per
                        vertex, which is what the gate bounds
  --no-mcsm             portfolio only: run no MCS-M candidate
  --drop-fill-up-to <n> portfolio only: rebuild the winner on a minimal
                        triangulation of the same graph, dropping the fill edges
                        its bags do not need, on graphs of at most n vertices,
                        in place of the built-in gate. The pass never widens,
                        and it runs only while what it is projected to cost
                        fits in what is left of the hard budget, so a wide n
                        costs memory rather than time
  --no-drop-fill        portfolio only: leave the winner's fill edges alone
  --no-hedge            portfolio only: run every candidate once, on uniform
                        weights, instead of repeating the candidates that read
                        weights on a ranking the portfolio computes itself
  --capped-restarts     portfolio only: stop the ordinary restarts at their
                        count instead of drawing seeds until the restart
                        deadline, which is the hard cutoff less the reserve
                        kept for the trailing FlowCutter candidate. Needs
                        --budget: with no deadline the count is what stops
                        them anyway
  --sample-band <eps>   portfolio only: the ordinary restarts draw from every
                        vertex whose elimination adds at most eps fill edges
                        more than the best. 0 draws only from the vertices
                        tied at the minimum; the library's own band is the
                        default. The other candidates keep their exact
                        minimum
  --sample-band-alternate
                        portfolio only: alternate the ordinary restarts
                        between the exact minimum and --sample-band, an even
                        restart drawing from the minimum and an odd one from
                        the band. Needs --sample-band above 0
  --expensive-orders-up-to <n>
                        portfolio only: the largest residual, in vertices left
                        after preprocessing, that still runs min-fill (default
                        300000). At or below 10000 vertices the whole schedule
                        runs. Above 10000 and at or below this, min-fill runs
                        but stops at half the time --budget has left when it
                        starts, so the restarts keep a share of it; nested
                        dissection, the diverse pass and the hedge stay off;
                        and the restarts follow min-fill if the initial
                        min-fill finished and min-degree if it did not. Above
                        this the portfolio keeps only its min-degree
                        candidates
  --trace               portfolio only: write one line per candidate and one
                        for the winner to stderr as they complete
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
    hedge_dims: Option<Vec<usize>>,
    hedge_random: Option<usize>,
    hedge_reserve: Option<f64>,
    mcs_up_to: Option<u32>,
    no_mcs: bool,
    mcsm_up_to: Option<u32>,
    no_mcsm: bool,
    drop_fill_up_to: Option<u32>,
    no_drop_fill: bool,
    no_hedge: bool,
    capped_restarts: bool,
    sample_band: Option<u64>,
    sample_band_alternate: bool,
    expensive_orders_up_to: Option<usize>,
    trace: bool,
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

/// Read a comma-separated dimension list such as `1,2,3`: one to
/// [`MAX_HEDGE_PASSES`] dimensions in `1..=MAX_DIM`, none of them repeated.
fn parse_hedge_dims(text: &str) -> Vec<usize> {
    let mut dims = Vec::new();
    for field in text.split(',') {
        let dim: usize = field.trim().parse().unwrap_or_else(|_| {
            usage_error(&format!(
                "--hedge-dims wants dimensions in 1..={MAX_DIM} separated by commas, not {text:?}"
            ))
        });
        if dim == 0 || dim > MAX_DIM {
            usage_error(&format!("--hedge-dims wants dimensions in 1..={MAX_DIM}"));
        }
        if dims.contains(&dim) {
            usage_error(&format!(
                "--hedge-dims runs dimension {dim} twice, which is the same stage twice"
            ));
        }
        dims.push(dim);
    }
    if dims.is_empty() || dims.len() > MAX_HEDGE_PASSES {
        usage_error(&format!(
            "--hedge-dims wants one to {MAX_HEDGE_PASSES} dimensions"
        ));
    }
    dims
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
    let mut hedge_dims: Option<Vec<usize>> = None;
    let mut hedge_random = None;
    let mut hedge_reserve = None;
    let mut mcs_up_to = None;
    let mut no_mcs = false;
    let mut mcsm_up_to = None;
    let mut no_mcsm = false;
    let mut drop_fill_up_to = None;
    let mut no_drop_fill = false;
    let mut no_hedge = false;
    let mut capped_restarts = false;
    let mut sample_band = None;
    let mut sample_band_alternate = false;
    let mut expensive_orders_up_to = None;
    let mut trace = false;
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
            "--hedge-dims" => {
                let list = value(&mut i, arg);
                hedge_dims = Some(parse_hedge_dims(&list));
            }
            "--hedge-random" => {
                let stages = number(&mut i, arg);
                if stages == 0 || stages > MAX_HEDGE_PASSES as u64 {
                    usage_error(&format!(
                        "--hedge-random wants a stage count in 1..={MAX_HEDGE_PASSES}"
                    ));
                }
                hedge_random = Some(stages as usize);
            }
            "--hedge-reserve" => {
                let text = value(&mut i, arg);
                let fraction: f64 = text.parse().unwrap_or_else(|_| {
                    usage_error(&format!(
                        "--hedge-reserve wants a fraction in 0 < f <= 1, such as 0.5, not {text:?}"
                    ))
                });
                if !fraction.is_finite() || fraction <= 0.0 || fraction > 1.0 {
                    usage_error("--hedge-reserve wants a fraction in 0 < f <= 1");
                }
                hedge_reserve = Some(fraction);
            }
            "--mcs-up-to" => {
                let vertices = number(&mut i, arg);
                if vertices > u64::from(u32::MAX) {
                    usage_error(&format!(
                        "--mcs-up-to wants a vertex count in 0..={}",
                        u32::MAX
                    ));
                }
                mcs_up_to = Some(vertices as u32);
            }
            "--no-mcs" => no_mcs = true,
            "--mcsm-up-to" => {
                let vertices = number(&mut i, arg);
                if vertices > u64::from(u32::MAX) {
                    usage_error(&format!(
                        "--mcsm-up-to wants a vertex count in 0..={}",
                        u32::MAX
                    ));
                }
                mcsm_up_to = Some(vertices as u32);
            }
            "--no-mcsm" => no_mcsm = true,
            "--drop-fill-up-to" => {
                let vertices = number(&mut i, arg);
                if vertices > u64::from(u32::MAX) {
                    usage_error(&format!(
                        "--drop-fill-up-to wants a vertex count in 0..={}",
                        u32::MAX
                    ));
                }
                drop_fill_up_to = Some(vertices as u32);
            }
            "--no-drop-fill" => no_drop_fill = true,
            "--no-hedge" => no_hedge = true,
            "--capped-restarts" => capped_restarts = true,
            "--sample-band" => sample_band = Some(number(&mut i, arg)),
            "--sample-band-alternate" => sample_band_alternate = true,
            "--expensive-orders-up-to" => {
                let vertices = number(&mut i, arg);
                expensive_orders_up_to = Some(usize::try_from(vertices).unwrap_or(usize::MAX));
            }
            "--trace" => trace = true,
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
    // --hedge-dims and --hedge-random both say what the hedge's weighted stages
    // run, so each is refused beside anything else that says it.
    if let Some(flag) = hedge_dims
        .is_some()
        .then_some("--hedge-dims")
        .or(hedge_random.is_some().then_some("--hedge-random"))
    {
        needs(flag, order == Method::Portfolio, "portfolio");
        if hedge_dims.is_some() && hedge_random.is_some() {
            usage_error(
                "--hedge-dims and --hedge-random each say what the hedge's weighted stages \
                 run; give one",
            );
        }
        if no_hedge {
            usage_error(&format!(
                "{flag} asks for weighted stages and --no-hedge runs none; give one"
            ));
        }
    }
    // The reserve decides how many stages follow the first, so it says nothing
    // where there is no second stage to refuse. With neither flag the series is
    // the portfolio's own, which has more than one stage.
    if hedge_reserve.is_some() {
        needs("--hedge-reserve", order == Method::Portfolio, "portfolio");
        let stages = hedge_dims
            .as_ref()
            .map(Vec::len)
            .or(hedge_random)
            .unwrap_or(DEFAULT_HEDGE_DIMS.len());
        if stages < 2 {
            usage_error(
                "--hedge-reserve decides how many weighted stages run after the first, and \
                 the first runs on any budget; give --hedge-dims or --hedge-random with two \
                 or more stages",
            );
        }
    }
    if no_hedge {
        needs("--no-hedge", order == Method::Portfolio, "portfolio");
    }
    // Each pair says whether one construction runs and how large a graph it
    // runs on, so giving both leaves one of them with nothing to decide.
    if mcs_up_to.is_some() {
        needs("--mcs-up-to", order == Method::Portfolio, "portfolio");
        if no_mcs {
            usage_error(
                "--mcs-up-to gates the maximum cardinality search candidate and --no-mcs runs \
                 none; give one",
            );
        }
    }
    if no_mcs {
        needs("--no-mcs", order == Method::Portfolio, "portfolio");
    }
    if mcsm_up_to.is_some() {
        needs("--mcsm-up-to", order == Method::Portfolio, "portfolio");
        if no_mcsm {
            usage_error("--mcsm-up-to gates the MCS-M candidate and --no-mcsm runs none; give one");
        }
    }
    if no_mcsm {
        needs("--no-mcsm", order == Method::Portfolio, "portfolio");
    }
    if drop_fill_up_to.is_some() {
        needs("--drop-fill-up-to", order == Method::Portfolio, "portfolio");
        if no_drop_fill {
            usage_error(
                "--drop-fill-up-to gates the fill-dropping pass and --no-drop-fill runs none; \
                 give one",
            );
        }
    }
    if no_drop_fill {
        needs("--no-drop-fill", order == Method::Portfolio, "portfolio");
    }
    // The count is what stops the restarts of a run with no deadline, so the
    // flag decides nothing there.
    if capped_restarts {
        needs("--capped-restarts", order == Method::Portfolio, "portfolio");
        if budget.is_none() {
            usage_error(
                "--capped-restarts requires --budget: with no deadline the restarts stop at \
                 their count anyway",
            );
        }
    }
    if sample_band.is_some() {
        needs("--sample-band", order == Method::Portfolio, "portfolio");
    }
    // Alternating with a band of zero is the exact minimum on every restart,
    // so the flag would decide nothing.
    if sample_band_alternate {
        needs(
            "--sample-band-alternate",
            order == Method::Portfolio,
            "portfolio",
        );
        if sample_band.unwrap_or(0) == 0 {
            usage_error(
                "--sample-band-alternate requires --sample-band above 0: it alternates \
                 between the exact minimum and the band",
            );
        }
    }
    if expensive_orders_up_to.is_some() {
        needs(
            "--expensive-orders-up-to",
            order == Method::Portfolio,
            "portfolio",
        );
    }
    if trace {
        needs("--trace", order == Method::Portfolio, "portfolio");
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
        hedge_dims,
        hedge_random,
        hedge_reserve,
        mcs_up_to,
        no_mcs,
        mcsm_up_to,
        no_mcsm,
        drop_fill_up_to,
        no_drop_fill,
        no_hedge,
        capped_restarts,
        sample_band,
        sample_band_alternate,
        expensive_orders_up_to,
        trace,
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

fn construct(args: &Args, graph: &Graph) -> TreeDecomposition {
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
            let mut config = budget.map_or_else(
                PortfolioConfig::standard,
                PortfolioConfig::standard_with_budget,
            );
            if let Some(hard_budget) = args.hard_budget {
                config = config.with_hard_budget(hard_budget);
            }
            if args.no_hedge {
                config = config.with_hedge(Hedge::Off);
            }
            if let Some(dims) = &args.hedge_dims {
                config = config.with_hedge(Hedge::Passes(HedgeSeries::eccentricity_dims(dims)));
            }
            if let Some(stages) = args.hedge_random {
                config = config.with_hedge(Hedge::Passes(HedgeSeries::random(stages)));
            }
            if let Some(fraction) = args.hedge_reserve {
                config = config.with_hedge_reserve(fraction);
            }
            if let Some(vertices) = args.mcs_up_to {
                config = config.with_maximum_cardinality(vertices);
            }
            if args.no_mcs {
                config = config.without_maximum_cardinality();
            }
            if let Some(vertices) = args.mcsm_up_to {
                config = config.with_minimal_triangulation(vertices);
            }
            if args.no_mcsm {
                config = config.without_minimal_triangulation();
            }
            if let Some(vertices) = args.drop_fill_up_to {
                config = config.with_triangulation_refinement(vertices);
            }
            if args.no_drop_fill {
                config = config.without_triangulation_refinement();
            }
            if args.capped_restarts {
                config = config.with_restarts_to_deadline(false);
            }
            if let Some(band) = args.sample_band {
                config = config.with_sample_band(band);
            }
            if args.sample_band_alternate {
                config = config.with_sample_band_alternate(true);
            }
            if let Some(vertices) = args.expensive_orders_up_to {
                config = config.with_expensive_orders_up_to(vertices);
            }
            let mut winner = None;
            let mut report = |candidate: CandidateTrace| {
                if args.trace {
                    print_candidate(&candidate);
                }
                if let CandidateOutcome::Produced { best: true, .. } = candidate.outcome {
                    winner = Some((candidate.stage, candidate.seed));
                }
            };
            let td = portfolio(graph, &weights, seed, config, &mut report)
                .unwrap_or_else(|error| fail(&error.to_string()));
            if args.trace
                && let Some((stage, seed)) = winner
            {
                eprintln!("c trace winner candidate={stage} seed={seed}");
            }
            td
        }
    }
}

/// `c trace candidate=…`, on stderr so it does not touch the decomposition.
fn print_candidate(candidate: &CandidateTrace) {
    let mut line = format!(
        "c trace candidate={} seed={}",
        candidate.stage, candidate.seed
    );
    match candidate.pass {
        Pass::Only => {}
        Pass::Plain => line.push_str(" pass=plain"),
        // A hedge of one weighting reports every modified candidate as stage 0,
        // so only a series says which stage a candidate came from.
        Pass::Modified { index: 0 } => line.push_str(" pass=modified"),
        Pass::Modified { index } => line.push_str(&format!(" pass=modified:{index}")),
    }
    match candidate.outcome {
        CandidateOutcome::Produced {
            width,
            total_bag_size,
            ..
        } => line.push_str(&format!(" width={width} bags={total_bag_size}")),
        CandidateOutcome::WidthAborted => line.push_str(" outcome=aborted"),
        CandidateOutcome::DeadlineReached => line.push_str(" outcome=deadline"),
        CandidateOutcome::NotStarted => line.push_str(" outcome=not-started"),
        CandidateOutcome::StageSkipped {
            projected,
            spent,
            allowance,
        } => line.push_str(&format!(
            " outcome=skipped projected={}ms spent={}ms allowance={}ms",
            projected.as_millis(),
            spent.as_millis(),
            allowance.as_millis()
        )),
    }
    eprintln!("{line} ms={}", candidate.elapsed.as_millis());
}

/// Ask the library to stop. The handler stores one byte and does nothing else,
/// so it is safe to run from a signal.
#[cfg(unix)]
extern "C" fn on_terminate(_signal: std::os::raw::c_int) {
    stop_flag().store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Answer `SIGTERM` by stopping the search rather than by dying, so a caller
/// that runs the tool under a wall clock still gets the decomposition found so
/// far. Anything the handler cannot reach, such as reading the graph, keeps the
/// default behaviour of ending the process.
#[cfg(unix)]
fn install_terminate_handler() {
    // SAFETY: `action` is fully initialized below, and the handler only stores
    // into an atomic. `sigaction` is given a valid pointer and a null old-action
    // pointer.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_terminate as *const () as usize;
        libc::sigemptyset(&raw mut action.sa_mask);
        action.sa_flags = libc::SA_RESTART;
        libc::sigaction(libc::SIGTERM, &raw const action, std::ptr::null_mut());
    }
}

#[cfg(not(unix))]
fn install_terminate_handler() {}

fn main() {
    install_terminate_handler();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);

    let start = Instant::now();
    let graph = Graph::from_gr(&read_input(&args.input))
        .unwrap_or_else(|e| fail(&format!("{}: {e}", args.input)));

    let mut td = construct(&args, &graph);
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
