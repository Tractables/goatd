use super::*;

/// Restart RNG, cutter and trajectory retained across cooperative checkpoints.
pub(crate) struct Search {
    graph: OrigGraph,
    random: MinstdRand,
    cutter: Cutter,
    round: Option<Round>,
    best: Option<Vec<u32>>,
    iteration: i32,
    iterations: i32,
    steps_left: i64,
    step_cost: i64,
}

impl Search {
    pub(crate) fn new(n: usize, edges: &[(u32, u32)], steps: i64, iterations: i32) -> Option<Self> {
        if n < 3 {
            return None;
        }
        let graph = OrigGraph::build(n as u32, edges)?;
        if !is_connected(&graph) {
            return None;
        }
        let arcs = graph.tail.len().max(1);
        let step_cost = (((n as f64).sqrt() * (arcs as f64).sqrt()) / 50.0).max(1.0) as i64;
        Some(Self {
            graph,
            random: MinstdRand::new(),
            cutter: Cutter::new(),
            round: None,
            best: None,
            iteration: 0,
            iterations: iterations.max(1),
            steps_left: steps,
            step_cost,
        })
    }

    pub(crate) fn exhausted(&self) -> bool {
        self.round.is_none() && (self.iteration >= self.iterations || self.steps_left <= 0)
    }

    pub(crate) fn attempts(&self) -> u64 {
        self.iteration as u64
    }

    /// Initialize one restart or advance one cut trajectory. Neither operation
    /// reads the clock; drivers check their own limits between calls.
    pub(crate) fn step(&mut self, offer: bool) -> Option<Vec<u32>> {
        if let Some(round) = &mut self.round {
            if round.step(&self.graph, &mut self.cutter) {
                let round = self.round.take().expect("the active round just finished");
                if offer {
                    retain_smaller(&mut self.best, round.best.clone());
                    return round.best;
                }
                retain_smaller(&mut self.best, round.best);
            }
            return None;
        }
        if self.exhausted() {
            return None;
        }
        let balance = match self.iteration % 3 {
            2 => 0.2_f32,
            1 => 0.1_f32,
            _ => 0.0_f32,
        };
        self.iteration += 1;
        self.steps_left -= self.step_cost;
        crate::meter::charge(super::super::native::iteration_work_units(
            self.graph.n as u64,
            (self.graph.tail.len() / 2) as u64,
        ));
        self.round = Round::new(
            &self.graph,
            &mut self.cutter,
            self.random.next() as u64,
            balance,
        );
        None
    }

    pub(crate) fn best_vertices(&self) -> Option<&[u32]> {
        let active = self.round.as_ref().and_then(|round| round.best.as_ref());
        match (&self.best, active) {
            (Some(best), Some(active)) if !active.is_empty() && active.len() < best.len() => {
                Some(active)
            }
            (Some(best), _) => Some(best),
            (None, active) => active.map(Vec::as_slice),
        }
    }

    pub(crate) fn into_vertices(mut self) -> Option<Vec<u32>> {
        if let Some(round) = self.round.take() {
            retain_smaller(&mut self.best, round.best);
        }
        self.best
    }
}

fn retain_smaller(best: &mut Option<Vec<u32>>, next: Option<Vec<u32>>) {
    if let Some(next) = next
        && !next.is_empty()
        && best.as_ref().is_none_or(|old| next.len() < old.len())
    {
        *best = Some(next);
    }
}

struct Round {
    best: Option<Vec<u32>>,
    best_score: f64,
    min_balance: f64,
    iterations: u32,
}

impl Round {
    fn new(graph: &OrigGraph, cutter: &mut Cutter, seed: u64, balance: f32) -> Option<Self> {
        let (s, t) = select_random_st_pair(graph.n, seed)?;
        cutter.init(
            graph,
            (orig_node_to_exp(s, false), orig_node_to_exp(t, true)),
        );
        Some(Self {
            best: None,
            best_score: f64::INFINITY,
            min_balance: balance as f64 * n_exp(graph.n) as f64,
            iterations: 0,
        })
    }

    fn step(&mut self, graph: &OrigGraph, cutter: &mut Cutter) -> bool {
        self.iterations += 1;
        if self.iterations > 10_000_000 {
            return true;
        }
        let cut_size = cutter.current_cut_size() as f64;
        // At least one node: `init` seeds both sides before the first read.
        let small_side = cutter.current_smaller_size() as f64;
        let mut score = cut_size / small_side;
        if cutter.current_smaller_size() < self.min_balance as u32 {
            score += 1_000_000.0;
        }
        if score < self.best_score {
            self.best_score = score;
            let separator = extract_original_separator(graph, cutter);
            let too_large = separator.len() > 10_000;
            self.best = Some(separator);
            if too_large {
                return true;
            }
        }
        let potential = (cut_size + 1.0) / (n_exp(graph.n) as f64 / 2.0);
        if potential >= self.best_score {
            return true;
        }
        !cutter.advance(graph)
    }
}
