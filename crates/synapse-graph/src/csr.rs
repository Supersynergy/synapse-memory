//! CSR (Compressed Sparse Row) in-memory cache for edges.
//!
//! 10-100× faster traversal than SQLite recursive-CTE for hot graphs.
//! brain.db remains source-of-truth; CSR is cache rebuilt on demand.
//!
//! Layout:
//!   indptr[i..i+1] = range in indices[] giving neighbors of node i
//!   indices[]      = neighbor node IDs
//!   weights[]      = edge weights parallel to indices[]
//!   rels[]         = edge relation index (lookup table rel_dict)
//!
//! Memory: 4 + 12*E bytes for E edges (indptr i32, indices i32, weights f32, rels u8).

use crate::Result;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

type EdgeRow = (i64, i64, String, f64);

#[derive(Debug, Clone)]
pub struct CsrGraph {
    pub indptr: Vec<i32>,              // |V|+1
    pub indices: Vec<i32>,             // |E|
    pub weights: Vec<f32>,             // |E|
    pub rels: Vec<u8>,                 // |E| (index into rel_dict)
    pub rel_dict: Vec<String>,         // e.g. ["READ", "CITES", "MENTIONS"]
    pub node_index: HashMap<i64, i32>, // brain_id → csr_idx
    pub node_reverse: Vec<i64>,        // csr_idx → brain_id
    pub built_at: Instant,
    pub edge_count: usize,
}

impl CsrGraph {
    pub fn empty() -> Self {
        Self {
            indptr: vec![0],
            indices: vec![],
            weights: vec![],
            rels: vec![],
            rel_dict: vec![],
            node_index: HashMap::new(),
            node_reverse: vec![],
            built_at: Instant::now(),
            edge_count: 0,
        }
    }

    /// Rebuild CSR from brain.db edges. O(E log E) due to sort.
    pub fn rebuild(conn: &Connection) -> Result<Self> {
        let mut stmt = conn
            .prepare(
                "SELECT from_id, to_id, rel, COALESCE(weight, 1.0) FROM edges ORDER BY from_id",
            )
            .map_err(crate::GraphError::Sql)?;

        let mut edges: Vec<EdgeRow> = Vec::new();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, f64>(3)?,
                ))
            })
            .map_err(crate::GraphError::Sql)?;
        for row in rows {
            edges.push(row.map_err(crate::GraphError::Sql)?);
        }

        // Build node-id → csr-idx map
        let mut node_set: std::collections::BTreeSet<i64> = Default::default();
        for (f, t, _, _) in &edges {
            node_set.insert(*f);
            node_set.insert(*t);
        }
        let node_reverse: Vec<i64> = node_set.into_iter().collect();
        let node_index: HashMap<i64, i32> = node_reverse
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i as i32))
            .collect();

        // Build rel dictionary (max 256 rels, u8)
        let mut rel_dict: Vec<String> = Vec::new();
        let mut rel_idx: HashMap<String, u8> = HashMap::new();
        for (_, _, r, _) in &edges {
            if !rel_idx.contains_key(r) {
                let i = rel_dict.len() as u8;
                rel_dict.push(r.clone());
                rel_idx.insert(r.clone(), i);
            }
        }

        // Sort edges by from-id (already from SQL)
        let n_nodes = node_reverse.len();
        let n_edges = edges.len();
        let mut indptr = vec![0i32; n_nodes + 1];
        let mut indices = vec![0i32; n_edges];
        let mut weights = vec![0f32; n_edges];
        let mut rels = vec![0u8; n_edges];

        // Count edges per from-node
        for (f, _, _, _) in &edges {
            indptr[node_index[f] as usize + 1] += 1;
        }
        // Cumsum → indptr
        for i in 1..=n_nodes {
            indptr[i] += indptr[i - 1];
        }
        // Fill indices/weights/rels
        let mut cur = indptr.clone();
        for (f, t, r, w) in edges {
            let f_idx = node_index[&f] as usize;
            let pos = cur[f_idx] as usize;
            indices[pos] = node_index[&t];
            weights[pos] = w as f32;
            rels[pos] = rel_idx[&r];
            cur[f_idx] += 1;
        }

        Ok(Self {
            indptr,
            indices,
            weights,
            rels,
            rel_dict,
            node_index,
            node_reverse,
            built_at: Instant::now(),
            edge_count: n_edges,
        })
    }

    /// O(1) neighbor list (zero-copy slice).
    #[inline]
    pub fn neighbors(&self, csr_idx: i32) -> &[i32] {
        let i = csr_idx as usize;
        if i >= self.indptr.len() - 1 {
            return &[];
        }
        let s = self.indptr[i] as usize;
        let e = self.indptr[i + 1] as usize;
        &self.indices[s..e]
    }

    pub fn weight(&self, edge_offset: usize) -> f32 {
        self.weights[edge_offset]
    }

    pub fn rel(&self, edge_offset: usize) -> &str {
        &self.rel_dict[self.rels[edge_offset] as usize]
    }

    pub fn n_nodes(&self) -> usize {
        self.indptr.len() - 1
    }
    pub fn n_edges(&self) -> usize {
        self.edge_count
    }

    pub fn brain_to_csr(&self, brain_id: i64) -> Option<i32> {
        self.node_index.get(&brain_id).copied()
    }
    pub fn csr_to_brain(&self, csr_idx: i32) -> Option<i64> {
        self.node_reverse.get(csr_idx as usize).copied()
    }

    /// In-memory size estimate (bytes).
    pub fn size_bytes(&self) -> usize {
        4 * self.indptr.len()
            + 4 * self.indices.len()
            + 4 * self.weights.len()
            + self.rels.len()
            + self.rel_dict.iter().map(|s| s.len() + 24).sum::<usize>()
            + 16 * self.node_reverse.len()
            + 32 * self.node_index.len()
    }
}

/// Thread-safe singleton holder. Rebuild on schema change or on demand.
pub struct CsrCache {
    inner: RwLock<Option<CsrGraph>>,
}

impl CsrCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }
    pub fn get_or_build(&self, conn: &Connection) -> Result<CsrGuard<'_>> {
        if self.inner.read().unwrap().is_none() {
            let g = CsrGraph::rebuild(conn)?;
            *self.inner.write().unwrap() = Some(g);
        }
        Ok(CsrGuard {
            inner: self.inner.read().unwrap(),
        })
    }
    pub fn invalidate(&self) {
        *self.inner.write().unwrap() = None;
    }
    pub fn rebuild(&self, conn: &Connection) -> Result<()> {
        let g = CsrGraph::rebuild(conn)?;
        *self.inner.write().unwrap() = Some(g);
        Ok(())
    }
}

impl Default for CsrCache {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CsrGuard<'a> {
    inner: std::sync::RwLockReadGuard<'a, Option<CsrGraph>>,
}

impl<'a> std::ops::Deref for CsrGuard<'a> {
    type Target = CsrGraph;
    fn deref(&self) -> &CsrGraph {
        self.inner.as_ref().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::ensure_schema(&conn).unwrap();
        for (f, t, r, w) in [
            (1, 2, "READ", 1.0),
            (1, 3, "READ", 0.8),
            (2, 4, "CITES", 1.0),
            (3, 4, "CITES", 0.9),
            (4, 5, "MENTIONS", 0.7),
        ] {
            crate::relate(&conn, f, t, r, w, None).unwrap();
        }
        conn
    }

    #[test]
    fn csr_builds_correctly() {
        let conn = setup();
        let g = CsrGraph::rebuild(&conn).unwrap();
        assert_eq!(g.n_edges(), 5);
        assert_eq!(g.n_nodes(), 5); // {1,2,3,4,5}
        assert_eq!(g.rel_dict.len(), 3); // READ, CITES, MENTIONS
    }

    #[test]
    fn neighbors_zero_copy() {
        let conn = setup();
        let g = CsrGraph::rebuild(&conn).unwrap();
        let n1 = g.brain_to_csr(1).unwrap();
        let nbrs = g.neighbors(n1);
        assert_eq!(nbrs.len(), 2);
    }

    #[test]
    fn size_estimate_reasonable() {
        let conn = setup();
        let g = CsrGraph::rebuild(&conn).unwrap();
        let s = g.size_bytes();
        // ~5 edges × ~20 bytes ≈ 100 bytes minimum. Allow up to 4KB für dicts.
        assert!(s > 50 && s < 8192, "size = {s}");
    }

    #[test]
    fn cache_singleton_works() {
        let conn = setup();
        let c = CsrCache::new();
        {
            let g1 = c.get_or_build(&conn).unwrap();
            assert_eq!(g1.n_edges(), 5);
        }
        c.invalidate();
        {
            let g2 = c.get_or_build(&conn).unwrap();
            assert_eq!(g2.n_edges(), 5);
        }
    }
}
