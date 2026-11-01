//! `goatd`: a PACE `.gr` graph in, a `.td` tree decomposition out, by any of
//! the library's routes.
//!
//! Usage and every option are in [`USAGE`]. Order-specific flags are rejected
//! when used with another construction.

use std::io::{BufWriter, Read, Write};
use std::process::exit;
use std::time::Duration;

use goatd::decomposition::refine_with_flowcutter;
use goatd::elimination::{Order, decompose as eliminate};
use goatd::embedding::MAX_DIM;
use goatd::flowcutter::{Budget, decompose as flowcutter};
use goatd::portfolio::{
    CandidateOutcome, CandidateTrace, DEFAULT_HEDGE_DIMS, Hedge, HedgeSeries, MAX_HEDGE_PASSES,
    Pass, PortfolioConfig, SamplingPatience, decompose as portfolio,
    decompose_traced as portfolio_traced,
};
use goatd::{Graph, TreeDecomposition, stop_flag};

const USAGE: &str = "\
usage: goatd <graph.gr | -> [options]

Reads a PACE .gr graph (- for stdin) and writes a PACE .td tree
decomposition to stdout, or to --out.

options:
  --out <file.td>       write the decomposition here instead of stdout
  --order <name>        which construction runs (default: portfolio)
                          minfill             greedy min-fill order
                          mindegree           greedy min-degree order
                          nested-dissection   multilevel nested dissection
                          flowcutter          the FlowCutter solver
                          merge-loop          independent decompositions built,
                                              improved and merged into one
                                              list of bags, searched for the
                                              narrowest tree it admits
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
                        deadline, flowcutter's run time, the portfolio's soft
                        deadline and the length of its trailing FlowCutter
                        slot, and the refinement's deadline. Each is its own
                        deadline, so --refine can spend it twice
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
                        asking for that many. The share is a share of a
                        deadline, so it needs --budget
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
  --recombine-up-to <n> portfolio only: after the candidates, search the bags
                        of all of them for the narrowest decomposition built out
                        of them, on graphs of at most n vertices, in place of
                        the built-in gate. The stage takes what its search is
                        estimated to cost off the end of the hard window, so it
                        needs --budget, and it costs a pass over the graph per
                        bag it pooled, which is what the gate bounds
  --no-recombine        portfolio only: return the best single candidate. The
                        stage runs only under --budget, so this needs one too
  --merge-up-to <n>     portfolio only: after the recombination stage, build a
                        decomposition independently of everything the run has,
                        improve it until it is no wider, and search the two
                        lists of bags together, on graphs of at most n
                        vertices, in place of the built-in gate. Like the
                        recombination stage it takes its share off the end of
                        the hard window, so it needs --budget
  --no-merge            portfolio only: merge no independent decomposition into
                        the run's answer. The stage runs only under --budget,
                        so this needs one too
  --local-merge-up-to <n>
                        portfolio only: after the merge loop, re-triangulate the
                        piece of the graph a bag of the answer and a bag of
                        another decomposition the run built leave between them,
                        and search the pooled bags together with the cliques
                        that come back, on graphs of at most n vertices, in
                        place of the built-in gate. Like the recombination
                        stage it takes its share off the end of the hard
                        window, so it needs --budget
  --no-local-merge      portfolio only: re-triangulate between no two of the
                        decompositions the run built. The stage runs only under
                        --budget, so this needs one too
  --no-hedge            portfolio only: run every candidate once, on uniform
                        weights, instead of repeating the candidates that read
                        weights on a ranking the portfolio computes itself
  --no-bipartite-lift   portfolio only: do not decompose the projection onto
                        one side of a bipartite graph and put the other side
                        back, which the budgeted portfolio otherwise tries
                        before its elimination orders. Only a budgeted run has
                        the stage, so this needs --budget
  --bipartite-lift-rate R
                        portfolio only: how much work the bipartite lift may do
                        per millisecond of the share it takes, in edges of the
                        input plus edges of the projection it would build
                        (default 150). Above the rate the stage does not run and
                        the time stays with the rest of the schedule. Only a
                        budgeted run has the stage, so this needs --budget
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
                        the band. Refused with --sample-band 0, which is the
                        exact minimum
  --sampling-patience <n>
                        portfolio only: the ordinary restarts stop once n of
                        them have run and the last one that improved the best
                        decomposition is in the first half of them, and the
                        trailing flowcutter candidate stops after half its
                        window without improving. n is capped at half the
                        restarts the schedule draws. Off by default, and it
                        costs width
  --no-sampling-patience
                        portfolio only: the default, run every ordinary
                        restart the count or the deadline allows and let the
                        trailing candidate run its window out
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
                        this the same schedule runs where --budget pays for
                        it: the portfolio times its first candidate, prices a
                        min-fill pass over the residual from that, and keeps
                        only its min-degree candidates unless the time left
                        holds two such passes
  --trace               portfolio only: write one line per candidate and one
                        for the winner to stderr as they complete
  --steps <n>           flowcutter only: a step budget in place of a clock,
                        for a run that repeats exactly
  --refine              re-cut the decomposition along FlowCutter separators
                        before writing it. Not with --order portfolio, whose
                        trailing candidate is FlowCutter already
  -h, --help            this text
";

/// Which construction `--order` named.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    MinFill,
    MinDegree,
    NestedDissection,
    FlowCutter,
    MergeLoop,
    Portfolio,
}

impl Method {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "minfill" => Method::MinFill,
            "mindegree" => Method::MinDegree,
            "nested-dissection" => Method::NestedDissection,
            "flowcutter" => Method::FlowCutter,
            "merge-loop" => Method::MergeLoop,
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
            Method::MergeLoop => "merge-loop",
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
    steps: Option<u64>,
    trace: bool,
    refine: bool,
    /// Built whatever the order is. Every portfolio flag is refused under the
    /// other constructions, so there this is the default nothing reads.
    portfolio: PortfolioConfig,
}

fn usage_error(msg: &str) -> ! {
    eprintln!("goatd: {msg}\n\n{USAGE}");
    exit(2)
}

fn fail(msg: &str) -> ! {
    eprintln!("goatd: {msg}");
    exit(1)
}

/// Read a comma-separated dimension list such as `1,2,3`: dimensions in
/// `1..=MAX_DIM`, none of them repeated, which holds the list to
/// [`MAX_HEDGE_PASSES`] entries.
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
    dims
}

#[derive(Default)]
struct RawArgs {
    input: Option<String>,
    out: Option<String>,
    order: Option<Method>,
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
    recombine_up_to: Option<u32>,
    no_recombine: bool,
    merge_up_to: Option<u32>,
    no_merge: bool,
    local_merge_up_to: Option<u32>,
    no_local_merge: bool,
    no_hedge: bool,
    no_bipartite_lift: bool,
    bipartite_lift_rate: Option<f64>,
    capped_restarts: bool,
    sample_band: Option<u64>,
    sample_band_alternate: bool,
    sampling_patience: Option<u64>,
    no_sampling_patience: bool,
    expensive_orders_up_to: Option<usize>,
    trace: bool,
    steps: Option<u64>,
    refine: bool,
}

fn arg_value(argv: &[String], i: &mut usize, flag: &str) -> String {
    *i += 1;
    argv.get(*i)
        .cloned()
        .unwrap_or_else(|| usage_error(&format!("{flag} needs a value")))
}

fn arg_number(argv: &[String], i: &mut usize, flag: &str) -> u64 {
    let v = arg_value(argv, i, flag);
    v.parse()
        .unwrap_or_else(|_| usage_error(&format!("{flag} wants a non-negative integer, got {v:?}")))
}

fn arg_vertices(argv: &[String], i: &mut usize, flag: &str) -> u32 {
    u32::try_from(arg_number(argv, i, flag)).unwrap_or_else(|_| {
        usage_error(&format!("{flag} wants a vertex count in 0..={}", u32::MAX))
    })
}

impl RawArgs {
    fn parse(argv: &[String]) -> Self {
        let mut raw = Self::default();
        let mut i = 0;
        while i < argv.len() {
            let arg = argv[i].as_str();
            match arg {
                "-h" | "--help" => {
                    print!("{USAGE}");
                    exit(0)
                }
                "--out" => raw.out = Some(arg_value(argv, &mut i, arg)),
                "--order" => {
                    let name = arg_value(argv, &mut i, arg);
                    raw.order = Some(
                        Method::parse(&name)
                            .unwrap_or_else(|| usage_error(&format!("unknown --order {name:?}"))),
                    );
                }
                "--seed" => raw.seed = Some(arg_number(argv, &mut i, arg)),
                "--ties" => {
                    let how = arg_value(argv, &mut i, arg);
                    if how != "sample" {
                        usage_error(&format!("--ties takes only `sample`, got {how:?}"));
                    }
                    raw.sample = true;
                }
                "--weights" => raw.weights = Some(arg_value(argv, &mut i, arg)),
                "--budget" => {
                    let milliseconds = arg_number(argv, &mut i, arg);
                    if milliseconds == 0 {
                        usage_error("--budget wants a positive millisecond count");
                    }
                    raw.budget = Some(Duration::from_millis(milliseconds));
                }
                "--hard-budget" => {
                    let milliseconds = arg_number(argv, &mut i, arg);
                    if milliseconds == 0 {
                        usage_error("--hard-budget wants a positive millisecond count");
                    }
                    raw.hard_budget = Some(Duration::from_millis(milliseconds));
                }
                "--steps" => {
                    let n = arg_number(argv, &mut i, arg);
                    if n == 0 {
                        usage_error("--steps wants a positive step count");
                    }
                    raw.steps = Some(n);
                }
                "--refine" => raw.refine = true,
                _ if raw.parse_portfolio_flag(argv, &mut i, arg) => {}
                _ if arg.starts_with('-') && arg != "-" => {
                    usage_error(&format!("unknown option {arg:?}"))
                }
                _ => {
                    if raw.input.replace(arg.to_string()).is_some() {
                        usage_error("more than one input graph given");
                    }
                }
            }
            i += 1;
        }
        raw
    }

    fn parse_portfolio_flag(&mut self, argv: &[String], i: &mut usize, arg: &str) -> bool {
        if self.parse_hedge_or_lift_flag(argv, i, arg) {
            return true;
        }
        match arg {
            "--mcs-up-to" => self.mcs_up_to = Some(arg_vertices(argv, i, arg)),
            "--no-mcs" => self.no_mcs = true,
            "--mcsm-up-to" => self.mcsm_up_to = Some(arg_vertices(argv, i, arg)),
            "--no-mcsm" => self.no_mcsm = true,
            "--drop-fill-up-to" => self.drop_fill_up_to = Some(arg_vertices(argv, i, arg)),
            "--no-drop-fill" => self.no_drop_fill = true,
            "--recombine-up-to" => self.recombine_up_to = Some(arg_vertices(argv, i, arg)),
            "--no-recombine" => self.no_recombine = true,
            "--merge-up-to" => self.merge_up_to = Some(arg_vertices(argv, i, arg)),
            "--no-merge" => self.no_merge = true,
            "--local-merge-up-to" => self.local_merge_up_to = Some(arg_vertices(argv, i, arg)),
            "--no-local-merge" => self.no_local_merge = true,
            "--capped-restarts" => self.capped_restarts = true,
            "--sample-band" => self.sample_band = Some(arg_number(argv, i, arg)),
            "--sample-band-alternate" => self.sample_band_alternate = true,
            "--sampling-patience" => {
                let restarts = arg_number(argv, i, arg);
                if restarts == 0 {
                    usage_error(
                        "--sampling-patience wants a positive restart floor; a floor of zero \
                         stops the restarts before the first one runs, and \
                         --no-sampling-patience is how the rule is turned off",
                    );
                }
                self.sampling_patience = Some(restarts);
            }
            "--no-sampling-patience" => self.no_sampling_patience = true,
            "--expensive-orders-up-to" => {
                self.expensive_orders_up_to = Some(
                    usize::try_from(arg_number(argv, i, arg)).unwrap_or_else(|_| {
                        usage_error(&format!("{arg} wants a vertex count in 0..={}", usize::MAX))
                    }),
                );
            }
            "--trace" => self.trace = true,
            _ => return false,
        }
        true
    }

    fn parse_hedge_or_lift_flag(&mut self, argv: &[String], i: &mut usize, arg: &str) -> bool {
        match arg {
            "--hedge-dims" => {
                let list = arg_value(argv, i, arg);
                self.hedge_dims = Some(parse_hedge_dims(&list));
            }
            "--hedge-random" => {
                let stages = arg_number(argv, i, arg);
                if stages == 0 || stages > MAX_HEDGE_PASSES as u64 {
                    usage_error(&format!(
                        "--hedge-random wants a stage count in 1..={MAX_HEDGE_PASSES}"
                    ));
                }
                self.hedge_random = Some(stages as usize);
            }
            "--hedge-reserve" => {
                let text = arg_value(argv, i, arg);
                let fraction: f64 = text.parse().unwrap_or_else(|_| {
                    usage_error(&format!(
                        "--hedge-reserve wants a fraction in 0 < f <= 1, such as 0.5, not {text:?}"
                    ))
                });
                if !fraction.is_finite() || fraction <= 0.0 || fraction > 1.0 {
                    usage_error("--hedge-reserve wants a fraction in 0 < f <= 1");
                }
                self.hedge_reserve = Some(fraction);
            }
            "--no-hedge" => self.no_hedge = true,
            "--no-bipartite-lift" => self.no_bipartite_lift = true,
            "--bipartite-lift-rate" => {
                let text = arg_value(argv, i, arg);
                let rate: f64 = text.parse().unwrap_or_else(|_| {
                    usage_error(&format!(
                        "--bipartite-lift-rate wants edges per millisecond above zero, \
                         such as 150, not {text:?}"
                    ))
                });
                if !rate.is_finite() || rate <= 0.0 {
                    usage_error("--bipartite-lift-rate wants edges per millisecond above zero");
                }
                self.bipartite_lift_rate = Some(rate);
            }
            _ => return false,
        }
        true
    }

    fn validate_order_and_hedge(&self, order: Method) {
        // Validate order-specific flags after the order and all flags are known.
        let greedy = matches!(order, Method::MinFill | Method::MinDegree);
        if self.sample {
            Self::require_order(order, "--ties sample", greedy, "minfill or mindegree");
        }
        if self.weights.is_some() {
            Self::require_order(order, "--weights", greedy, "minfill or mindegree");
            if !self.sample {
                usage_error("--weights requires --ties sample");
            }
        }
        if self.seed.is_some() {
            Self::require_order(
                order,
                "--seed",
                order != Method::FlowCutter,
                "minfill, mindegree, nested-dissection, merge-loop or portfolio",
            );
        }
        if self.steps.is_some() {
            Self::require_order(order, "--steps", order == Method::FlowCutter, "flowcutter");
            if self.budget.is_some() {
                usage_error("--steps and --budget both bound flowcutter; give one");
            }
        }
        self.validate_portfolio_only_flags(order);
        if let Some(hard) = self.hard_budget {
            let Some(soft) = self.budget else {
                usage_error("--hard-budget requires --budget");
            };
            if hard < soft {
                usage_error("--hard-budget must be at least --budget");
            }
        }
        // --hedge-dims and --hedge-random both say what the hedge's weighted stages
        // run, so each is refused beside anything else that says it.
        if let Some(flag) = self
            .hedge_dims
            .is_some()
            .then_some("--hedge-dims")
            .or(self.hedge_random.is_some().then_some("--hedge-random"))
        {
            if self.hedge_dims.is_some() && self.hedge_random.is_some() {
                usage_error(
                    "--hedge-dims and --hedge-random each say what the hedge's weighted stages \
                     run; give one",
                );
            }
            if self.no_hedge {
                usage_error(&format!(
                    "{flag} asks for weighted stages and --no-hedge runs none; give one"
                ));
            }
        }
    }

    fn require_order(order: Method, flag: &str, ok: bool, orders: &str) {
        if !ok {
            usage_error(&format!(
                "{flag} is not valid with --order {}; use --order {orders}",
                order.name()
            ));
        }
    }

    fn validate_portfolio_only_flags(&self, order: Method) {
        // Every flag here tunes something only the portfolio has.
        for (flag, given) in [
            ("--hard-budget", self.hard_budget.is_some()),
            ("--hedge-dims", self.hedge_dims.is_some()),
            ("--hedge-random", self.hedge_random.is_some()),
            ("--hedge-reserve", self.hedge_reserve.is_some()),
            ("--no-hedge", self.no_hedge),
            ("--no-bipartite-lift", self.no_bipartite_lift),
            ("--bipartite-lift-rate", self.bipartite_lift_rate.is_some()),
            ("--mcs-up-to", self.mcs_up_to.is_some()),
            ("--no-mcs", self.no_mcs),
            ("--mcsm-up-to", self.mcsm_up_to.is_some()),
            ("--no-mcsm", self.no_mcsm),
            ("--drop-fill-up-to", self.drop_fill_up_to.is_some()),
            ("--no-drop-fill", self.no_drop_fill),
            ("--recombine-up-to", self.recombine_up_to.is_some()),
            ("--no-recombine", self.no_recombine),
            ("--merge-up-to", self.merge_up_to.is_some()),
            ("--no-merge", self.no_merge),
            ("--local-merge-up-to", self.local_merge_up_to.is_some()),
            ("--no-local-merge", self.no_local_merge),
            ("--capped-restarts", self.capped_restarts),
            ("--sample-band", self.sample_band.is_some()),
            ("--sample-band-alternate", self.sample_band_alternate),
            ("--sampling-patience", self.sampling_patience.is_some()),
            ("--no-sampling-patience", self.no_sampling_patience),
            (
                "--expensive-orders-up-to",
                self.expensive_orders_up_to.is_some(),
            ),
            ("--trace", self.trace),
        ] {
            if given {
                Self::require_order(order, flag, order == Method::Portfolio, "portfolio");
            }
        }
    }

    fn validate_stage_gates_and_budgets(&self, order: Method) {
        self.validate_hedge_and_sampling_gates();
        self.validate_stage_gate_conflicts();
        // Four stages take their share off a window a run with no budget does not
        // have. The library refuses the equivalent config.
        if self.budget.is_none() {
            for (flag, given, stage, window) in [
                (
                    "--no-bipartite-lift",
                    self.no_bipartite_lift,
                    "bipartite lift",
                    "soft budget",
                ),
                (
                    "--bipartite-lift-rate",
                    self.bipartite_lift_rate.is_some(),
                    "bipartite lift",
                    "soft budget",
                ),
                (
                    "--recombine-up-to",
                    self.recombine_up_to.is_some(),
                    "recombination stage",
                    "hard window",
                ),
                (
                    "--no-recombine",
                    self.no_recombine,
                    "recombination stage",
                    "hard window",
                ),
                (
                    "--merge-up-to",
                    self.merge_up_to.is_some(),
                    "merge loop",
                    "hard window",
                ),
                ("--no-merge", self.no_merge, "merge loop", "hard window"),
                (
                    "--local-merge-up-to",
                    self.local_merge_up_to.is_some(),
                    "local re-triangulation stage",
                    "hard window",
                ),
                (
                    "--no-local-merge",
                    self.no_local_merge,
                    "local re-triangulation stage",
                    "hard window",
                ),
            ] {
                if given {
                    usage_error(&format!(
                        "{flag} requires --budget: the {stage} runs on a share of the {window}, \
                         and a run with no budget does not run it at all"
                    ));
                }
            }
        }
        // The portfolio's trailing candidate is FlowCutter, so the winner has
        // already been cut along FlowCutter separators when the run ends.
        if self.refine && order == Method::Portfolio {
            usage_error(
                "--refine is not valid with --order portfolio: the portfolio's own trailing \
                 FlowCutter candidate runs inside the budget, and there is nothing left for a \
                 refinement pass to do afterwards",
            );
        }
    }

    fn validate_hedge_and_sampling_gates(&self) {
        // The reserve decides how many stages follow the first, so it says nothing
        // where there is no second stage to refuse. With neither flag the series is
        // the portfolio's own, which has more than one stage.
        if self.hedge_reserve.is_some() {
            let stages = self
                .hedge_dims
                .as_ref()
                .map(Vec::len)
                .or(self.hedge_random)
                .unwrap_or(DEFAULT_HEDGE_DIMS.len());
            if stages < 2 {
                usage_error(
                    "--hedge-reserve decides how many weighted stages run after the first, and \
                     the first runs on any budget; give --hedge-dims or --hedge-random with two \
                     or more stages",
                );
            }
            if self.no_hedge {
                usage_error(
                    "--hedge-reserve says how much of the budget the hedge's weighted stages may \
                     spend and --no-hedge runs none; give one",
                );
            }
            if self.budget.is_none() {
                usage_error(
                    "--hedge-reserve requires --budget: the reserve is a fraction of the time the \
                     restarts would otherwise take, and a run with no budget leaves every stage \
                     unbounded",
                );
            }
        }
        // The rate decides whether the bipartite lift runs, so it says nothing
        // where the stage is off.
        if self.bipartite_lift_rate.is_some() && self.no_bipartite_lift {
            usage_error(
                "--bipartite-lift-rate says how much work the bipartite lift may do, and \
                 --no-bipartite-lift turns it off",
            );
        }
        // The count is what stops the restarts of a run with no deadline, so the
        // flag decides nothing there.
        if self.capped_restarts && self.budget.is_none() {
            usage_error(
                "--capped-restarts requires --budget: with no deadline the restarts stop at \
                 their count anyway",
            );
        }
        // Alternating with a band of zero is the exact minimum on every restart,
        // so the flag would decide nothing. Without --sample-band the library's
        // own band applies, which is not zero, so only the flag set to zero is
        // the inert pair.
        if self.sample_band_alternate && self.sample_band == Some(0) {
            usage_error(
                "--sample-band-alternate alternates between the exact minimum and the band, \
                 and --sample-band 0 is the exact minimum",
            );
        }
        if self.sampling_patience.is_some() && self.no_sampling_patience {
            usage_error(
                "--sampling-patience and --no-sampling-patience both say when the restarts \
                 stop; give one",
            );
        }
    }

    fn validate_stage_gate_conflicts(&self) {
        // Each pair says whether one construction runs and how large a graph it
        // runs on, so giving both leaves one of them with nothing to decide.
        for (gate, gate_given, off, off_given, stage) in [
            (
                "--mcs-up-to",
                self.mcs_up_to.is_some(),
                "--no-mcs",
                self.no_mcs,
                "maximum cardinality search candidate",
            ),
            (
                "--mcsm-up-to",
                self.mcsm_up_to.is_some(),
                "--no-mcsm",
                self.no_mcsm,
                "MCS-M candidate",
            ),
            (
                "--drop-fill-up-to",
                self.drop_fill_up_to.is_some(),
                "--no-drop-fill",
                self.no_drop_fill,
                "fill-dropping pass",
            ),
            (
                "--recombine-up-to",
                self.recombine_up_to.is_some(),
                "--no-recombine",
                self.no_recombine,
                "recombination stage",
            ),
            (
                "--merge-up-to",
                self.merge_up_to.is_some(),
                "--no-merge",
                self.no_merge,
                "merge loop",
            ),
            (
                "--local-merge-up-to",
                self.local_merge_up_to.is_some(),
                "--no-local-merge",
                self.no_local_merge,
                "local re-triangulation stage",
            ),
        ] {
            if gate_given && off_given {
                usage_error(&format!(
                    "{gate} gates the {stage} and {off} runs none; give one"
                ));
            }
        }
    }

    fn build_portfolio_config(&self) -> PortfolioConfig {
        let mut config = self.budget.map_or_else(
            PortfolioConfig::standard,
            PortfolioConfig::standard_with_budget,
        );
        if let Some(hard_budget) = self.hard_budget {
            config = config.with_hard_budget(hard_budget);
        }
        if self.no_hedge {
            config = config.with_hedge(Hedge::Off);
        }
        if let Some(rate) = self.bipartite_lift_rate {
            config = config.with_bipartite_lift_rate(rate);
        }
        if self.no_bipartite_lift {
            config = config.without_bipartite_lift();
        }
        if let Some(dims) = &self.hedge_dims {
            config = config.with_hedge(Hedge::Passes(HedgeSeries::eccentricity_dims(dims)));
        }
        if let Some(stages) = self.hedge_random {
            config = config.with_hedge(Hedge::Passes(HedgeSeries::random(stages)));
        }
        if let Some(fraction) = self.hedge_reserve {
            config = config.with_hedge_reserve(fraction);
        }
        config = self.apply_stage_gate_config(config);
        if self.capped_restarts {
            config = config.with_restarts_to_deadline(false);
        }
        if let Some(band) = self.sample_band {
            config = config.with_sample_band(band);
        }
        if self.sample_band_alternate {
            config = config.with_sample_band_alternate(true);
        }
        if let Some(min_restarts) = self.sampling_patience {
            config = config.with_sampling_patience(SamplingPatience::Halving { min_restarts });
        }
        if self.no_sampling_patience {
            config = config.with_sampling_patience(SamplingPatience::Off);
        }
        if let Some(count) = self.expensive_orders_up_to {
            config = config.with_expensive_orders_up_to(count);
        }
        config
    }

    fn apply_stage_gate_config(&self, mut config: PortfolioConfig) -> PortfolioConfig {
        if let Some(count) = self.mcs_up_to {
            config = config.with_maximum_cardinality(count);
        }
        if self.no_mcs {
            config = config.without_maximum_cardinality();
        }
        if let Some(count) = self.mcsm_up_to {
            config = config.with_minimal_triangulation(count);
        }
        if self.no_mcsm {
            config = config.without_minimal_triangulation();
        }
        if let Some(count) = self.drop_fill_up_to {
            config = config.with_triangulation_refinement(count);
        }
        if self.no_drop_fill {
            config = config.without_triangulation_refinement();
        }
        if let Some(count) = self.recombine_up_to {
            config = config.with_recombination(count);
        }
        if self.no_recombine {
            config = config.without_recombination();
        }
        if let Some(count) = self.merge_up_to {
            config = config.with_merge_loop(count);
        }
        if self.no_merge {
            config = config.without_merge_loop();
        }
        if let Some(count) = self.local_merge_up_to {
            config = config.with_local_merge(count);
        }
        if self.no_local_merge {
            config = config.without_local_merge();
        }
        config
    }
}

fn parse_args(argv: &[String]) -> Args {
    let raw = RawArgs::parse(argv);
    let Some(input) = raw.input.clone() else {
        usage_error("no input graph given");
    };
    let order = raw.order.unwrap_or(Method::Portfolio);
    raw.validate_order_and_hedge(order);
    raw.validate_stage_gates_and_budgets(order);
    let portfolio = raw.build_portfolio_config();

    Args {
        input,
        out: raw.out,
        order,
        seed: raw.seed,
        sample: raw.sample,
        weights: raw.weights,
        budget: raw.budget,
        steps: raw.steps,
        trace: raw.trace,
        refine: raw.refine,
        portfolio,
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
    let result = match args.order {
        Method::MinFill | Method::MinDegree => {
            let weights = args.sample.then(|| match &args.weights {
                Some(path) => read_weights(path, graph.num_vertices()),
                None => vec![1; graph.num_vertices() as usize],
            });
            let config = match (args.order, weights.as_deref()) {
                (Method::MinFill, None) => Order::MinFill,
                (Method::MinFill, Some(weights)) => Order::MinFillSampled { weights },
                (_, None) => Order::MinDegree,
                (_, Some(weights)) => Order::MinDegreeSampled { weights },
            };
            eliminate(graph, config, seed, budget)
        }
        Method::NestedDissection => eliminate(graph, Order::NestedDissection, seed, budget),
        Method::FlowCutter => flowcutter(graph, Budget::standalone(budget, args.steps)),
        Method::MergeLoop => goatd::decomposition::decompose_by_merging(graph, seed, budget),
        // A traced run computes each candidate's shape numbers, which is a pass
        // over its bags. Without --trace there is nowhere to report them, so
        // the untraced run does not ask for them.
        Method::Portfolio if !args.trace => {
            let weights = vec![1; graph.num_vertices() as usize];
            portfolio(graph, &weights, seed, args.portfolio)
        }
        Method::Portfolio => {
            let weights = vec![1; graph.num_vertices() as usize];
            let mut winner = None;
            let mut report = |candidate: CandidateTrace| {
                print_candidate(&candidate);
                if let CandidateOutcome::Produced { best: true, .. } = candidate.outcome {
                    winner = Some((candidate.stage, candidate.seed));
                }
            };
            let result = portfolio_traced(graph, &weights, seed, args.portfolio, &mut report);
            if result.is_ok()
                && let Some((stage, seed)) = winner
            {
                eprintln!("c trace winner candidate={stage} seed={seed}");
            }
            result
        }
    };
    result.unwrap_or_else(|error| fail(&error.to_string()))
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
            shape,
            ..
        } => {
            line.push_str(&format!(" width={width} bags={total_bag_size}"));
            if let Some(shape) = shape {
                line.push_str(&format!(
                    " bag-mass={:.2} max-separator={}",
                    shape.bag_mass, shape.max_separator
                ));
            }
        }
        CandidateOutcome::SamplingStopped {
            restarts,
            last_improvement,
            left,
        } => push_sampling_stopped_trace(&mut line, restarts, last_improvement, left),
        CandidateOutcome::TailBounded {
            window,
            patience,
            spent,
        } => line.push_str(&format!(
            " outcome=tail-bounded window-ms={} patience-ms={} spent-ms={}",
            window.as_millis(),
            patience.as_millis(),
            spent.as_millis()
        )),
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

fn push_sampling_stopped_trace(
    line: &mut String,
    restarts: u64,
    last_improvement: Option<u64>,
    left: Option<Duration>,
) {
    line.push_str(&format!(" outcome=sampling-stopped restarts={restarts}"));
    match last_improvement {
        Some(index) => line.push_str(&format!(" last-improvement={index}")),
        None => line.push_str(" last-improvement=none"),
    }
    match left {
        Some(duration) => {
            line.push_str(&format!(" left-ms={}", duration.as_millis()));
        }
        None => line.push_str(" left-ms=none"),
    }
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

    let graph = Graph::from_gr(&read_input(&args.input))
        .unwrap_or_else(|e| fail(&format!("{}: {e}", args.input)));

    let mut td = construct(&args, &graph);
    if args.refine {
        // The budget is a deadline per phase, as it is for every other phase
        // the usage text lists. Giving the pass what the construction left of
        // one shared budget made it a no-op on every graph the construction
        // did not finish early, which is the graph it is wanted on.
        td = refine_with_flowcutter(td, &graph, args.budget)
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
