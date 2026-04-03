use std::collections::HashMap;
use hnsw_rs::prelude::*;

pub struct VectorIndex {
    hnsw: Hnsw<'static, f32, DistCosine>,
    id_to_idx: HashMap<String, usize>,
    idx_to_id: HashMap<usize, String>,
    next_idx: usize,
    dim: usize,
}

impl VectorIndex {
    /// Create a new HNSW index for vectors of dimension `dim` with default capacity (100K).
    /// Parameters: max_connections=16, nb_layers=16, ef_construction=200, DistCosine.
    pub fn new(dim: usize) -> Self {
        Self::with_capacity(dim, 100_000)
    }

    /// Create a new HNSW index with a custom maximum element capacity.
    pub fn with_capacity(dim: usize, max_elements: usize) -> Self {
        // Hnsw::new(max_nb_connection, max_elements, max_layer, ef_construction, dist)
        let hnsw = Hnsw::<f32, DistCosine>::new(16, max_elements, 16, 200, DistCosine {});
        VectorIndex {
            hnsw,
            id_to_idx: HashMap::new(),
            idx_to_id: HashMap::new(),
            next_idx: 0,
            dim,
        }
    }

    /// Insert a vector with the given string ID.
    /// Returns the internal usize index assigned to this vector.
    ///
    /// Returns `Err` if `embedding.len() != self.dim` (NEW-4: no panic/Mutex poison).
    pub fn insert(&mut self, id: &str, embedding: &[f32]) -> Result<usize, String> {
        if embedding.len() != self.dim {
            return Err(format!(
                "embedding dimension mismatch: expected {}, got {}",
                self.dim,
                embedding.len()
            ));
        }

        let idx = self.next_idx;
        self.hnsw.insert((embedding, idx));
        self.id_to_idx.insert(id.to_string(), idx);
        self.idx_to_id.insert(idx, id.to_string());
        self.next_idx += 1;
        Ok(idx)
    }

    /// Search for the `k` nearest neighbours of `query`.
    /// Returns (id, distance) pairs sorted by ascending distance.
    ///
    /// Returns `Err` if `query.len() != self.dim` (NEW-4: no panic/Mutex poison).
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<(String, f32)>, String> {
        if query.len() != self.dim {
            return Err(format!(
                "query dimension mismatch: expected {}, got {}",
                self.dim,
                query.len()
            ));
        }

        if self.is_empty() {
            return Ok(Vec::new());
        }

        let ef_search = (k * 3).max(30);
        let neighbours = self.hnsw.search(query, k, ef_search);
        Ok(neighbours
            .into_iter()
            .filter_map(|n| {
                self.idx_to_id
                    .get(&n.d_id)
                    .map(|id| (id.clone(), n.distance))
            })
            .collect())
    }

    /// Returns the number of vectors currently indexed.
    pub fn len(&self) -> usize {
        self.next_idx
    }

    /// Returns true if no vectors have been indexed.
    pub fn is_empty(&self) -> bool {
        self.next_idx == 0
    }

    /// Returns true if a vector with the given string ID has been inserted.
    pub fn contains(&self, id: &str) -> bool {
        self.id_to_idx.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_embedding(dim: usize, seed: f32) -> Vec<f32> {
        (0..dim).map(|j| (j as f32 * 0.001 + seed).sin()).collect()
    }

    #[test]
    fn test_insert_and_search() {
        let dim = 768;
        let mut index = VectorIndex::new(dim);

        // Insert 50 vectors with distinct seeds
        for i in 0..50usize {
            let emb = make_embedding(dim, i as f32 * 0.5);
            index.insert(&format!("d{}", i), &emb).unwrap();
        }

        assert_eq!(index.len(), 50);

        // Search for the closest to d0
        let query = make_embedding(dim, 0.0);
        let results = index.search(&query, 5).unwrap();

        assert!(!results.is_empty(), "search must return results");

        // d0 should be the nearest neighbour (distance to itself is 0 or very close)
        let nearest_id = &results[0].0;
        assert_eq!(nearest_id, "d0", "d0 should be the nearest neighbour to itself");
    }

    #[test]
    fn test_empty_search() {
        let dim = 768;
        let index = VectorIndex::new(dim);
        let query = make_embedding(dim, 0.0);
        let results = index.search(&query, 5).unwrap();
        assert!(results.is_empty(), "search on empty index must return empty vec");
    }

    #[test]
    fn test_contains() {
        let dim = 768;
        let mut index = VectorIndex::new(dim);
        let emb = make_embedding(dim, 1.0);
        index.insert("abc", &emb).unwrap();
        assert!(index.contains("abc"), "should contain 'abc' after insert");
        assert!(!index.contains("xyz"), "should not contain 'xyz'");
    }

    #[test]
    fn test_insert_dimension_mismatch() {
        let mut index = VectorIndex::new(768);
        let wrong_emb = vec![1.0f32; 10]; // wrong dimension
        let result = index.insert("bad", &wrong_emb);
        assert!(result.is_err(), "insert with wrong dimension should return Err");
        assert!(result.unwrap_err().contains("dimension mismatch"));
    }

    #[test]
    fn test_search_dimension_mismatch() {
        let index = VectorIndex::new(768);
        let wrong_query = vec![1.0f32; 10];
        let result = index.search(&wrong_query, 5);
        assert!(result.is_err(), "search with wrong dimension should return Err");
        assert!(result.unwrap_err().contains("dimension mismatch"));
    }
}
