//! Vertex sets as words, and the graph as one row of words per vertex.
//!
//! The programme of [`super`] compares and combines vertex sets far more often
//! than it walks them: every bag it takes in splits the graph, every component
//! that comes out is looked up in a map, and every cap is tested for lying
//! between a separator and a block. Sorted vertex lists make each of those a
//! pass over as many ids as the set holds; words make them a pass over a
//! sixty-fourth of the graph.
//!
//! The rows cost `n²/8` bytes, which is why [`Adjacency::of`] refuses a graph
//! whose rows would be larger than [`MAX_ADJACENCY_BYTES`]. The stage this
//! serves runs on graphs of a couple of thousand vertices, so the refusal is
//! the answer for a graph it was never going to search anyway.

/// The rows a graph may take before it is refused: 64 MiB, which is a graph of
/// about 23,000 vertices.
const MAX_ADJACENCY_BYTES: usize = 64 << 20;

/// A set of vertex ids under a fixed capacity, held as words.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct VertexSet {
    words: Box<[u64]>,
}

impl VertexSet {
    /// The empty set over `words` words, that is over `64 * words` vertices.
    pub(crate) fn empty(words: usize) -> Self {
        Self {
            words: vec![0; words].into_boxed_slice(),
        }
    }

    /// The set holding `vertices`, all of which must be under the capacity.
    pub(crate) fn of(words: usize, vertices: &[u32]) -> Self {
        let mut set = Self::empty(words);
        for &vertex in vertices {
            set.insert(vertex);
        }
        set
    }

    /// Add `vertex`.
    pub(crate) fn insert(&mut self, vertex: u32) {
        self.words[vertex as usize / 64] |= 1 << (vertex as usize % 64);
    }

    /// Whether `vertex` is in the set.
    pub(crate) fn contains(&self, vertex: u32) -> bool {
        self.words[vertex as usize / 64] & (1 << (vertex as usize % 64)) != 0
    }

    /// How many vertices the set holds.
    pub(crate) fn len(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// Whether the set holds nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.words.iter().all(|&word| word == 0)
    }

    /// The lowest vertex the set holds.
    pub(crate) fn first(&self) -> Option<u32> {
        self.words
            .iter()
            .enumerate()
            .find(|&(_, &word)| word != 0)
            .map(|(index, &word)| (index * 64 + word.trailing_zeros() as usize) as u32)
    }

    /// Whether every vertex of the set is in `other`.
    pub(crate) fn is_subset(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(&other.words)
            .all(|(&mine, &theirs)| mine & !theirs == 0)
    }

    /// Whether the two sets share a vertex.
    pub(crate) fn intersects(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(&other.words)
            .any(|(&mine, &theirs)| mine & theirs != 0)
    }

    /// Add every vertex the adjacency row `row` holds.
    pub(crate) fn union_row(&mut self, row: &[u64]) {
        for (word, &mask) in self.words.iter_mut().zip(row) {
            *word |= mask;
        }
    }

    /// Add every vertex of `other`.
    pub(crate) fn union_with(&mut self, other: &Self) {
        for (mine, &theirs) in self.words.iter_mut().zip(&other.words) {
            *mine |= theirs;
        }
    }

    /// Keep only the vertices that are also in `other`.
    pub(crate) fn intersect_with(&mut self, other: &Self) {
        for (mine, &theirs) in self.words.iter_mut().zip(&other.words) {
            *mine &= theirs;
        }
    }

    /// Drop every vertex of `other`.
    pub(crate) fn subtract(&mut self, other: &Self) {
        for (mine, &theirs) in self.words.iter_mut().zip(&other.words) {
            *mine &= !theirs;
        }
    }

    /// Copy `other` over this set, which must be as wide.
    pub(crate) fn copy_from(&mut self, other: &Self) {
        self.words.copy_from_slice(&other.words);
    }

    /// Drop every vertex.
    pub(crate) fn clear(&mut self) {
        self.words.fill(0);
    }

    /// The vertices in increasing order.
    pub(crate) fn iter(&self) -> Iter<'_> {
        Iter {
            words: &self.words,
            index: 0,
            current: self.words.first().copied().unwrap_or(0),
        }
    }

    /// The vertices in increasing order, as a list.
    pub(crate) fn to_vec(&self) -> Vec<u32> {
        self.iter().collect()
    }
}

/// The iterator of [`VertexSet::iter`].
pub(crate) struct Iter<'a> {
    words: &'a [u64],
    index: usize,
    current: u64,
}

impl Iterator for Iter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        while self.current == 0 {
            self.index += 1;
            self.current = *self.words.get(self.index)?;
        }
        let bit = self.current.trailing_zeros() as usize;
        self.current &= self.current - 1;
        Some((self.index * 64 + bit) as u32)
    }
}

/// The graph as one row of words per vertex, with the components of the graph
/// less a vertex set read off those rows.
pub(crate) struct Adjacency {
    words: usize,
    rows: Vec<u64>,
    /// Every vertex of the graph, so that "the rest of the graph" is one mask.
    all: VertexSet,
}

/// The components of `G − Ω`, each with the vertices of `Ω` on its border.
///
/// The two lists run together: `borders[i]` is `N(C)` for `components[i]`.
pub(crate) struct Split {
    pub(crate) components: Vec<VertexSet>,
    pub(crate) borders: Vec<VertexSet>,
}

impl Adjacency {
    /// The rows of `graph`, or `None` where they would be larger than
    /// [`MAX_ADJACENCY_BYTES`].
    pub(crate) fn of(graph: &crate::Graph) -> Option<Self> {
        let vertices = graph.num_vertices() as usize;
        let words = vertices.div_ceil(64).max(1);
        if words.checked_mul(vertices)?.checked_mul(8)? > MAX_ADJACENCY_BYTES {
            return None;
        }
        let mut rows = vec![0u64; words * vertices];
        for &(left, right) in graph.edges() {
            if left == right {
                continue;
            }
            rows[left as usize * words + right as usize / 64] |= 1 << (right as usize % 64);
            rows[right as usize * words + left as usize / 64] |= 1 << (left as usize % 64);
        }
        let mut all = VertexSet::empty(words);
        for vertex in 0..vertices as u32 {
            all.insert(vertex);
        }
        Some(Self { words, rows, all })
    }

    /// An empty set over this graph.
    pub(crate) fn empty_set(&self) -> VertexSet {
        VertexSet::empty(self.words)
    }

    /// The set holding `vertices`.
    pub(crate) fn set_of(&self, vertices: &[u32]) -> VertexSet {
        VertexSet::of(self.words, vertices)
    }

    /// The neighbours of `vertex`.
    pub(crate) fn row(&self, vertex: u32) -> &[u64] {
        let start = vertex as usize * self.words;
        &self.rows[start..start + self.words]
    }

    /// Whether `left` and `right` are adjacent.
    pub(crate) fn adjacent(&self, left: u32, right: u32) -> bool {
        self.row(left)[right as usize / 64] & (1 << (right as usize % 64)) != 0
    }

    /// The connected components of `G − removed`, each with its border.
    pub(crate) fn split(&self, removed: &VertexSet, scratch: &mut Scratch) -> Split {
        let mut components = Vec::new();
        let mut borders = Vec::new();
        scratch.left.copy_from(&self.all);
        scratch.left.subtract(removed);
        while let Some(start) = scratch.left.first() {
            let mut component = self.empty_set();
            let mut reach = self.empty_set();
            scratch.frontier.clear();
            scratch.frontier.insert(start);
            component.insert(start);
            scratch.left.subtract(&scratch.frontier);
            while !scratch.frontier.is_empty() {
                scratch.next.clear();
                for vertex in scratch.frontier.iter() {
                    scratch.next.union_row(self.row(vertex));
                }
                reach.union_with(&scratch.next);
                scratch.next.intersect_with(&scratch.left);
                component.union_with(&scratch.next);
                scratch.left.subtract(&scratch.next);
                std::mem::swap(&mut scratch.frontier, &mut scratch.next);
            }
            reach.intersect_with(removed);
            components.push(component);
            borders.push(reach);
        }
        Split {
            components,
            borders,
        }
    }
}

/// What the traversals and the tests reuse, so that a set is allocated once
/// rather than once per call.
pub(crate) struct Scratch {
    left: VertexSet,
    frontier: VertexSet,
    next: VertexSet,
    /// Two sets the callers borrow for a set they build and throw away.
    pub(crate) held: VertexSet,
    pub(crate) reach: VertexSet,
    /// Where a vertex sits in the list a caller is working with. Only the
    /// entries of that list are written, so it is never cleared.
    pub(crate) position: Vec<u32>,
    /// Room for a square of bits over a vertex list.
    pub(crate) square: Vec<u64>,
    /// Room for a row of bits over a vertex list.
    pub(crate) row: Vec<u64>,
}

impl Scratch {
    pub(crate) fn new(adjacency: &Adjacency) -> Self {
        Self {
            left: adjacency.empty_set(),
            frontier: adjacency.empty_set(),
            next: adjacency.empty_set(),
            held: adjacency.empty_set(),
            reach: adjacency.empty_set(),
            position: vec![0; adjacency.rows.len() / adjacency.words],
            square: Vec::new(),
            row: Vec::new(),
        }
    }
}
