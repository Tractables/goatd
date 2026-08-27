//! The hypergraph used at every level of the hierarchy, stored in both
//! directions at once.
//!
//! Coarsening walks vertex -> hyperedges -> pins to find match candidates and
//! refinement walks hyperedge -> pins to update gains, so both incidence
//! directions are materialized rather than derived on demand. Only the finest
//! level has all-ones weights: coarsening sums what it merges.

use crate::Error;

/// A hypergraph with optional hyperedge weights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hypergraph {
    pub(super) num_vertices: usize,
    /// Fine vertices collapsed into this one. Balance is measured in these
    /// units and cut cost in hyperedge-weight units; the two are not
    /// commensurate, which matters wherever a single quantity has to price both
    /// (see `refine_flow`).
    pub(super) vertex_weights: Vec<u32>,
    /// Default: all 1s when no weights given.
    pub(super) hyperedge_weights: Vec<u32>,
    /// `pins[hyperedge_offsets[e]..hyperedge_offsets[e + 1]]` is hyperedge
    /// `e`'s pin set.
    pub(super) hyperedge_offsets: Vec<u32>,
    pins: Vec<u32>,
    /// `incident_hyperedges[vertex_hyperedge_offsets[v]..
    /// vertex_hyperedge_offsets[v + 1]]` is vertex `v`'s incidence list.
    pub(super) vertex_hyperedge_offsets: Vec<u32>,
    incident_hyperedges: Vec<u32>,
}

impl Hypergraph {
    /// Build a hypergraph over vertices `0..num_vertices`.
    ///
    /// `hyperedge_weights`, when present, has one positive entry per
    /// hyperedge. Pins
    /// within each hyperedge must be unique. Singleton hyperedges are dropped;
    /// repeated hyperedges are merged by adding their weights.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, repeated, or out-of-range pins; a zero
    /// weight; a weight count mismatch; or counts and total weight outside the
    /// implementation's integer ranges.
    pub fn new(
        num_vertices: u32,
        hyperedges: &[Vec<u32>],
        hyperedge_weights: Option<&[u32]>,
    ) -> Result<Self, Error> {
        let num_vertices = num_vertices as usize;
        if hyperedges.len() > u32::MAX as usize {
            return Err(Error::InvalidInput(format!(
                "hyperedge count {} does not fit in u32",
                hyperedges.len()
            )));
        }
        let total_pins = hyperedges
            .iter()
            .try_fold(0usize, |total, pins| total.checked_add(pins.len()))
            .ok_or_else(|| Error::InvalidInput("hypergraph pin count overflows usize".into()))?;
        if total_pins > u32::MAX as usize {
            return Err(Error::InvalidInput(format!(
                "hypergraph pin count {total_pins} does not fit in u32"
            )));
        }
        if let Some(hyperedge_weights) = hyperedge_weights
            && hyperedge_weights.len() != hyperedges.len()
        {
            return Err(Error::InvalidInput(format!(
                "hypergraph has {} hyperedges but {} hyperedge weights",
                hyperedges.len(),
                hyperedge_weights.len()
            )));
        }
        let mut normalized = Vec::with_capacity(hyperedges.len());
        for (hyperedge, pins) in hyperedges.iter().enumerate() {
            if pins.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "hyperedge {hyperedge} has no pins"
                )));
            }
            let weight = hyperedge_weights.map_or(1, |weights| weights[hyperedge]);
            if weight == 0 {
                return Err(Error::InvalidInput(format!(
                    "hyperedge {hyperedge} has weight 0; weights must be positive"
                )));
            }
            let mut pins = pins.clone();
            pins.sort_unstable();
            if let Some(&vertex) = pins.iter().find(|&&vertex| vertex as usize >= num_vertices) {
                return Err(Error::InvalidInput(format!(
                    "hyperedge {hyperedge} contains vertex {vertex}, outside 0..{num_vertices}"
                )));
            }
            if let Some(vertex) = pins
                .windows(2)
                .find(|pair| pair[0] == pair[1])
                .map(|pair| pair[0])
            {
                return Err(Error::InvalidInput(format!(
                    "hyperedge {hyperedge} contains vertex {vertex} more than once"
                )));
            }
            if pins.len() < 2 {
                continue;
            }
            normalized.push((pins, weight));
        }
        normalized.sort_unstable_by(|left, right| left.0.cmp(&right.0));

        let mut canonical_hyperedges: Vec<Vec<u32>> = Vec::with_capacity(normalized.len());
        let mut canonical_weights: Vec<u32> = Vec::with_capacity(normalized.len());
        for (pins, weight) in normalized {
            if canonical_hyperedges
                .last()
                .is_some_and(|last| last.as_slice() == pins.as_slice())
            {
                let combined = canonical_weights
                    .last_mut()
                    .expect("a repeated hyperedge has a previous weight");
                *combined = combined.checked_add(weight).ok_or_else(|| {
                    Error::InvalidInput("merged hyperedge weight does not fit in u32".into())
                })?;
            } else {
                canonical_hyperedges.push(pins);
                canonical_weights.push(weight);
            }
        }
        let total_hyperedge_weight: u64 = canonical_weights
            .iter()
            .map(|&weight| u64::from(weight))
            .sum();
        if total_hyperedge_weight > u32::MAX as u64 {
            return Err(Error::InvalidInput(format!(
                "total hyperedge weight {total_hyperedge_weight} does not fit in u32"
            )));
        }

        Ok(Self::from_hyperedges(
            num_vertices,
            &canonical_hyperedges,
            Some(&canonical_weights),
        ))
    }

    /// Number of vertices in the hypergraph.
    pub fn num_vertices(&self) -> u32 {
        self.num_vertices as u32
    }

    /// Number of distinct non-singleton hyperedges after canonicalization.
    pub fn num_hyperedges(&self) -> usize {
        self.hyperedge_offsets.len() - 1
    }

    /// Canonical non-singleton hyperedges and their positive weights.
    ///
    /// Pins within an edge are sorted. Equal input hyperedges appear once with
    /// their weights added.
    pub fn hyperedges(&self) -> impl ExactSizeIterator<Item = (&[u32], u32)> + '_ {
        (0..self.num_hyperedges()).map(|hyperedge| {
            (
                self.hyperedge_pins_unmetered(hyperedge),
                self.hyperedge_weights[hyperedge],
            )
        })
    }

    /// Pins are stored exactly as given: each hyperedge must already be
    /// deduplicated, since `greedy_growing` and the FM passes compare a
    /// running count of pins on one side against the hyperedge's pin count,
    /// and a repeated pin makes a fully-contained hyperedge never reach its own
    /// pin count. `weights` is parallel to `hyperedges`.
    pub(super) fn from_hyperedges(
        num_vertices: usize,
        hyperedges: &[Vec<u32>],
        weights: Option<&[u32]>,
    ) -> Self {
        let mut hyperedge_offsets = Vec::with_capacity(hyperedges.len() + 1);
        let mut pins = Vec::new();
        hyperedge_offsets.push(0);
        for hyperedge in hyperedges {
            pins.extend_from_slice(hyperedge);
            hyperedge_offsets.push(pins.len() as u32);
        }

        let hyperedge_weights = match weights {
            Some(w) => w.to_vec(),
            None => vec![1; hyperedges.len()],
        };

        let mut v_to_he: Vec<Vec<u32>> = vec![Vec::new(); num_vertices];
        for (hei, he) in hyperedges.iter().enumerate() {
            for &v in he {
                v_to_he[v as usize].push(hei as u32);
            }
        }
        let mut vertex_hyperedge_offsets = Vec::with_capacity(num_vertices + 1);
        let mut incident_hyperedges = Vec::new();
        vertex_hyperedge_offsets.push(0);
        for list in &v_to_he {
            incident_hyperedges.extend_from_slice(list);
            vertex_hyperedge_offsets.push(incident_hyperedges.len() as u32);
        }

        Hypergraph {
            num_vertices,
            vertex_weights: vec![1; num_vertices],
            hyperedge_weights,
            hyperedge_offsets,
            pins,
            vertex_hyperedge_offsets,
            incident_hyperedges,
        }
    }

    /// One of the two ways into the pin structure, and therefore one of the two
    /// places this family's work is charged.
    ///
    /// Every coarsening, partitioning and refinement loop here reaches its data
    /// through this accessor or through [`Hypergraph::vertex_hyperedges`], so
    /// charging the length of the slice each hands back prices all of them from
    /// one place rather than from a charge in every loop. A caller that takes a
    /// slice only to read its length is charged for pins it never visits, which
    /// is the safe direction for a clock whose job is to stop a build before a
    /// wall does.
    pub(super) fn charged_hyperedge_pins(&self, hei: usize) -> &[u32] {
        let pins = self.hyperedge_pins_unmetered(hei);
        crate::meter::charge(pins.len() as u64);
        pins
    }

    fn hyperedge_pins_unmetered(&self, hei: usize) -> &[u32] {
        let start = self.hyperedge_offsets[hei] as usize;
        let end = self.hyperedge_offsets[hei + 1] as usize;
        &self.pins[start..end]
    }

    /// How many pins each hyperedge has on each side of `part`.
    ///
    /// The whole hypergraph gain model is a statement about these two numbers
    /// reaching 0, 1 or 2, so every refiner starts by building them and then
    /// maintains them across its own moves.
    pub(super) fn pin_counts(&self, part: &[u8]) -> Vec<[u32; 2]> {
        let mut counts = vec![[0u32; 2]; self.num_hyperedges()];
        for (hei, he_counts) in counts.iter_mut().enumerate() {
            for &v in self.charged_hyperedge_pins(hei) {
                he_counts[part[v as usize] as usize] += 1;
            }
        }
        counts
    }

    /// The other charged incidence direction.
    pub(super) fn vertex_hyperedges(&self, v: usize) -> &[u32] {
        let start = self.vertex_hyperedge_offsets[v] as usize;
        let end = self.vertex_hyperedge_offsets[v + 1] as usize;
        crate::meter::charge((end - start) as u64);
        &self.incident_hyperedges[start..end]
    }
}
