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
        let dim = dim.clamp(1, MAX_DIM);
        let mut rng = Xorshift64::from_state(seed.wrapping_add(SEED_OFFSET));
        let mut coords = vec![0.0f32; graph.num_vertices() as usize * dim];
        for slot in &mut coords {
            *slot = unit_interval(&mut rng);
        }
        run_rounds(
            &mut coords,
            dim,
            graph,
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
        let mut order: Vec<u32> = (0..count as u32).collect();
        order.sort_by(|&left, &right| {
            let (a, b) = (self.eccentricity(left), self.eccentricity(right));
            let by_eccentricity = if peripheral_first {
                b.total_cmp(&a)
            } else {
                a.total_cmp(&b)
            };
            by_eccentricity.then(left.cmp(&right))
        });
        let mut weights = vec![0u32; count];
        if count < 2 {
            return weights;
        }
        for (rank, &vertex) in order.iter().enumerate() {
            weights[vertex as usize] =
                (rank as u64 * u64::from(u32::MAX) / (count as u64 - 1)) as u32;
        }
        weights
    }
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
fn run_rounds(
    coords: &mut Vec<f32>,
    dim: usize,
    graph: &Graph,
    rng: &mut Xorshift64,
    budget: Budget,
    stop: &mut dyn FnMut() -> bool,
) {
    let vertex_count = graph.num_vertices() as usize;
    let (starts, targets) = adjacency(graph);
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
    let round_units = (targets.len() + vertex_count * dim * dim) as u64;
    let patience = budget.patience.max(1);
    let mut next = vec![0.0f32; vertex_count * dim];
    let mut settled = 0usize;
    let mut sums = [0.0f32; MAX_DIM];

    for _ in 0..budget.max_rounds {
        for vertex in 0..vertex_count {
            let (start, end) = (starts[vertex], starts[vertex + 1]);
            let base = vertex * dim;
            if start == end {
                next[base..base + dim].copy_from_slice(&coords[base..base + dim]);
                continue;
            }
            sums[..dim].fill(0.0);
            for &neighbour in &targets[start..end] {
                let row = neighbour as usize * dim;
                for (axis, sum) in sums[..dim].iter_mut().enumerate() {
                    *sum += coords[row + axis];
                }
            }
            let degree = (end - start) as f32;
            for axis in 0..dim {
                next[base + axis] = 0.5 * (coords[base + axis] + sums[axis] / degree);
            }
        }
        // After the swap `next` holds the previous round's whitened cloud,
        // which is what the change below is measured against.
        std::mem::swap(coords, &mut next);
        whiten(coords, dim, &moving, rng);
        crate::meter::charge(round_units);

        // The leading axis settles long before the whole cloud does, so a
        // consumer that reads only one axis can stop much earlier than this.
        if largest_invariant_change(coords, &next, dim, &starts, &targets) <= budget.tolerance {
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

/// Compressed adjacency: `targets[starts[v]..starts[v + 1]]` are `v`'s
/// neighbours.
fn adjacency(graph: &Graph) -> (Vec<usize>, Vec<u32>) {
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
    crate::meter::charge((vertex_count + targets.len()) as u64);
    (starts, targets)
}

/// The largest change between two whitened clouds in the quantities read back
/// out of one: the squared distance of a vertex from the centre, and the
/// squared length of an edge.
///
/// Whitening fixes the frame only up to a rotation — two axes with close
/// eigenvalues can come back swapped or flipped — so comparing coordinates
/// directly reports movement in a cloud whose geometry has stopped changing.
/// These two quantities are invariant under that rotation.
fn largest_invariant_change(
    coords: &[f32],
    previous: &[f32],
    dim: usize,
    starts: &[usize],
    targets: &[u32],
) -> f32 {
    let mut largest = 0.0f32;
    for (row, was) in coords.chunks_exact(dim).zip(previous.chunks_exact(dim)) {
        let mut now = 0.0f32;
        let mut before = 0.0f32;
        for (value, earlier) in row.iter().zip(was) {
            now += value * value;
            before += earlier * earlier;
        }
        largest = largest.max((now - before).abs());
    }
    for vertex in 0..starts.len().saturating_sub(1) {
        let base = vertex * dim;
        for &neighbour in &targets[starts[vertex]..starts[vertex + 1]] {
            // Every edge is stored from both ends; measure it once.
            if neighbour as usize <= vertex {
                continue;
            }
            let other = neighbour as usize * dim;
            let mut now = 0.0f32;
            let mut before = 0.0f32;
            for axis in 0..dim {
                let offset = coords[base + axis] - coords[other + axis];
                now += offset * offset;
                let earlier = previous[base + axis] - previous[other + axis];
                before += earlier * earlier;
            }
            largest = largest.max((now - before).abs());
        }
    }
    largest
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
fn whiten(coords: &mut [f32], dim: usize, moving: &[u32], rng: &mut Xorshift64) {
    let count = moving.len() as f64;

    let mut centre = [0.0f64; MAX_DIM];
    for &vertex in moving {
        let row = vertex as usize * dim;
        for (axis, value) in centre[..dim].iter_mut().enumerate() {
            *value += f64::from(coords[row + axis]);
        }
    }
    for value in &mut centre[..dim] {
        *value /= count;
    }
    for row in coords.chunks_exact_mut(dim) {
        for (axis, value) in row.iter_mut().enumerate() {
            *value -= centre[axis] as f32;
        }
    }

    let mut covariance = [0.0f64; MAX_DIM * MAX_DIM];
    for &vertex in moving {
        let row = vertex as usize * dim;
        for i in 0..dim {
            let value = f64::from(coords[row + i]);
            for j in i..dim {
                covariance[i * dim + j] += value * f64::from(coords[row + j]);
            }
        }
    }
    for i in 0..dim {
        for j in i..dim {
            let entry = covariance[i * dim + j] / count;
            covariance[i * dim + j] = entry;
            covariance[j * dim + i] = entry;
        }
    }

    let mut vectors = [0.0f64; MAX_DIM * MAX_DIM];
    jacobi(&mut covariance, &mut vectors, dim);
    let mut order = [0usize; MAX_DIM];
    for (axis, slot) in order[..dim].iter_mut().enumerate() {
        *slot = axis;
    }
    order[..dim].sort_by(|&left, &right| {
        covariance[right * dim + right]
            .total_cmp(&covariance[left * dim + left])
            .then(left.cmp(&right))
    });

    let mut rotated = [0.0f64; MAX_DIM];
    for row in coords.chunks_exact_mut(dim) {
        for (axis, value) in rotated[..dim].iter_mut().enumerate() {
            let column = order[axis];
            let mut projection = 0.0f64;
            for (i, coordinate) in row.iter().enumerate() {
                projection += f64::from(*coordinate) * vectors[i * dim + column];
            }
            *value = projection;
        }
        for (axis, value) in row.iter_mut().enumerate() {
            *value = rotated[axis] as f32;
        }
    }

    for axis in 0..dim {
        if axis_spread(coords, dim, moving, axis).1 > FLAT_AXIS_DEVIATION {
            continue;
        }
        // A flat axis carries no direction to rescale. Spread it from the
        // generator so the cloud keeps its dimension in the next round.
        for row in coords.chunks_exact_mut(dim) {
            row[axis] += unit_interval(rng) - 0.5;
        }
    }
    for axis in 0..dim {
        let (mean, deviation) = axis_spread(coords, dim, moving, axis);
        let scale = if deviation > FLAT_AXIS_DEVIATION {
            1.0 / deviation
        } else {
            1.0
        };
        for row in coords.chunks_exact_mut(dim) {
            row[axis] = ((f64::from(row[axis]) - mean) * scale) as f32;
        }
    }
}

/// The mean and standard deviation of one axis over `moving`.
fn axis_spread(coords: &[f32], dim: usize, moving: &[u32], axis: usize) -> (f64, f64) {
    let count = moving.len() as f64;
    let mut mean = 0.0f64;
    for &vertex in moving {
        mean += f64::from(coords[vertex as usize * dim + axis]);
    }
    mean /= count;
    let mut variance = 0.0f64;
    for &vertex in moving {
        let offset = f64::from(coords[vertex as usize * dim + axis]) - mean;
        variance += offset * offset;
    }
    (mean, (variance / count).sqrt())
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
