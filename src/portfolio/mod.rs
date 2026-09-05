//! Run several decomposition constructions and keep the candidates they
//! produce.
//!
//! The candidates themselves come from the elimination engine; the single-order
//! construction in [`crate::elimination::decompose`] does not go through
//! here.

use std::cell::OnceCell;
use std::time::{Duration, Instant};

mod candidates;
mod config;
mod trace;

#[cfg(test)]
mod tests;

use crate::deadline::{expired, remaining};
use crate::decomposition;
use crate::elimination::Order;
use crate::elimination::engine;
use crate::elimination::execution::ElimStop;
use crate::embedding::{self, Embedding};
use crate::flowcutter::{Budget, decompose as flowcutter_decompose};
use crate::{Error, Graph, TreeDecomposition};
use candidates::{CandidateSet, ScheduleStop};
use config::{FLOWCUTTER_RESERVE, MIN_FLOWCUTTER_CANDIDATE_MS};

pub use config::{
    DEFAULT_HEDGE_DIMS, Hedge, HedgeSeries, HedgeWeights, MAX_DIVERSE_SAMPLING_RUNS,
    MAX_HEDGE_PASSES, PortfolioConfig,
};
pub use trace::{CandidateOutcome, CandidateTrace, Pass, Stage};

/// Exit early if FlowCutter hasn't improved treewidth for this long. Caps
/// per-graph overhead where FlowCutter converges fast.
const FLOWCUTTER_CANDIDATE_PATIENCE: Duration = Duration::from_millis(500);
const FLOWCUTTER_CANDIDATE_ITERATIONS: u32 = 50;
const SAMPLE_SEED_OFFSET: u64 = 100;
const SAMPLE_SEED_STRIDE: u64 = 7919;
/// Separates a random hedge weighting from the run's other draws.
const HEDGE_RANDOM_SEED_OFFSET: u64 = 6151;
/// Separates one random hedge weighting from the next.
const HEDGE_RANDOM_SEED_STRIDE: u64 = 104_729;
pub(crate) const SECOND_CANDIDATE_SEED_OFFSET: u64 = 42;

/// Restarts kept back at the end of the hard window for the trailing FlowCutter
/// candidate to stop in and hand its result back. See `flowcutter_candidate`.
const RESERVE_RESTARTS: u32 = 2;

fn is_min_degree_variant(order: Order<'_>) -> bool {
    matches!(order, Order::MinDegree | Order::MinDegreeSampled { .. })
}

fn is_min_fill_variant(order: Order<'_>) -> bool {
    matches!(order, Order::MinFill | Order::MinFillSampled { .. })
}

/// How the residual left after preprocessing stands against the size rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Residual {
    /// At or below [`config::MAX_RESIDUAL_FOR_FULL_SCHEDULE`], so the whole
    /// schedule runs: every initial order, the diverse pass, the hedge, and
    /// sampled min-fill restarts.
    Ordinary,
    /// Above [`config::MAX_RESIDUAL_FOR_FULL_SCHEDULE`] and at or below the
    /// limit from [`PortfolioConfig::with_expensive_orders_up_to`]. The
    /// expensive initial orders run, each on half the time the restart deadline
    /// has left rather than on the whole window; nested dissection, the diverse
    /// pass and the hedge do not; the initial candidates and the restarts both
    /// run to the restart deadline; and the restarts follow whichever of
    /// min-fill and min-degree produced a decomposition.
    Admitted,
    /// Past that limit: min-degree candidates and sampled min-degree restarts,
    /// nothing else.
    Large,
}

impl Residual {
    /// Where `active` vertices leave a run whose limit is `limit`.
    fn classify(active: usize, limit: usize) -> Self {
        if active > limit {
            Residual::Large
        } else if active > config::MAX_RESIDUAL_FOR_FULL_SCHEDULE {
            Residual::Admitted
        } else {
            Residual::Ordinary
        }
    }
}

fn sample_seed(base_seed: u64, sample_index: u64) -> u64 {
    base_seed.wrapping_add(SAMPLE_SEED_OFFSET + sample_index.wrapping_mul(SAMPLE_SEED_STRIDE))
}

/// The seed the `stream`-th random hedge weighting is drawn from:
/// `base_seed + 6151 + stream * 104729`. The offset is not a multiple of the
/// ordinary sampling stride, so no weighting lands on a restart's seed, and the
/// weights come off a stream of their own inside
/// [`random_weights`](crate::embedding::random_weights), so the elimination
/// draws are untouched.
fn hedge_random_seed(base_seed: u64, stream: u64) -> u64 {
    base_seed
        .wrapping_add(HEDGE_RANDOM_SEED_OFFSET)
        .wrapping_add(stream.wrapping_mul(HEDGE_RANDOM_SEED_STRIDE))
}

/// The sampling weights one weighted stage runs on. They are derived the first
/// time a candidate of that stage asks for them, which is after the plain
/// diverse pass, so a run that never reaches the stage never pays for them.
#[derive(Clone, Copy)]
enum ModifiedWeights<'a> {
    /// The vertices placed and ranked by eccentricity, most peripheral first.
    Ranked {
        cell: &'a OnceCell<Vec<u32>>,
        graph: &'a Graph,
        dim: usize,
        rounds: usize,
        seed: u64,
        deadline: Option<Instant>,
    },
    /// Uniform weights from `seed`, drawn on first use into `cell`.
    Random {
        cell: &'a OnceCell<Vec<u32>>,
        count: usize,
        seed: u64,
    },
}

impl<'a> ModifiedWeights<'a> {
    fn get(self) -> &'a [u32] {
        match self {
            ModifiedWeights::Ranked {
                cell,
                graph,
                dim,
                rounds,
                seed,
                deadline,
            } => cell.get_or_init(|| {
                Embedding::compute(
                    graph,
                    dim,
                    seed,
                    rounds,
                    embedding::DEFAULT_PATIENCE,
                    embedding::DEFAULT_TOLERANCE,
                    &mut || expired(deadline),
                )
                .rank_weights(true)
            }),
            ModifiedWeights::Random { cell, count, seed } => {
                cell.get_or_init(|| embedding::random_weights(count, seed))
            }
        }
    }
}

/// How wide a tie set the ordinary restarts draw from, and whether every
/// second restart drops back to the exact minimum.
#[derive(Clone, Copy, Default)]
struct SampleBand {
    /// How far above the minimum fill the tie set reaches, in fill edges.
    width: u64,
    /// Alternate between the exact minimum and `width`, restart by restart.
    alternate: bool,
}

impl SampleBand {
    /// The band the `index`-th ordinary restart draws from. Alternating, the
    /// even restarts draw from the exact minimum, which is the candidate a
    /// portfolio without a band runs at that seed, and the odd ones from the
    /// band; so the restarts keep both draws instead of trading one for the
    /// other.
    fn at(self, index: u64) -> u64 {
        if self.alternate && index.is_multiple_of(2) {
            0
        } else {
            self.width
        }
    }
}

/// Everything the sampling phase draws a candidate from. The phase asks for
/// index 0, 1, 2, … and stops at the first `None`.
#[derive(Clone, Copy)]
struct Schedule<'a> {
    base_seed: u64,
    /// The ordinary restarts run sampled min-degree in place of sampled
    /// min-fill. Set where min-fill has no prospect of finishing: a residual
    /// past the caller's size limit, or one where the initial min-fill did not
    /// come back inside its cutoff.
    min_degree_restarts: bool,
    /// Ordinary restarts on offer: the configured count, or `u64::MAX` where
    /// the restart deadline ends them instead of the count.
    ordinary_runs: u64,
    /// Diverse candidates in one pass.
    diverse_runs: u64,
    /// One entry per weighted stage, in the order the stages run. Empty when
    /// nothing is hedged.
    modified: &'a [ModifiedWeights<'a>],
    /// How many fixed orders each weighted stage repeats, zero when nothing
    /// repeats them.
    fixed_runs: u64,
    /// Builds the fixed orders, for the repeats.
    initial_orders: InitialOrderBuilder,
    /// The sampling weights every plain candidate draws with: the caller's.
    weights: &'a [u32],
    /// The band the ordinary restarts draw their tie set from.
    band: SampleBand,
}

impl<'a> Schedule<'a> {
    /// Whether the portfolio's own candidates run against modified ones.
    fn hedged(self) -> bool {
        !self.modified.is_empty()
    }

    /// How many weighted stages run, one per weighting. Zero when nothing is
    /// hedged.
    fn modified_stages(self) -> u64 {
        self.modified.len() as u64
    }

    /// The weights of the `stage`-th weighted stage.
    ///
    /// # Panics
    ///
    /// Panics when the schedule has no such stage, which leaves no modified
    /// candidate to ask.
    fn modified_weights(self, stage: u64) -> &'a [u32] {
        self.modified
            .get(stage as usize)
            .expect("a modified candidate runs only under a hedge that has its stage")
            .get()
    }

    /// Candidates in one weighted stage: the fixed orders that read the
    /// weights, then the diverse pass again.
    fn stage_length(self) -> u64 {
        self.fixed_runs.saturating_add(self.diverse_runs)
    }

    /// Candidates before the ordinary restarts: the plain diverse pass, then
    /// one weighted stage per weighting.
    fn passes_total(self) -> u64 {
        self.diverse_runs
            .saturating_add(self.stage_length().saturating_mul(self.modified_stages()))
    }

    /// Candidates the sampling phase has to offer.
    fn total(self) -> u64 {
        self.passes_total().saturating_add(self.ordinary_runs)
    }

    /// Which pass a candidate belongs to, `stage` counting the weighted stages
    /// from zero.
    fn pass(self, plain: bool, stage: u64) -> Pass {
        match (self.hedged(), plain) {
            (false, _) => Pass::Only,
            (true, true) => Pass::Plain,
            (true, false) => Pass::Modified { index: stage as u8 },
        }
    }

    /// The `index`-th fixed order that reads weights, on the `stage`-th
    /// weighted stage's weights.
    fn weighted_fixed(self, index: u64, stage: u64) -> Option<(Order<'a>, u64)> {
        (self.initial_orders)(self.base_seed, self.modified_weights(stage))
            .into_iter()
            .filter(|candidate| reads_weights(candidate.order))
            .nth(index as usize)
            .map(|candidate| (candidate.order, candidate.seed))
    }

    /// The `index`-th candidate of one diverse pass, on the caller's weights
    /// when `plain` and on the `stage`-th weighted stage's otherwise.
    fn diverse_sample(self, index: u64, plain: bool, stage: u64) -> Option<Sample<'a>> {
        let (degree_coefficient, seed_index) = Self::diverse_candidate(index);
        let weights = if plain {
            self.weights
        } else {
            self.modified_weights(stage)
        };
        sample_at(
            Order::FillDegreeSampled {
                weights,
                degree_coefficient,
            },
            sample_seed(self.base_seed, seed_index),
            self.pass(plain, stage),
            EliminationPhase::ExtraSampling,
        )
    }

    /// The coefficient and seed index of the `index`-th candidate of one
    /// diverse pass.
    fn diverse_candidate(index: u64) -> (i8, u64) {
        let initial_runs = config::DIVERSE_INITIAL_COEFFICIENTS.len() as u64;
        if index < initial_runs {
            return (config::DIVERSE_INITIAL_COEFFICIENTS[index as usize], 0);
        }
        let replay_index = index - initial_runs;
        let replay_runs = config::DIVERSE_REPLAY_COEFFICIENTS.len() as u64;
        (
            config::DIVERSE_REPLAY_COEFFICIENTS[(replay_index % replay_runs) as usize],
            1 + replay_index / replay_runs,
        )
    }
}

/// How much of the budget the hedge's weighted stages may spend, and how much
/// of it they have spent.
///
/// Each stage is as many candidates as the plain diverse pass, taken from the
/// restarts, so a schedule of several can leave a graph whose plain pass nearly
/// filled the budget with no restarts at all. The plain pass is the portfolio's
/// own measurement of what one stage costs: the same fixed orders and the same
/// diverse candidates, on other weights. So the stages get a fraction of what
/// the restart phase had left when the plain pass ended, and stop when one more
/// of them would not fit in it.
///
/// The first stage is outside the rule. A hedge runs one weighted stage
/// whatever the budget, so the rule decides how many stages come after it, not
/// whether the hedge happens at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StageBudget {
    /// What the plain pass cost: preprocessing, the fixed orders and the plain
    /// diverse pass.
    plain: Duration,
    /// What the stages may spend between them. Unbounded without a soft budget,
    /// where nothing is being taken from anything.
    allowance: Duration,
    /// What the stages that have run cost between them.
    spent: Duration,
    /// What the last stage that ran cost.
    last: Option<Duration>,
}

impl StageBudget {
    /// The stages' share of the budget, decided once the plain pass has both
    /// cost `plain` and left `left` of the time the restart phase has.
    ///
    /// `left` is `None` for a run with no soft budget. The share is then
    /// unbounded and every stage of the series runs: the rule exists to leave
    /// the restarts their time, and a run with no deadline is taking that time
    /// from nothing. It also keeps such a run's schedule independent of how
    /// long any candidate took.
    fn new(plain: Duration, left: Option<Duration>, reserve: f64) -> Self {
        Self {
            plain,
            allowance: left.map_or(Duration::MAX, |left| left.mul_f64(reserve)),
            spent: Duration::ZERO,
            last: None,
        }
    }

    /// What one more stage is projected to cost. The plain pass is the first
    /// model for it; a stage that has run is a better one, and never the worse
    /// of the two, since the incumbent width bounds the candidates of every
    /// stage after the first.
    fn projected(&self) -> Duration {
        match self.last {
            Some(last) => last.min(self.plain),
            None => self.plain,
        }
    }

    /// Whether one more stage fits in what is left of the stages' share. The
    /// first stage is not asked: a hedge runs one weighted stage on any budget,
    /// and nothing has been charged before it, which is what `last` says here.
    fn fits(&self) -> bool {
        self.last.is_none() || self.spent.saturating_add(self.projected()) <= self.allowance
    }

    /// Record a stage that ran and cost `cost`.
    fn charge(&mut self, cost: Duration) {
        self.spent = self.spent.saturating_add(cost);
        self.last = Some(cost);
    }

    /// What the rule refused a stage on.
    fn refusal(&self) -> CandidateOutcome {
        CandidateOutcome::StageSkipped {
            projected: self.projected(),
            spent: self.spent,
            allowance: self.allowance,
        }
    }
}

/// One candidate of the sampling phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sample<'a> {
    order: Order<'a>,
    seed: u64,
    pass: Pass,
    stage: Stage,
    /// How far above the minimum score this candidate's tie set reaches. Only
    /// the ordinary restarts ever carry a band.
    band: u64,
}

/// One sample, labelled as `phase` labels its order.
fn sample_at(
    order: Order<'_>,
    seed: u64,
    pass: Pass,
    phase: EliminationPhase,
) -> Option<Sample<'_>> {
    Some(Sample {
        order,
        seed,
        pass,
        stage: stage_of(order, phase),
        band: 0,
    })
}

fn extra_sample(schedule: Schedule<'_>, index: u64) -> Option<Sample<'_>> {
    let base_seed = schedule.base_seed;
    if schedule.min_degree_restarts {
        // These runs are the whole sampling phase: a residual that falls back
        // to min-degree runs neither the diverse pass nor a weighted stage, so
        // there is nothing here for a hedge to run against.
        if index >= schedule.ordinary_runs {
            return None;
        }
        return sample_at(
            Order::MinDegreeSampled {
                weights: schedule.weights,
            },
            sample_seed(base_seed, index),
            Pass::Only,
            EliminationPhase::ExtraSampling,
        );
    }

    debug_assert!(schedule.diverse_runs <= config::MAX_DIVERSE_SAMPLING_RUNS);
    // The first diverse pass is the one a portfolio without the hedge runs:
    // the caller's weights and the same seeds.
    if index < schedule.diverse_runs {
        return schedule.diverse_sample(index, true, 0);
    }
    let stage_length = schedule.stage_length();
    if schedule.hedged() && stage_length > 0 {
        // Then one weighted stage per weighting, in the series' order: the
        // fixed orders that read the weights, on that stage's ranking this
        // time, and then the diverse pass again on it.
        let after_plain = index - schedule.diverse_runs;
        if after_plain < stage_length.saturating_mul(schedule.modified_stages()) {
            let stage_index = after_plain / stage_length;
            let within = after_plain % stage_length;
            if within < schedule.fixed_runs {
                let (order, seed) = schedule.weighted_fixed(within, stage_index)?;
                return sample_at(
                    order,
                    seed,
                    schedule.pass(false, stage_index),
                    EliminationPhase::Initial,
                );
            }
            return schedule.diverse_sample(within - schedule.fixed_runs, false, stage_index);
        }
    }

    let ordinary_index = index - schedule.passes_total();
    if ordinary_index >= schedule.ordinary_runs {
        return None;
    }
    // Nothing is given up here: the restarts are the whole sequence a
    // portfolio without the hedge runs, seed for seed.
    let mut restart = sample_at(
        Order::MinFillSampled {
            weights: schedule.weights,
        },
        sample_seed(base_seed, ordinary_index),
        schedule.pass(true, 0),
        EliminationPhase::ExtraSampling,
    )?;
    restart.band = schedule.band.at(ordinary_index);
    Some(restart)
}

/// The label for `order` in `phase`. A sampled min-fill order is a restart in
/// the sampling phase and the portfolio's own min-fill candidate before it.
fn stage_of(order: Order<'_>, phase: EliminationPhase) -> Stage {
    match (order, phase) {
        (Order::NestedDissection, _) => Stage::NestedDissection,
        (Order::MinDegree | Order::MinDegreeSampled { .. }, _) => Stage::MinDegree,
        (
            Order::MinFill | Order::MinFillSampled { .. },
            EliminationPhase::Initial | EliminationPhase::AdmittedInitial(_),
        ) => Stage::MinFill,
        (Order::MinFill | Order::MinFillSampled { .. }, EliminationPhase::ExtraSampling) => {
            Stage::Sample
        }
        (
            Order::FillDegreeSampled {
                degree_coefficient, ..
            },
            _,
        ) => Stage::Diverse { degree_coefficient },
    }
}

/// The window the trailing FlowCutter candidate is given.
///
/// `left` is what the hard deadline still has, and `None` on a run without one.
/// Against a hard deadline the window stops `reserve` short of it, so the run
/// ends inside the time the portfolio actually has. Without a hard deadline
/// there is nothing to end inside and the configured budget stands.
fn flowcutter_window(
    configured_budget: Duration,
    left: Option<Duration>,
    reserve: Duration,
) -> Duration {
    match left {
        Some(left) => left.min(configured_budget).saturating_sub(reserve),
        None => configured_budget,
    }
}

/// What the run has spent so far, on both clocks.
#[derive(Clone, Copy)]
struct Spent {
    elapsed: Duration,
    charged_units: u64,
}

/// `estimate` at the rate this run has actually been going.
///
/// The library charges graph work in the units the FlowCutter estimates are
/// written in, so the wall time a run has spent divided by the work it has
/// charged says what one modelled millisecond has cost here. On a box running
/// one solve per core it is several. The value is never scaled down: the
/// estimate at the model's own rate is the floor. Under an armed meter both
/// numbers come from the same clock and the estimate is returned unchanged.
fn at_observed_rate(estimate: Duration, spent: Spent) -> Duration {
    let modelled = crate::meter::milliseconds_for_units(spent.charged_units);
    let elapsed = u64::try_from(spent.elapsed.as_millis()).unwrap_or(u64::MAX);
    if modelled == 0 || elapsed <= modelled {
        return estimate;
    }
    let estimate_ms = u64::try_from(estimate.as_millis()).unwrap_or(u64::MAX);
    Duration::from_millis(estimate_ms.saturating_mul(elapsed) / modelled)
}

fn flowcutter_candidate(
    graph: &Graph,
    configured_budget: Duration,
    hard_deadline: Option<Instant>,
    spent: Spent,
) -> Result<Option<TreeDecomposition>, Error> {
    let vertices = u64::from(graph.num_vertices);
    let edges = graph.edges.len() as u64;
    // The same work-unit model the metered path charges the backend with.
    let one_restart = Duration::from_millis(crate::meter::milliseconds_for_units(
        crate::flowcutter::iteration_work_units(vertices, edges),
    ));
    // What the end of the run costs once the window is up. The backend tests
    // its deadline between restarts, so it returns up to one restart late, and
    // the result is then copied out of it a bag at a time. On a 1,728-vertex
    // primal graph whose result has 114,600 bags the two came to about 200 ms
    // against a modelled restart of 172, so the reserve is two restarts. Both
    // are taken at the rate the run has been going: at the model's own rate the
    // reserve is a fraction of what a loaded machine spends here, and the
    // candidate then returns after the deadline it was sized for.
    let reserve = at_observed_rate(RESERVE_RESTARTS * one_restart, spent);
    let timeout = flowcutter_window(
        configured_budget,
        hard_deadline.map(crate::deadline::remaining),
        reserve,
    );
    // Skip windows too small to seed useful FlowCutter iterations; FFI overhead
    // alone eats tens of ms on small graphs.
    if timeout < Duration::from_millis(MIN_FLOWCUTTER_CANDIDATE_MS) {
        return Ok(None);
    }
    // Skip windows too small for this graph: a graph whose setup and first
    // restart already outlast the window cannot be stopped inside it, so the
    // run comes back long after the portfolio's hard deadline with a result the
    // caller has no time left to write. Measured at a 4.75-second window: 6.8
    // seconds on a graph of 79,000 vertices and 175,000 edges, 115 seconds on
    // one of 92,000 and 1.08 million.
    let first_restart = crate::flowcutter::first_restart_units(vertices, edges);
    if Duration::from_millis(crate::meter::milliseconds_for_units(first_restart)) > timeout {
        return Ok(None);
    }
    match flowcutter_decompose(
        graph,
        Budget::timed(
            timeout,
            Some(FLOWCUTTER_CANDIDATE_PATIENCE),
            FLOWCUTTER_CANDIDATE_ITERATIONS,
        ),
    ) {
        Ok(decomposition) => Ok(Some(decomposition)),
        // A timed backend run may end before it has a result. The elimination
        // candidates already make the portfolio complete, so this one can be
        // absent without changing the contract.
        Err(Error::NoDecomposition | Error::TooLarge(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EliminationPhase {
    Initial,
    /// An expensive initial order on a residual the caller's raised limit
    /// admitted, carrying the instant it runs to.
    AdmittedInitial(Option<Instant>),
    ExtraSampling,
}

/// Where the sampled restarts stop.
///
/// On a residual at or below the caller's limit they run past the soft deadline
/// into the hard window, keeping [`FLOWCUTTER_RESERVE`] at the end of it for
/// the trailing FlowCutter candidate. More restart time is worth more than a
/// longer FlowCutter tail on these graphs, and an admitted residual that spent
/// its soft budget on the first candidate has nothing else to do with the
/// second stage: a restart projected to run into the deadline is never started,
/// so the wider window only adds restarts that fit.
///
/// Past the caller's limit the soft deadline stands, so the second stage stays
/// with FlowCutter: a restart there costs a large fraction of the window, and
/// on graphs of that size FlowCutter is regularly the widest margin the
/// portfolio has. The soft deadline also stands on a run with no hard deadline,
/// and where the reserve would put the stop before the soft deadline on a hard
/// window shorter than the reserve.
fn restart_deadline(
    residual: Residual,
    soft_deadline: Option<Instant>,
    hard_deadline: Option<Instant>,
) -> Option<Instant> {
    if residual == Residual::Large {
        return soft_deadline;
    }
    let (Some(soft), Some(hard)) = (soft_deadline, hard_deadline) else {
        return soft_deadline;
    };
    match hard.checked_sub(FLOWCUTTER_RESERVE) {
        Some(reserved) if reserved > soft => Some(reserved),
        _ => Some(soft),
    }
}

/// The cutoff an expensive order on an admitted residual runs to: half the time
/// the restarts' own deadline has left when the order starts.
///
/// Each order gives back at least what it does not use, so however many of them
/// run, the restarts still start with time in hand. An admitted residual's
/// restart deadline sits a reserve short of the hard deadline, so this halves
/// what is left of both stages. With no restart deadline there is nothing to
/// halve and the order runs to the portfolio's hard deadline, as it does below
/// the band.
fn admitted_cutoff(
    restart_deadline: Option<Instant>,
    hard_deadline: Option<Instant>,
) -> Option<Instant> {
    let Some(restart) = restart_deadline else {
        return hard_deadline;
    };
    Some(crate::meter::now() + remaining(restart) / 2)
}

/// The deadline the initial loop reads between candidates: once it has passed,
/// whatever has already returned is the answer.
///
/// An admitted residual reads the deadline its restarts stop at, the hard
/// window less the FlowCutter reserve, so a first candidate that spends the
/// whole soft budget by itself no longer ends the schedule. On graphs of that
/// size that was the common case, and the paced min-fill that follows is the
/// candidate most likely to be narrower than the min-degree pass that answered.
///
/// The soft deadline still does its two other jobs there: preprocessing stops
/// at it, and a deterministic greedy order that reaches it switches to cheaper
/// scoring for the rest of its elimination. What the change moves is only
/// whether the next candidate starts.
///
/// Below and above the band the soft deadline stands. An ordinary residual has
/// a schedule of cheap candidates that finish well inside it, and past the
/// caller's limit only min-degree candidates run and the second stage belongs
/// to FlowCutter.
fn initial_candidate_deadline(
    residual: Residual,
    soft_deadline: Option<Instant>,
    restart_deadline: Option<Instant>,
) -> Option<Instant> {
    match residual {
        Residual::Admitted => restart_deadline,
        Residual::Ordinary | Residual::Large => soft_deadline,
    }
}

/// Whether another restart is admitted: one projected to cost `projected` and
/// started at `now` has to end before every deadline it must respect.
///
/// A restart that would run into its deadline is stopped part-way and leaves
/// nothing behind, so starting it only takes time from the trailing FlowCutter
/// candidate. The projection is what the previous restart cost, which is the
/// portfolio's own measurement of one restart on this graph.
fn restart_admitted(now: Instant, projected: Duration, deadlines: [Option<Instant>; 2]) -> bool {
    let Some(finish) = now.checked_add(projected) else {
        return false;
    };
    deadlines
        .iter()
        .flatten()
        .all(|deadline| finish <= *deadline)
}

/// Initial candidates may use the complete two-stage window so the first one
/// can always return a decomposition. Extra samples stop at the restart
/// deadline; the rest of the hard window belongs to FlowCutter.
///
/// An expensive order on an admitted residual stops at a cutoff of its own,
/// half the time the restart deadline had left when it started. At that size it
/// often cannot finish, and given the whole window it returns nothing and
/// leaves no time for the restarts either.
fn elimination_stop(
    phase: EliminationPhase,
    soft_deadline: Option<Instant>,
    hard_deadline: Option<Instant>,
    width_bound: Option<u32>,
) -> ElimStop {
    ElimStop {
        soft_deadline,
        hard_deadline: match phase {
            EliminationPhase::Initial => hard_deadline,
            EliminationPhase::AdmittedInitial(cutoff) => cutoff,
            EliminationPhase::ExtraSampling => soft_deadline,
        },
        width_bound,
    }
}

/// One candidate before the restart phase.
#[derive(Clone, Copy)]
struct InitialCandidate<'a> {
    order: Order<'a>,
    seed: u64,
    preprocess: bool,
    update_order_ties: bool,
}

/// Builds a run's candidate list for a seed, given the weight vector.
type InitialOrderBuilder = for<'w> fn(u64, &'w [u32]) -> Vec<InitialCandidate<'w>>;

#[derive(Clone, Copy)]
enum CandidateRetention {
    All,
    BestOnly,
}

/// The standard portfolio's fixed candidates. Vertex-order min-degree runs
/// first so it supplies a deterministic incumbent before the sampled orders.
fn standard_orders(base_seed: u64, weights: &[u32]) -> Vec<InitialCandidate<'_>> {
    let second_seed = base_seed.wrapping_add(SECOND_CANDIDATE_SEED_OFFSET);
    vec![
        InitialCandidate {
            order: Order::MinDegree,
            seed: base_seed,
            preprocess: false,
            update_order_ties: true,
        },
        InitialCandidate {
            order: Order::MinDegreeSampled { weights },
            seed: base_seed,
            preprocess: true,
            update_order_ties: false,
        },
        InitialCandidate {
            order: Order::NestedDissection,
            seed: base_seed,
            preprocess: true,
            update_order_ties: false,
        },
        InitialCandidate {
            order: Order::MinFillSampled { weights },
            seed: base_seed,
            preprocess: true,
            update_order_ties: false,
        },
        InitialCandidate {
            order: Order::MinDegreeSampled { weights },
            seed: second_seed,
            preprocess: true,
            update_order_ties: false,
        },
        InitialCandidate {
            order: Order::NestedDissection,
            seed: second_seed,
            preprocess: true,
            update_order_ties: false,
        },
    ]
}

/// Whether `order` draws its ties from the sampling weights, and so runs a
/// second time under a hedge.
fn reads_weights(order: Order<'_>) -> bool {
    order.tie_weights().is_some()
}

/// The fixed candidate at the front of the sampled-min-fill portfolio.
fn sampled_min_fill_orders(base_seed: u64, weights: &[u32]) -> Vec<InitialCandidate<'_>> {
    vec![InitialCandidate {
        order: Order::MinFillSampled { weights },
        seed: base_seed,
        preprocess: true,
        update_order_ties: false,
    }]
}

/// Run a portfolio: the initial orders the residual's size allows, then extra
/// sampled orders with the remaining budget, then the trailing FlowCutter
/// candidate where configured.
///
/// At least one candidate always carries a decomposition: the first is exempt from both
/// between-candidate skips, runs with no `width_bound` (so it cannot abort on
/// width) and with deadline completion enabled (so a deadline stop still
/// yields a decomposition).
///
/// `weights` has one entry per vertex and is what the sampling orders draw tie
/// sets with. Every candidate and every sampled order shares it unless the
/// portfolio hedges, which runs a weighted stage per weighting on weights of
/// its own.
fn run_portfolio(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    initial_orders: InitialOrderBuilder,
    config: PortfolioConfig,
    retention: CandidateRetention,
    trace: &mut dyn FnMut(CandidateTrace),
) -> Result<CandidateSet, crate::Error> {
    config::validate(config)?;
    let started = crate::meter::now();
    // Where the run stands on the work clock, so estimates written in work
    // units can be read at the rate this machine is actually running them.
    let started_units = crate::meter::units_spent();
    let deadlines =
        crate::deadline::staged(started, config.soft_budget, config.hard_budget, "portfolio")?;
    let soft_deadline = deadlines.soft;
    let hard_deadline = deadlines.hard;
    let mut prebuilt = engine::prebuild(graph, soft_deadline);
    let mut original = None;
    let residual = Residual::classify(prebuilt.num_active(), config.expensive_orders_up_to);
    // Where the restart phase stops, and where the initial loop stops starting
    // another candidate.
    let restart_deadline = restart_deadline(residual, soft_deadline, hard_deadline);
    let initial_deadline = initial_candidate_deadline(residual, soft_deadline, restart_deadline);
    let cells: [OnceCell<Vec<u32>>; MAX_HEDGE_PASSES] = std::array::from_fn(|_| OnceCell::new());
    // Only an ordinary residual hedges. A larger one runs restarts and nothing
    // else, so there is nothing there for a weighted stage to run against, and
    // the ranking it would place is work the restarts would rather have. Each
    // stage's weights are derived when its first candidate asks for them.
    let modified: Vec<ModifiedWeights<'_>> = match config.hedge.series() {
        Some(series) if residual == Residual::Ordinary => series
            .weights()
            .iter()
            .zip(&cells)
            .map(|(entry, cell)| match *entry {
                HedgeWeights::Eccentricity { dim, rounds } => ModifiedWeights::Ranked {
                    cell,
                    graph,
                    dim,
                    rounds,
                    seed,
                    deadline: restart_deadline,
                },
                HedgeWeights::Random { stream } => ModifiedWeights::Random {
                    cell,
                    count: graph.num_vertices() as usize,
                    seed: hedge_random_seed(seed, stream),
                },
            })
            .collect(),
        _ => Vec::new(),
    };
    // Every weighted stage repeats the fixed orders that read weights after the
    // plain diverse pass. Which orders those are does not depend on the
    // weights, so the count is known before a ranking is placed.
    let fixed_runs = if !modified.is_empty() {
        initial_orders(seed, weights)
            .iter()
            .filter(|candidate| reads_weights(candidate.order))
            .count() as u64
    } else {
        0
    };
    // The builder is needed again for the fixed orders the hedge repeats.
    let order_builder = initial_orders;
    let initial_orders = initial_orders(seed, weights);
    let mut candidates = match retention {
        CandidateRetention::All => CandidateSet::all(initial_orders.len() + 1),
        CandidateRetention::BestOnly => CandidateSet::best_only(),
    };

    // Set after any candidate reaches the hard deadline, or when it expires
    // between candidates. Later runs would stop at the same point.
    let mut hard_deadline_tripped = false;
    // Set by an initial min-fill order that produced a decomposition.
    let mut min_fill_finished = false;

    for (i, candidate) in initial_orders.iter().copied().enumerate() {
        let order = candidate.order;
        // Honour the deadline between orders (when set), but always run
        // order 0 so we return something even on huge graphs that would
        // otherwise time out inside the first order.
        if i > 0 && expired(initial_deadline) {
            break;
        }
        // Past the caller's limit, only min-degree variants reliably complete;
        // nested dissection and min-fill can overrun a short budget.
        let expensive = !is_min_degree_variant(order);
        if i > 0 && residual == Residual::Large && expensive {
            continue;
        }
        // Nested dissection reads its deadline between levels, and its
        // bisection of one level on a graph of a million edges takes seconds
        // on its own, so a cutoff does not bound it. An admitted residual does
        // not run it; the slot is traced so a reader can see it was given up.
        if residual == Residual::Admitted && matches!(order, Order::NestedDissection) {
            trace(CandidateTrace {
                stage: Stage::NestedDissection,
                seed: candidate.seed,
                pass: Pass::Only,
                outcome: CandidateOutcome::NotStarted,
                elapsed: crate::meter::now().saturating_duration_since(started),
            });
            continue;
        }
        // An admitted residual gives each expensive order half the time the
        // restarts' own deadline has left, so whatever it does with that time
        // the restarts still get a share of the budget. The min-degree
        // candidates keep the window they have on any residual, since one of
        // them has to come back with a decomposition.
        let phase = if residual == Residual::Admitted && expensive {
            EliminationPhase::AdmittedInitial(admitted_cutoff(restart_deadline, hard_deadline))
        } else {
            EliminationPhase::Initial
        };
        // Complete the residual only while no candidate has produced a usable
        // decomposition yet. Once one has, completing a later candidate's
        // residual would be wasted work: its wide decomposition would lose on
        // width and total bag size to the existing winner.
        let complete_on_deadline = candidates.is_empty();
        let candidate_graph = if candidate.preprocess {
            &mut prebuilt
        } else {
            original.get_or_insert_with(|| engine::prebuild_original(graph))
        };
        let run = engine::run_order_prebuilt(
            candidate_graph,
            engine::RunSpec {
                order,
                seed: candidate.seed,
                // Only the restarts draw from a band; see the sampling phase
                // below.
                sample_band: 0,
                update_order_ties: candidate.update_order_ties,
                stop: elimination_stop(
                    phase,
                    soft_deadline,
                    hard_deadline,
                    candidates.best_width(),
                ),
                complete_on_deadline,
                // On an admitted residual the fill counts the order seeds
                // its buckets from are computed under the same cutoff as the
                // order itself; below the band the setup runs to completion
                // as it always has.
                setup_deadline: match phase {
                    EliminationPhase::AdmittedInitial(cutoff) => cutoff,
                    _ => None,
                },
            },
        );
        let (outcome, stop) = candidates.record_elimination(run);
        // What the restarts of an admitted residual follow: a min-fill order
        // that came back with a decomposition finished inside its cutoff, so
        // sampled min-fill has a prospect of finishing too.
        if is_min_fill_variant(order) && matches!(outcome, CandidateOutcome::Produced { .. }) {
            min_fill_finished = true;
        }
        trace(CandidateTrace {
            stage: stage_of(order, phase),
            seed: candidate.seed,
            pass: Pass::Only,
            outcome,
            elapsed: crate::meter::now().saturating_duration_since(started),
        });
        // An expensive order on an admitted residual runs to a cutoff of its
        // own, and the engine reports reaching that the same way it reports the
        // portfolio's hard deadline. Reaching it says nothing about how much of
        // the portfolio's budget is left, so read the clock instead of taking
        // the candidate's word and stopping the run.
        let own_cutoff = matches!(phase, EliminationPhase::AdmittedInitial(_));
        hard_deadline_tripped = match stop {
            ScheduleStop::HardDeadline if !own_cutoff => true,
            // Either the candidate finished inside its budget, or it was
            // stopped by a cutoff that was not the portfolio's. Either way the
            // portfolio still holds whatever is left of the hard deadline, so
            // only the clock decides.
            ScheduleStop::HardDeadline | ScheduleStop::Continue => match outcome {
                // Nothing usable from this candidate, but the portfolio is
                // still inside its budget.
                CandidateOutcome::WidthAborted => false,
                // Only the sampling phase has stages to skip, and only the
                // trailing FlowCutter slot reports an unstarted candidate.
                CandidateOutcome::StageSkipped { .. } | CandidateOutcome::NotStarted => false,
                CandidateOutcome::Produced { .. } | CandidateOutcome::DeadlineReached => {
                    expired(hard_deadline)
                }
            },
        };
        if hard_deadline_tripped {
            break;
        }
    }
    // Sampling phase: try additional seeds of the full-tie-set sampling
    // order with any remaining budget. Measured ≥79% of min-fill pops have
    // ≥2 tied candidates, so different seeds explore different
    // elimination orders and can lower width on small/medium graphs where the
    // base portfolio returns in tens of ms. Falls back to sampled min-degree
    // where min-fill has no prospect of finishing: past the caller's size limit,
    // matching the main loop's skip rule, and on an admitted residual whose
    // initial min-fill did not come back. A started extra
    // sample stops at the restart deadline so it cannot consume the trailing
    // FlowCutter and output interval. On extended small/medium runs, diverse
    // fill-degree scores precede the complete ordinary min-fill seed sequence.
    // A hedge adds one weighted stage per weighting between the two — the fixed
    // orders that read weights and the diverse pass again — and leaves the
    // restarts where they were.
    //
    // The sampling count caps how many seeds are drawn, not the clock, so a
    // graph whose candidates are quick can finish the schedule with budget
    // left. Configured to, the restarts carry on from the next seed of the
    // same sequence and the restart deadline ends them. Without a deadline
    // there is nothing else to stop at, so the count stands.
    let ordinary_runs = if config.restarts_to_deadline && restart_deadline.is_some() {
        u64::MAX
    } else {
        config.sampling_runs
    };
    let diverse_samples = if residual == Residual::Ordinary {
        config.diverse_sampling_runs
    } else {
        0
    };
    // Sampled min-fill restarts are worth drawing only where min-fill can
    // finish: below the size rule, or on an admitted residual where the
    // initial min-fill did finish. Everywhere else they fall back to sampled
    // min-degree, which is what a residual past the limit has always run.
    let min_degree_restarts = match residual {
        Residual::Ordinary => false,
        Residual::Admitted => !min_fill_finished,
        Residual::Large => true,
    };
    let schedule = Schedule {
        base_seed: seed,
        min_degree_restarts,
        ordinary_runs,
        diverse_runs: diverse_samples,
        modified: &modified,
        fixed_runs,
        initial_orders: order_builder,
        weights,
        band: SampleBand {
            width: config.sample_band,
            alternate: config.sample_band_alternate,
        },
    };
    let total_samples = schedule.total();
    // Where the weighted stages sit in the sample sequence, and how long one of
    // them is. A schedule with no stage leaves this empty.
    let stage_length = schedule.stage_length();
    let stages_start = schedule.diverse_runs;
    let stages_end = schedule.passes_total();
    let stage_count = schedule.modified_stages();
    // Decided at the end of the plain pass, from what that pass cost and what
    // the restart phase has left.
    let mut stage_budget: Option<StageBudget> = None;
    let mut stage_started = Duration::ZERO;
    let mut sample_index: u64 = 0;
    // When the last restart ended and what it cost, for the admission rule
    // below. The first restart of the loop has nothing to be projected from and
    // runs on the deadline checks alone.
    let mut restart_finished = crate::meter::now();
    let mut previous_restart: Option<Duration> = None;
    // Normally the restart deadline fires first; the portfolio hard-deadline
    // check also prevents another sample after an initial candidate used the
    // complete two-stage window.
    while sample_index < total_samples
        && !hard_deadline_tripped
        && !expired(restart_deadline)
        && !expired(hard_deadline)
    {
        // At the front of a weighted stage, charge the one that just ended and
        // ask whether one more fits; the first stage runs whatever the answer.
        // Nothing after a refusal fits either — the projection never grows and
        // the spend never falls — so the refusal takes every stage that is left
        // and the restarts start here.
        if stage_length > 0
            && (stages_start..stages_end).contains(&sample_index)
            && (sample_index - stages_start).is_multiple_of(stage_length)
        {
            let elapsed = crate::meter::now().saturating_duration_since(started);
            let stage_index = (sample_index - stages_start) / stage_length;
            let budget = stage_budget.get_or_insert_with(|| {
                StageBudget::new(
                    elapsed,
                    restart_deadline.map(remaining),
                    config.hedge_reserve,
                )
            });
            if stage_index > 0 {
                budget.charge(elapsed.saturating_sub(stage_started));
            }
            if !budget.fits() {
                let outcome = budget.refusal();
                for skipped in stage_index..stage_count {
                    trace(CandidateTrace {
                        stage: Stage::WeightedStage,
                        seed,
                        pass: Pass::Modified {
                            index: skipped as u8,
                        },
                        outcome,
                        elapsed,
                    });
                }
                sample_index = stages_end;
                continue;
            }
            stage_started = elapsed;
        }
        // One more restart is only started when the last one's cost still fits
        // before both deadlines.
        if let Some(projected) = previous_restart
            && !restart_admitted(
                restart_finished,
                projected,
                [restart_deadline, hard_deadline],
            )
        {
            break;
        }
        let candidate = extra_sample(schedule, sample_index)
            .expect("sample index is below the configured total");
        // Extra sampling only runs after the fixed candidates, so at least one prior
        // candidate won, so deadline completion is unnecessary here.
        let run = engine::run_order_prebuilt(
            &mut prebuilt,
            engine::RunSpec {
                order: candidate.order,
                seed: candidate.seed,
                sample_band: candidate.band,
                update_order_ties: false,
                stop: elimination_stop(
                    EliminationPhase::ExtraSampling,
                    restart_deadline,
                    hard_deadline,
                    candidates.best_width(),
                ),
                complete_on_deadline: false,
                setup_deadline: None,
            },
        );
        let (outcome, _) = candidates.record_elimination(run);
        let finished = crate::meter::now();
        previous_restart = Some(finished.saturating_duration_since(restart_finished));
        restart_finished = finished;
        trace(CandidateTrace {
            stage: candidate.stage,
            seed: candidate.seed,
            pass: candidate.pass,
            outcome,
            elapsed: finished.saturating_duration_since(started),
        });
        match outcome {
            // No time left for more sampled orders.
            CandidateOutcome::DeadlineReached => break,
            // A width-aborted seed keeps sampling: another seed
            // explores a different elimination order.
            //
            // A skipped stage is reported by the rule above and never comes
            // back from a candidate.
            //
            // A candidate that ran has a result, so it never reports
            // `NotStarted`; only a slot the size rule gave up does.
            CandidateOutcome::Produced { .. }
            | CandidateOutcome::WidthAborted
            | CandidateOutcome::NotStarted
            | CandidateOutcome::StageSkipped { .. } => {
                sample_index += 1;
            }
        }
    }
    // Runs vanilla FlowCutter once as a final portfolio candidate. Placed after
    // the extra-sampling loop so it runs in whatever is left of the hard
    // window: the restart reserve at or below the caller's limit, and up to the
    // whole second stage past it, where the restarts stopped at the soft
    // deadline. FlowCutter already returns a complete decomposition, so no
    // separator-refinement pass is applied to it. It runs on every residual;
    // on the large ones it is often the best candidate by a wide margin, and
    // `flowcutter_candidate` has its own vertex cap.
    if let Some(configured_budget) = config
        .flowcutter_budget
        .filter(|_| !hard_deadline_tripped && !expired(hard_deadline))
        && let Some(decomposition) = flowcutter_candidate(
            graph,
            configured_budget,
            hard_deadline,
            Spent {
                elapsed: crate::meter::now().saturating_duration_since(started),
                charged_units: crate::meter::units_spent().saturating_sub(started_units),
            },
        )?
    {
        let outcome = candidates.push(decomposition);
        trace(CandidateTrace {
            stage: Stage::FlowCutter,
            seed,
            pass: Pass::Only,
            outcome,
            elapsed: crate::meter::now().saturating_duration_since(started),
        });
    }
    Ok(candidates)
}

/// Run one sampled min-fill order, then up to
/// the configured number of further seeds, then an optional trailing
/// FlowCutter candidate — and return every decomposition produced, in candidate
/// order. Never empty: the first candidate always produces one.
///
/// The caller picks among them, commonly by width and then total bag size.
///
/// # Errors
///
/// Returns an error for a weight count that differs from the graph vertex
/// count, an invalid deadline or FlowCutter budget, or an invalid FlowCutter
/// result.
pub fn sampled_min_fill_candidates(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
) -> Result<Vec<TreeDecomposition>, crate::Error> {
    validate_weights(graph, weights)?;
    Ok(run_portfolio(
        graph,
        weights,
        seed,
        sampled_min_fill_orders,
        config,
        CandidateRetention::All,
        &mut |_| {},
    )?
    .into_decompositions())
}

fn standard_candidate_set(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
    retention: CandidateRetention,
    trace: &mut dyn FnMut(CandidateTrace),
) -> Result<CandidateSet, crate::Error> {
    validate_weights(graph, weights)?;
    run_portfolio(
        graph,
        weights,
        seed,
        standard_orders,
        config,
        retention,
        trace,
    )
}

/// Run the standard portfolio and return every decomposition it produced,
/// sorted ascending by width and total bag size, with ties kept in candidate order
/// (a stable sort), so the first is the portfolio's winner. Never empty.
///
/// # Errors
///
/// Returns an error for a weight count that differs from the graph vertex
/// count, an invalid deadline or FlowCutter budget, or an invalid FlowCutter
/// result.
pub fn candidates(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
) -> Result<Vec<TreeDecomposition>, crate::Error> {
    let mut decompositions = standard_candidate_set(
        graph,
        weights,
        seed,
        config,
        CandidateRetention::All,
        &mut |_| {},
    )?
    .into_decompositions();
    decompositions.sort_by_key(TreeDecomposition::quality_key);
    Ok(decompositions)
}

/// Return the standard portfolio's best candidate by width, then total bag
/// size. Bags contained in an adjacent bag are contracted before return.
///
/// # Errors
///
/// Returns the same configuration and weight errors as [`candidates`].
pub fn decompose(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
) -> Result<TreeDecomposition, crate::Error> {
    decompose_traced(graph, weights, seed, config, &mut |_| {})
}

/// [`decompose`], reporting every candidate to `trace` as it finishes.
///
/// The portfolio returns one decomposition and says nothing about where it
/// came from; this says. The candidate the portfolio returns is the last one
/// reported as [`CandidateOutcome::Produced`] with `best` set.
///
/// # Errors
///
/// Returns the same configuration and weight errors as [`candidates`].
pub fn decompose_traced(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
    trace: &mut dyn FnMut(CandidateTrace),
) -> Result<TreeDecomposition, crate::Error> {
    Ok(standard_candidate_set(
        graph,
        weights,
        seed,
        config,
        CandidateRetention::BestOnly,
        trace,
    )?
    .into_decompositions()
    .into_iter()
    .next()
    .expect("first candidate always produces a decomposition"))
}

/// The standard portfolio's winner, refined by FlowCutter cuts
/// ([`refine_with_flowcutter`](crate::decomposition::refine_with_flowcutter)).
///
/// `refinement_budget` bounds the refinement pass. Both halves are anytime:
/// the portfolio keeps the best decomposition found so far, and a skipped
/// refinement returns it unchanged.
///
/// # Errors
///
/// Returns the same errors as [`decompose`] and
/// [`refine_with_flowcutter`](crate::decomposition::refine_with_flowcutter).
pub fn decompose_and_refine(
    graph: &Graph,
    weights: &[u32],
    seed: u64,
    config: PortfolioConfig,
    refinement_budget: Option<Duration>,
) -> Result<TreeDecomposition, crate::Error> {
    let td = decompose(graph, weights, seed, config)?;
    decomposition::refine_with_flowcutter(td, graph, refinement_budget)
}

fn validate_weights(graph: &Graph, weights: &[u32]) -> Result<(), crate::Error> {
    if weights.len() != graph.num_vertices as usize {
        return Err(crate::Error::InvalidInput(format!(
            "portfolio has {} weights for {} vertices",
            weights.len(),
            graph.num_vertices
        )));
    }
    Ok(())
}
