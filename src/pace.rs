//! PACE text: the `.gr` graph format every treewidth solver reads and the `.td`
//! decomposition format they write.
//!
//! Both formats number vertices and bags from 1; everything stored in this
//! crate is 0-based. A `.td`'s bag vertices are bounded by the vertex count its
//! solution line declares.

use crate::error::Error;
use crate::graph::{Graph, canonical};
use crate::td::{TdBag, TreeDecomposition};

impl Graph {
    /// Render as a PACE `.gr` graph (1-indexed vertices).
    pub fn to_gr(&self) -> String {
        let mut out = format!("p tw {} {}\n", self.num_vertices, self.edges.len());
        for &(u, v) in &self.edges {
            out.push_str(&format!("{} {}\n", u + 1, v + 1));
        }
        out
    }

    /// Read a PACE `.gr` graph. Self-loops are dropped and repeated edges kept
    /// once, so the edge count the problem line declares may exceed
    /// `edges.len()`.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] naming a malformed problem or edge line, an unparseable
    /// id, a vertex id outside the range the problem line declares, or a
    /// mismatch between the declared and actual edge-line counts, or a missing
    /// problem line.
    pub fn from_gr(text: &str) -> Result<Self, Error> {
        let mut num_vertices: Option<u32> = None;
        let mut declared_edge_lines = 0usize;
        let mut num_edge_lines = 0usize;
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('c') {
                continue;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens[0] == "p" {
                // "p tw <num_vertices> <num_edges>"
                if num_vertices.is_some() {
                    return Err(Error::Parse(format!("more than one problem line: {line}")));
                }
                if tokens.len() != 4 || tokens[1] != "tw" {
                    return Err(Error::Parse(format!("malformed problem line: {line}")));
                }
                num_vertices = Some(parse_count(tokens[2])?);
                declared_edge_lines = parse_count(tokens[3])?;
                continue;
            }
            let Some(n) = num_vertices else {
                return Err(Error::Parse(format!(
                    "edge line before the problem line: {line}"
                )));
            };
            if tokens.len() != 2 {
                return Err(Error::Parse(format!("malformed edge line: {line}")));
            }
            let u: u32 = to_zero_based("vertex", parse_count(tokens[0])?, n as usize)?;
            let v: u32 = to_zero_based("vertex", parse_count(tokens[1])?, n as usize)?;
            num_edge_lines += 1;
            if u != v {
                edges.push((u.min(v), u.max(v)));
            }
        }
        let Some(num_vertices) = num_vertices else {
            return Err(Error::Parse("no problem line".into()));
        };
        if num_edge_lines != declared_edge_lines {
            return Err(Error::Parse(format!(
                "the problem line declares {declared_edge_lines} edge lines but the file contains \
                 {num_edge_lines}"
            )));
        }
        Ok(Graph {
            num_vertices,
            edges: canonical(edges),
        })
    }
}

impl TreeDecomposition {
    /// Render as a PACE `.td` decomposition (1-indexed bags and vertices) of a
    /// graph over `num_vertices` vertices — the count the solution line has to
    /// declare, which the bags alone do not determine. A decomposition forest
    /// is connected between component roots for the PACE format; an empty
    /// decomposition is written as one empty bag.
    pub fn to_td(&self, num_vertices: u32) -> String {
        if self.bags.is_empty() {
            return format!("s td 1 0 {num_vertices}\nb 1\n");
        }
        let max_bag = self
            .bags
            .iter()
            .map(|b| b.vertices.len())
            .max()
            .unwrap_or(0);
        let mut out = format!("s td {} {} {}\n", self.bags.len(), max_bag, num_vertices);
        for bag in &self.bags {
            out.push_str(&format!("b {}", bag.id + 1));
            for &v in &bag.vertices {
                out.push_str(&format!(" {}", v + 1));
            }
            out.push('\n');
        }
        for (i, nbs) in self.adj.iter().enumerate() {
            for &j in nbs {
                if i < j {
                    out.push_str(&format!("{} {}\n", i + 1, j + 1));
                }
            }
        }
        let mut seen = vec![false; self.bags.len()];
        let mut component_roots = Vec::new();
        for start in 0..self.bags.len() {
            if seen[start] {
                continue;
            }
            component_roots.push(start);
            seen[start] = true;
            let mut stack = vec![start];
            while let Some(bag) = stack.pop() {
                for &neighbour in &self.adj[bag] {
                    if !seen[neighbour] {
                        seen[neighbour] = true;
                        stack.push(neighbour);
                    }
                }
            }
        }
        for roots in component_roots.windows(2) {
            out.push_str(&format!("{} {}\n", roots[0] + 1, roots[1] + 1));
        }
        out
    }

    /// Read a PACE `.td` decomposition — a treewidth solver's output.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] describing what is wrong with `text`: a malformed
    /// solution or bag line, an unparseable id, a bag or vertex id outside the
    /// range the solution line declares, a bag list that does not define each
    /// declared bag exactly once, a maximum-bag-size mismatch, a bag graph that
    /// is not one tree, or no bags at all.
    pub fn from_td(text: &str) -> Result<Self, Error> {
        let mut bags: Vec<TdBag> = Vec::new();
        let mut adj: Vec<Vec<usize>> = Vec::new();
        let mut num_bags = 0usize;
        let mut declared_max_bag_size = 0usize;
        let mut declared_vertices = 0usize;
        let mut saw_solution_line = false;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('c') {
                continue;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            match tokens[0] {
                "s" => {
                    // "s td <num_bags> <max_bag_size> <num_vertices>" — the
                    // counts every id below is checked against.
                    if saw_solution_line {
                        return Err(Error::Parse(format!("more than one solution line: {line}")));
                    }
                    if tokens.len() != 5 || tokens[1] != "td" {
                        return Err(Error::Parse(format!("Malformed solution line: {line}")));
                    }
                    num_bags = parse_count(tokens[2])?;
                    declared_max_bag_size = parse_count(tokens[3])?;
                    declared_vertices = parse_count(tokens[4])?;
                    bags = Vec::with_capacity(num_bags);
                    adj = vec![Vec::new(); num_bags];
                    saw_solution_line = true;
                }
                "b" => {
                    // "b <bag_id> <v1> <v2> ..."
                    if tokens.len() < 2 {
                        return Err(Error::Parse(format!("Malformed bag line: {line}")));
                    }
                    if !saw_solution_line {
                        return Err(Error::Parse(format!(
                            "bag line before the solution line: {line}"
                        )));
                    }
                    let bag_id = to_zero_based("bag id", parse_count(tokens[1])?, num_bags)?;
                    let vertices: Vec<u32> = tokens[2..]
                        .iter()
                        .map(|t| to_zero_based("vertex", parse_count(t)?, declared_vertices))
                        .collect::<Result<_, Error>>()?;
                    bags.push(TdBag {
                        id: bag_id,
                        vertices,
                    });
                }
                _ => {
                    // Tree edge: two bare integers "<bag_id1> <bag_id2>".
                    if !saw_solution_line {
                        return Err(Error::Parse(format!(
                            "tree edge before the solution line: {line}"
                        )));
                    }
                    if tokens.len() != 2 {
                        return Err(Error::Parse(format!("Malformed tree edge: {line}")));
                    }
                    let a: usize =
                        to_zero_based("bag id", parse_count::<usize>(tokens[0])?, num_bags)?;
                    let b: usize =
                        to_zero_based("bag id", parse_count::<usize>(tokens[1])?, num_bags)?;
                    if a >= b {
                        return Err(Error::Parse(format!(
                            "a tree edge must name its smaller bag id first: {line}"
                        )));
                    }
                    adj[a].push(b);
                    adj[b].push(a);
                }
            }
        }

        if !saw_solution_line {
            return Err(Error::Parse(
                "No solution line in tree decomposition output".into(),
            ));
        }
        if bags.is_empty() {
            return Err(Error::Parse(
                "No bags found in tree decomposition output".into(),
            ));
        }

        bags.sort_by_key(|b| b.id);

        // `adj` is sized from the solution line and `bags` from the bag lines,
        // so the two are co-indexed only when the file defines each declared
        // bag exactly once.
        if bags.len() != num_bags {
            return Err(Error::Parse(format!(
                "the solution line declares {num_bags} bags but the file defines {}",
                bags.len()
            )));
        }
        if let Some((_, bag)) = bags.iter().enumerate().find(|(i, b)| b.id != *i) {
            return Err(Error::Parse(format!(
                "bag {} is defined more than once; each of the {num_bags} declared bags is \
                 defined once",
                bag.id + 1
            )));
        }
        let actual_max_bag_size = bags.iter().map(|bag| bag.vertices.len()).max().unwrap_or(0);
        if actual_max_bag_size != declared_max_bag_size {
            return Err(Error::Parse(format!(
                "the solution line declares maximum bag size {declared_max_bag_size} but the \
                 largest bag contains {actual_max_bag_size} vertices"
            )));
        }
        let num_tree_edges = adj.iter().map(Vec::len).sum::<usize>() / 2;
        let expected_tree_edges = num_bags - 1;
        if num_tree_edges != expected_tree_edges {
            return Err(Error::Parse(format!(
                "the bag tree has {num_tree_edges} edges; a tree on {num_bags} bags has \
                 {expected_tree_edges}"
            )));
        }
        let mut seen = vec![false; num_bags];
        seen[0] = true;
        let mut stack = vec![0usize];
        while let Some(bag) = stack.pop() {
            for &neighbour in &adj[bag] {
                if !seen[neighbour] {
                    seen[neighbour] = true;
                    stack.push(neighbour);
                }
            }
        }
        if let Some(disconnected) = seen.iter().position(|&reached| !reached) {
            return Err(Error::Parse(format!(
                "bag {} is not connected to the bag tree rooted at 1",
                disconnected + 1
            )));
        }

        Ok(TreeDecomposition { bags, adj })
    }
}

fn parse_count<T: std::str::FromStr<Err = std::num::ParseIntError>>(
    token: &str,
) -> Result<T, Error> {
    token.parse::<T>().map_err(|e| Error::Parse(e.to_string()))
}

/// Ids are written 1-based and stored 0-based, so `0` is not an id at all;
/// `limit` is the count the problem or solution line declared.
fn to_zero_based<T: TryFrom<usize>>(what: &str, id: usize, limit: usize) -> Result<T, Error> {
    if id == 0 {
        return Err(Error::Parse(format!(
            "{what} 0 is out of range; ids are 1-based"
        )));
    }
    if id > limit {
        return Err(Error::Parse(format!(
            "{what} {id} is out of range; the header line declares {limit}"
        )));
    }
    T::try_from(id - 1).map_err(|_| Error::Parse(format!("{what} {id} does not fit")))
}
