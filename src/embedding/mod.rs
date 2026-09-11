//! Coordinates for the vertices of a graph.
//!
//! [`Embedding::compute`] moves every vertex halfway toward the mean of its
//! neighbours and then whitens the cloud: recentre it, rotate it onto the
//! eigenvectors of its covariance, and rescale every axis to unit standard
//! deviation. Without the whitening the repeated averaging collapses the whole
//! graph onto one point. With it the averaging is subspace iteration on the
//! lazy random walk, so the axes settle on the walk's slowest modes and the
//! leading one approximates a Fiedler vector.
//!
//! The distance of a vertex from the centre — [`Embedding::eccentricity`] —
//! says how peripheral it is. [`Embedding::rank_weights`] turns that order
//! into the tie weights a sampled elimination draws with.

use std::fmt;

use crate::Graph;
use crate::prefetch::{prefetch, prefetching};
use crate::rng::{SEED_OFFSET, Xorshift64};

#[cfg(test)]
mod tests;

/// The largest dimension an embedding can have.
pub const MAX_DIM: usize = 8;

/// Rounds [`Embedding::compute`] runs when a caller has no other bound.
///
/// The mode a round has to suppress decays by a factor close to 1 on a large
/// sparse graph — on a 20×20 grid, 0.9969 per round — so a cloud takes on the
/// order of a thousand rounds to settle.
pub const DEFAULT_MAX_ROUNDS: usize = 1_000;

/// Consecutive settled rounds before [`Embedding::compute`] stops.
pub const DEFAULT_PATIENCE: usize = 5;

/// Change in a squared eccentricity or a squared edge length, in whitened
/// units, at or below which a round counts as settled.
pub const DEFAULT_TOLERANCE: f32 = 1e-4;

/// Sweeps of the cyclic Jacobi rotation used to diagonalise a covariance.
const JACOBI_SWEEPS: usize = 12;

/// Off-diagonal mass at or below which the Jacobi sweeps stop.
const JACOBI_TOLERANCE: f64 = 1e-18;

/// Standard deviation at or below which an axis counts as flat and is
/// jittered instead of rescaled.
const FLAT_AXIS_DEVIATION: f64 = 1e-6;

/// How many edges ahead of the averaging walk a neighbour's row is
/// prefetched. Far enough to cover a miss at the rate the walk consumes
/// edges, short enough that the line is still there when the walk reaches it.
const PREFETCH_DISTANCE: usize = 8;

/// Odd constant [`random_weights`] adds to its seed, so its stream is not the
/// one a placement at the same seed draws from. Changing it reshuffles every
/// random weight vector.
const RANDOM_WEIGHT_OFFSET: u64 = 0x2545_F491_4F6C_DD1D;

/// Where a run of rounds stops.
#[derive(Clone, Copy)]
struct Budget {
    max_rounds: usize,
    patience: usize,
    tolerance: f32,
}

/// One point per vertex of a graph, in `dim` dimensions.
///
/// Row `v` of the cloud is `coord(v)`. Coordinates are whitened, so every axis
/// has zero mean and unit standard deviation over the vertices that have a
/// neighbour, and distances are comparable across graphs.
#[derive(Clone)]
pub struct Embedding {
    dim: usize,
    /// `dim` values per vertex, vertex `v`'s row starting at `v * dim`.
    coords: Vec<f32>,
}

impl fmt::Debug for Embedding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Embedding")
            .field("dim", &self.dim)
            .field("num_vertices", &self.num_vertices())
            .finish()
    }
}

/// Two embeddings are equal when they have the same dimension and identical
/// coordinate bits.
impl PartialEq for Embedding {
    fn eq(&self, other: &Self) -> bool {
        self.dim == other.dim
            && self.coords.len() == other.coords.len()
            && self
                .coords
                .iter()
                .zip(&other.coords)
                .all(|(left, right)| left.to_bits() == right.to_bits())
    }
}

impl Eq for Embedding {}

impl Embedding {
    /// Place the vertices of `graph` by repeated neighbour averaging.
    ///
    /// Every round moves each vertex halfway toward the mean of its neighbours
    /// and whitens the cloud. The loop ends after `max_rounds` rounds, after
    /// `patience` consecutive rounds in which no squared eccentricity and no
    /// squared edge length changed by more than `tolerance`, or when `stop`
    /// returns true; `stop` is polled once per round, so a caller can pass a
    /// deadline check. The coordinates of the last round are returned.
    ///
    /// `dim` is clamped to `1..=`[`MAX_DIM`]. `seed` selects the stream the
    /// starting positions are drawn from. A vertex with no neighbour is never
    /// averaged and takes no part in the whitening statistics; the whitening
    /// the rest of the cloud decides on still moves it.
    ///
    /// The work is charged to the construction meter, so a budget stated in
    /// charged work covers the embedding as it covers everything else.
    pub fn compute(
        graph: &Graph,
        dim: usize,
        seed: u64,
        max_rounds: usize,
        patience: usize,
        tolerance: f32,
        stop: &mut dyn FnMut() -> bool,
    ) -> Self {
        Self::compute_on(
            &Adjacency::of(graph),
            dim,
            seed,
            max_rounds,
            patience,
            tolerance,
            stop,
        )
    }

    /// [`Embedding::compute`] on an adjacency the caller holds.
    ///
    /// A caller that places several embeddings of one graph builds the
    /// adjacency once and passes it to each of them. Every embedding charges
    /// the meter for building it either way, so the work a budget stated in
    /// charged work pays for does not depend on the sharing.
    pub(crate) fn compute_on(
        adjacency: &Adjacency,
        dim: usize,
        seed: u64,
        max_rounds: usize,
        patience: usize,
        tolerance: f32,
        stop: &mut dyn FnMut() -> bool,
    ) -> Self {
        let dim = dim.clamp(1, MAX_DIM);
        let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
        let mut coords = vec![0.0f32; adjacency.vertex_count() * dim];
        for slot in &mut coords {
            *slot = unit_interval(&mut rng);
        }
        run_rounds(
            &mut coords,
            dim,
            adjacency,
            &mut rng,
            Budget {
                max_rounds,
                patience,
                tolerance,
            },
            stop,
        );
        Embedding { dim, coords }
    }

    /// How many coordinates each vertex has.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// How many vertices the cloud covers.
    pub fn num_vertices(&self) -> usize {
        self.coords.len() / self.dim
    }

    /// Vertex `vertex`'s coordinates.
    ///
    /// # Panics
    ///
    /// Panics when `vertex` is outside `0..num_vertices()`.
    pub fn coord(&self, vertex: u32) -> &[f32] {
        let start = vertex as usize * self.dim;
        &self.coords[start..start + self.dim]
    }

    /// How far `vertex` sits from the centre of the cloud.
    pub fn eccentricity(&self, vertex: u32) -> f32 {
        self.coord(vertex)
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt()
    }

    /// One tie weight per vertex, spread over the whole `u32` range in
    /// eccentricity order: 0 for the most peripheral vertex when
    /// `peripheral_first`, 0 for the most central otherwise, ties broken by
    /// vertex id.
    ///
    /// A sampled elimination order draws a tied vertex with mass
    /// `u32::MAX - weight + 1`, so the most peripheral vertex of a tie set is
    /// drawn, and eliminated, first. The ranks are spread rather than used
    /// literally: ranks of `0..count` differ in mass by a few parts in 2^32
    /// and would draw almost uniformly.
    pub fn rank_weights(&self, peripheral_first: bool) -> Vec<u32> {
        let count = self.num_vertices();
        // The eccentricity and the vertex id packed into one key, so that the
        // sort compares keys in place instead of loading two eccentricities
        // from wherever the ids point on every comparison. Keys are distinct,
        // so the unstable sort orders them the way the comparison did.
        let mut order: Vec<u64> = (0..count as u32)
            .map(|v| {
                let mut key = total_order_key(self.eccentricity(v));
                if peripheral_first {
                    key = !key;
                }
                (u64::from(key) << 32) | u64::from(v)
            })
            .collect();
        order.sort_unstable();
        let mut weights = vec![0u32; count];
        if count < 2 {
            return weights;
        }
        for (rank, &key) in order.iter().enumerate() {
            weights[key as u32 as usize] =
                (rank as u64 * u64::from(u32::MAX) / (count as u64 - 1)) as u32;
        }
        weights
    }
}

/// A key that orders `u32` as [`f32::total_cmp`] orders the values: the bits
/// of the value, with the magnitude bits of a negative flipped so that it
/// sorts below every positive, and the sign flipped so that unsigned order
/// runs from the most negative to the most positive.
fn total_order_key(value: f32) -> u32 {
    let bits = value.to_bits() as i32;
    let signed = bits ^ (((bits >> 31) as u32) >> 1) as i32;
    (signed as u32) ^ (1 << 31)
}

/// `count` tie weights drawn uniformly at random from `seed`.
///
/// The control for the tie weights that mean something: it perturbs the tie
/// sets by as much as they do while carrying no information about the graph, so
/// a gain that survives here came from the perturbation and not the signal.
///
/// The stream starts at its own offset, so a run that also places an embedding
/// from the same seed draws different numbers here.
pub(crate) fn random_weights(count: usize, seed: u64) -> Vec<u32> {
    let mut rng = Xorshift64::from_state(seed.wrapping_add(RANDOM_WEIGHT_OFFSET));
    (0..count).map(|_| rng.next_u32()).collect()
}

/// Move every vertex halfway toward the mean of its neighbours and whiten the
/// cloud, until the budget or `stop` ends it.
///
/// The dimension becomes a constant here, once per `compute`, so that every
/// inner loop of a round is a fixed number of lanes wide.
fn run_rounds(
    coords: &mut Vec<f32>,
    dim: usize,
    adjacency: &Adjacency,
    rng: &mut Xorshift64,
    budget: Budget,
    stop: &mut dyn FnMut() -> bool,
) {
    const {
        assert!(
            MAX_DIM == 8,
            "the dispatch below needs one arm per dimension"
        )
    };
    match dim {
        1 => run_rounds_dim::<1>(coords, adjacency, rng, budget, stop),
        2 => run_rounds_dim::<2>(coords, adjacency, rng, budget, stop),
        3 => run_rounds_dim::<3>(coords, adjacency, rng, budget, stop),
        4 => run_rounds_dim::<4>(coords, adjacency, rng, budget, stop),
        5 => run_rounds_dim::<5>(coords, adjacency, rng, budget, stop),
        6 => run_rounds_dim::<6>(coords, adjacency, rng, budget, stop),
        7 => run_rounds_dim::<7>(coords, adjacency, rng, budget, stop),
        8 => run_rounds_dim::<8>(coords, adjacency, rng, budget, stop),
        _ => unreachable!("the dimension is clamped to 1..=MAX_DIM"),
    }
}

/// [`run_rounds`] at a known dimension.
fn run_rounds_dim<const D: usize>(
    coords: &mut Vec<f32>,
    adjacency: &Adjacency,
    rng: &mut Xorshift64,
    budget: Budget,
    stop: &mut dyn FnMut() -> bool,
) {
    let Adjacency { starts, targets } = adjacency;
    let vertex_count = adjacency.vertex_count();
    // Building the adjacency, charged here whether this run built it or was
    // handed one, so that an embedding costs the meter the same either way.
    crate::meter::charge((vertex_count + targets.len()) as u64);
    // The whitening statistics ignore vertices with no neighbour: they never
    // move, so they would only add an isotropic cloud of starting positions to
    // the covariance.
    let moving: Vec<u32> = (0..vertex_count as u32)
        .filter(|&vertex| starts[vertex as usize + 1] > starts[vertex as usize])
        .collect();
    if moving.is_empty() {
        return;
    }

    // One unit per adjacency visit in the update, plus the per-vertex
    // covariance and rotation of the whitening.
    let round_units = (targets.len() + vertex_count * D * D) as u64;
    let patience = budget.patience.max(1);
    let ahead_of_walk = prefetching(vertex_count);
    let mut next = vec![0.0f32; vertex_count * D];

    let mut settled = 0usize;
    for _ in 0..budget.max_rounds {
        for vertex in 0..vertex_count {
            let (start, end) = (starts[vertex], starts[vertex + 1]);
            let base = vertex * D;
            if start == end {
                next[base..base + D].copy_from_slice(&coords[base..base + D]);
                continue;
            }
            let mut sums = [0.0f32; D];
            for (position, &neighbour) in targets[start..end].iter().enumerate() {
                // The rows arrive at scattered offsets, so ask for the one a
                // fixed number of edges further along the adjacency. The
                // lookahead runs past the end of this vertex's own row into
                // the rows the next vertices read, which is where the walk
                // goes next. It reads nothing, so the sums below are the ones
                // an unprefetched walk makes.
                if ahead_of_walk
                    && let Some(&ahead) = targets.get(start + position + PREFETCH_DISTANCE)
                {
                    prefetch(coords.as_slice(), ahead as usize * D);
                }
                // The neighbour's row is taken whole: the inner loop is a few
                // adds over an array the same length as `sums`, so neither the
                // trip count nor a bounds check reaches the hottest line of the
                // round.
                for (sum, value) in sums
                    .iter_mut()
                    .zip(row_at::<D>(coords, neighbour as usize * D))
                {
                    *sum += value;
                }
            }
            let degree = (end - start) as f32;
            let here = row_at::<D>(coords, base);
            for ((slot, value), sum) in next[base..base + D].iter_mut().zip(here).zip(sums.iter()) {
                *slot = 0.5 * (value + *sum / degree);
            }
        }
        // After the swap `next` holds the previous round's whitened cloud,
        // which is what the change below is measured against.
        std::mem::swap(coords, &mut next);
        whiten_dim::<D>(coords, &moving, rng);
        crate::meter::charge(round_units);

        // The leading axis settles long before the whole cloud does, so a
        // consumer that reads only one axis can stop much earlier than this.
        // `D` is a constant here, so the dispatch inside folds away.
        if is_settled(coords, &next, D, starts, targets, budget.tolerance) {
            settled += 1;
            if settled >= patience {
                break;
            }
        } else {
            settled = 0;
        }
        if stop() {
            break;
        }
    }
}

/// The row of `D` coordinates that starts at `base`.
#[inline(always)]
fn row_at<const D: usize>(coords: &[f32], base: usize) -> &[f32; D] {
    let row: &[f32] = &coords[base..base + D];
    row.try_into().expect("a row is D coordinates wide")
}

/// Run `body` on the row of every vertex the statistics are taken over.
///
/// When every vertex moves, `moving` is `0..vertex_count` in order, which is
/// the order the rows are stored in, so the rows are walked directly and the
/// gather through `moving` is skipped. Either way `body` sees the same rows in
/// the same order.
#[inline(always)]
fn for_moving_rows<const D: usize>(
    coords: &[f32],
    moving: &[u32],
    mut body: impl FnMut(&[f32; D]),
) {
    if moving.len() * D == coords.len() {
        for row in coords.as_chunks::<D>().0 {
            body(row);
        }
    } else {
        for &vertex in moving {
            body(row_at::<D>(coords, vertex as usize * D));
        }
    }
}

/// Compressed adjacency: `targets[starts[v]..starts[v + 1]]` are `v`'s
/// neighbours.
///
/// Building it is a pass over the edges and about as much memory as the
/// coordinates take, so a caller placing several embeddings of one graph
/// builds it once and hands it to each of them.
pub(crate) struct Adjacency {
    starts: Vec<usize>,
    targets: Vec<u32>,
}

impl Adjacency {
    /// The adjacency of `graph`.
    pub(crate) fn of(graph: &Graph) -> Self {
        let vertex_count = graph.num_vertices() as usize;
        let mut starts = vec![0usize; vertex_count + 1];
        for &(left, right) in graph.edges() {
            starts[left as usize + 1] += 1;
            starts[right as usize + 1] += 1;
        }
        for vertex in 0..vertex_count {
            starts[vertex + 1] += starts[vertex];
        }
        let mut cursor = starts[..vertex_count].to_vec();
        let mut targets = vec![0u32; graph.edges().len() * 2];
        for &(left, right) in graph.edges() {
            targets[cursor[left as usize]] = right;
            cursor[left as usize] += 1;
            targets[cursor[right as usize]] = left;
            cursor[right as usize] += 1;
        }
        Adjacency { starts, targets }
    }

    /// How many vertices the graph has.
    fn vertex_count(&self) -> usize {
        self.starts.len() - 1
    }
}

/// Whether two whitened clouds agree to `tolerance` in the quantities read
/// back out of one: the squared distance of a vertex from the centre, and the
/// squared length of an edge.
///
/// Whitening fixes the frame only up to a rotation — two axes with close
/// eigenvalues can come back swapped or flipped — so comparing coordinates
/// directly reports movement in a cloud whose geometry has stopped changing.
/// These two quantities are invariant under that rotation.
///
/// The comparison is strict, so a change that is not a number counts as no
/// change, which is what taking the maximum over the changes did.
fn is_settled(
    coords: &[f32],
    previous: &[f32],
    dim: usize,
    starts: &[usize],
    targets: &[u32],
    tolerance: f32,
) -> bool {
    match dim {
        1 => is_settled_dim::<1>(coords, previous, starts, targets, tolerance),
        2 => is_settled_dim::<2>(coords, previous, starts, targets, tolerance),
        3 => is_settled_dim::<3>(coords, previous, starts, targets, tolerance),
        4 => is_settled_dim::<4>(coords, previous, starts, targets, tolerance),
        5 => is_settled_dim::<5>(coords, previous, starts, targets, tolerance),
        6 => is_settled_dim::<6>(coords, previous, starts, targets, tolerance),
        7 => is_settled_dim::<7>(coords, previous, starts, targets, tolerance),
        8 => is_settled_dim::<8>(coords, previous, starts, targets, tolerance),
        _ => unreachable!("the dimension is clamped to 1..=MAX_DIM"),
    }
}

/// [`is_settled`] at a known dimension.
fn is_settled_dim<const D: usize>(
    coords: &[f32],
    previous: &[f32],
    starts: &[usize],
    targets: &[u32],
    tolerance: f32,
) -> bool {
    for (row, was) in coords
        .as_chunks::<D>()
        .0
        .iter()
        .zip(previous.as_chunks::<D>().0)
    {
        let mut now = 0.0f32;
        let mut before = 0.0f32;
        for (value, earlier) in row.iter().zip(was) {
            now += value * value;
            before += earlier * earlier;
        }
        if (now - before).abs() > tolerance {
            return false;
        }
    }
    for vertex in 0..starts.len().saturating_sub(1) {
        let base = vertex * D;
        let row = row_at::<D>(coords, base);
        let was = row_at::<D>(previous, base);
        for &neighbour in &targets[starts[vertex]..starts[vertex + 1]] {
            // Every edge is stored from both ends; measure it once.
            if neighbour as usize <= vertex {
                continue;
            }
            let other = neighbour as usize * D;
            let other_row = row_at::<D>(coords, other);
            let other_was = row_at::<D>(previous, other);
            let mut now = 0.0f32;
            let mut before = 0.0f32;
            for (((value, other_value), earlier), other_earlier) in
                row.iter().zip(other_row).zip(was).zip(other_was)
            {
                let offset = value - other_value;
                now += offset * offset;
                let earlier = earlier - other_earlier;
                before += earlier * earlier;
            }
            if (now - before).abs() > tolerance {
                return false;
            }
        }
    }
    true
}

/// Recentre the cloud, rotate it onto the eigenvectors of its covariance, and
/// rescale every axis to unit standard deviation.
///
/// Axes come out in descending order of covariance eigenvalue. Averaging
/// contracts the fast modes of the random walk hardest, so the axis left with
/// the most spread is the graph's slowest mode: descending spread here is
/// ascending order of the graph's own spectrum, and the leading axis is the
/// Fiedler-like one.
///
/// `moving` carries the vertices the statistics are taken over. An axis with
/// no spread left is jittered from `rng` so that the rescaling has something
/// to divide by and repeated rounds cannot collapse the cloud.
fn whiten_dim<const D: usize>(coords: &mut [f32], moving: &[u32], rng: &mut Xorshift64) {
    let count = moving.len() as f64;

    let mut centre = [0.0f64; D];
    for_moving_rows::<D>(coords, moving, |row| {
        for (value, coordinate) in centre.iter_mut().zip(row) {
            *value += f64::from(*coordinate);
        }
    });
    for value in &mut centre {
        *value /= count;
    }
    // The same shift is taken off every row, so it is rounded to `f32` once
    // instead of once per row.
    let mut shift = [0.0f32; D];
    for (slot, value) in shift.iter_mut().zip(&centre) {
        *slot = *value as f32;
    }
    let mut covariance = [[0.0f64; D]; D];
    if moving.len() * D == coords.len() {
        // Every vertex takes part in the statistics, so the shift comes off
        // the same rows, in the same order, that the covariance sums over.
        // The row's new coordinate is stored first and read back from the
        // cloud, so the covariance still sums the `f32` values the cloud
        // holds.
        for row in coords.as_chunks_mut::<D>().0 {
            for (value, taken) in row.iter_mut().zip(&shift) {
                *value -= *taken;
            }
            accumulate_covariance(&mut covariance, row);
        }
    } else {
        for row in coords.as_chunks_mut::<D>().0 {
            for (value, taken) in row.iter_mut().zip(&shift) {
                *value -= *taken;
            }
        }
        for_moving_rows::<D>(coords, moving, |row| {
            accumulate_covariance(&mut covariance, row);
        });
    }
    for i in 0..D {
        let (upper, lower) = covariance.split_at_mut(i + 1);
        let cells = &mut upper[i];
        for cell in &mut cells[i..] {
            *cell /= count;
        }
        for (below, &entry) in lower.iter_mut().zip(&cells[i + 1..]) {
            below[i] = entry;
        }
    }

    let mut vectors = [[0.0f64; D]; D];
    jacobi(covariance.as_flattened_mut(), vectors.as_flattened_mut(), D);
    let mut order = [0usize; D];
    for (axis, slot) in order.iter_mut().enumerate() {
        *slot = axis;
    }
    order.sort_by(|&left, &right| {
        covariance[right][right]
            .total_cmp(&covariance[left][left])
            .then(left.cmp(&right))
    });

    // The rotation reads the eigenvector matrix by column, once per row of the
    // cloud. Permuting it into the axis order here lets the row loop run `i`
    // outside and the axis inside: every axis still sums over `i` ascending,
    // and a coordinate is widened to `f64` once instead of once per axis.
    let mut permuted = [[0.0f64; D]; D];
    for (i, weights) in permuted.iter_mut().enumerate() {
        for (weight, &column) in weights.iter_mut().zip(&order) {
            *weight = vectors[i][column];
        }
    }
    for row in coords.as_chunks_mut::<D>().0 {
        let mut rotated = [0.0f64; D];
        for (i, weights) in permuted.iter().enumerate() {
            let coordinate = f64::from(row[i]);
            for (projection, weight) in rotated.iter_mut().zip(weights) {
                *projection += coordinate * weight;
            }
        }
        for (value, projection) in row.iter_mut().zip(&rotated) {
            *value = *projection as f32;
        }
    }

    // Every axis is measured from its own column and rescaled in it, and
    // nothing an axis does reaches another one, so the columns are measured
    // together and rescaled together instead of a strided pass over the whole
    // cloud per axis. A jittered axis is remeasured where it is jittered, so
    // the generator is still drawn from in axis order.
    let (mut means, mut deviations) = axis_spreads_dim::<D>(coords, moving);
    let mut scales = [1.0f64; D];
    for axis in 0..D {
        if deviations[axis] <= FLAT_AXIS_DEVIATION {
            // A flat axis carries no direction to rescale. Spread it from the
            // generator so the cloud keeps its dimension in the next round.
            for row in coords.as_chunks_mut::<D>().0 {
                row[axis] += unit_interval(rng) - 0.5;
            }
            let (jittered, spread) = axis_spreads_dim::<D>(coords, moving);
            (means[axis], deviations[axis]) = (jittered[axis], spread[axis]);
        }
        if deviations[axis] > FLAT_AXIS_DEVIATION {
            scales[axis] = 1.0 / deviations[axis];
        }
    }
    for row in coords.as_chunks_mut::<D>().0 {
        for ((value, mean), scale) in row.iter_mut().zip(&means).zip(&scales) {
            *value = ((f64::from(*value) - mean) * scale) as f32;
        }
    }
}

/// Add one recentred row's outer product to the upper triangle of
/// `covariance`.
///
/// The row is widened once and every product is taken from the widened
/// values, so a coordinate is converted once however many cells it reaches.
#[inline(always)]
fn accumulate_covariance<const D: usize>(covariance: &mut [[f64; D]; D], row: &[f32; D]) {
    let mut wide = [0.0f64; D];
    for (slot, value) in wide.iter_mut().zip(row) {
        *slot = f64::from(*value);
    }
    for (i, cells) in covariance.iter_mut().enumerate() {
        let value = wide[i];
        for (cell, other) in cells[i..].iter_mut().zip(&wide[i..]) {
            *cell += value * *other;
        }
    }
}

/// The mean and standard deviation of every axis over `moving`.
///
/// One pass for the means and one for the deviations, each row read across
/// all of its axes. An axis's sum runs over `moving` in the order it is
/// stored in, so the two passes give an axis the value a pass over that
/// column alone gives it.
fn axis_spreads_dim<const D: usize>(coords: &[f32], moving: &[u32]) -> ([f64; D], [f64; D]) {
    let count = moving.len() as f64;
    let mut means = [0.0f64; D];
    for_moving_rows::<D>(coords, moving, |row| {
        for (mean, value) in means.iter_mut().zip(row) {
            *mean += f64::from(*value);
        }
    });
    for mean in &mut means {
        *mean /= count;
    }
    let mut deviations = [0.0f64; D];
    for_moving_rows::<D>(coords, moving, |row| {
        for ((variance, mean), value) in deviations.iter_mut().zip(&means).zip(row) {
            let offset = f64::from(*value) - *mean;
            *variance += offset * offset;
        }
    });
    for variance in &mut deviations {
        *variance = (*variance / count).sqrt();
    }
    (means, deviations)
}

/// Cyclic Jacobi diagonalisation of the symmetric `dim`×`dim` `matrix`.
///
/// On return the eigenvalues are `matrix`'s diagonal and the eigenvectors are
/// the columns of `vectors`. Both are addressed with stride `dim`.
fn jacobi(matrix: &mut [f64], vectors: &mut [f64], dim: usize) {
    for i in 0..dim {
        for j in 0..dim {
            vectors[i * dim + j] = if i == j { 1.0 } else { 0.0 };
        }
    }
    for _ in 0..JACOBI_SWEEPS {
        let mut off_diagonal = 0.0f64;
        for p in 0..dim {
            for q in (p + 1)..dim {
                off_diagonal += matrix[p * dim + q] * matrix[p * dim + q];
            }
        }
        if off_diagonal <= JACOBI_TOLERANCE {
            break;
        }
        for p in 0..dim {
            for q in (p + 1)..dim {
                let pivot = matrix[p * dim + q];
                if pivot == 0.0 {
                    continue;
                }
                let theta = (matrix[q * dim + q] - matrix[p * dim + p]) / (2.0 * pivot);
                let tangent = if theta >= 0.0 {
                    1.0 / (theta + (theta * theta + 1.0).sqrt())
                } else {
                    -1.0 / (-theta + (theta * theta + 1.0).sqrt())
                };
                let cosine = 1.0 / (tangent * tangent + 1.0).sqrt();
                let sine = tangent * cosine;
                for k in 0..dim {
                    let (left, right) = (matrix[k * dim + p], matrix[k * dim + q]);
                    matrix[k * dim + p] = cosine * left - sine * right;
                    matrix[k * dim + q] = sine * left + cosine * right;
                }
                for k in 0..dim {
                    let (left, right) = (matrix[p * dim + k], matrix[q * dim + k]);
                    matrix[p * dim + k] = cosine * left - sine * right;
                    matrix[q * dim + k] = sine * left + cosine * right;
                }
                for k in 0..dim {
                    let (left, right) = (vectors[k * dim + p], vectors[k * dim + q]);
                    vectors[k * dim + p] = cosine * left - sine * right;
                    vectors[k * dim + q] = sine * left + cosine * right;
                }
            }
        }
    }
}

/// A value in `[0, 1)` with 24 bits of precision.
fn unit_interval(rng: &mut Xorshift64) -> f32 {
    const SCALE: f32 = 1.0 / (1u32 << 24) as f32;
    ((rng.next_u64() >> 40) as u32) as f32 * SCALE
}
