use rusqlite::{params, Connection};
use std::collections::HashMap;

/// Run label propagation community detection on the call+import graph.
///
/// Algorithm:
/// 1. Build adjacency from edge table where edge_type IN ('calls', 'imports') and reality_id = ?
/// 2. Each node starts with its own label (index)
/// 3. Iteratively: each node adopts the most frequent label among its neighbors
/// 4. Stop when labels converge or max_iterations reached
/// 5. Store clusters as edges: edge_type='belongs_to_cluster', from_id=file, to_id=cluster:{reality_id}:{idx}
///
/// Returns number of clusters found.
///
/// Capped at 5000 nodes to avoid runaway cost on huge codebases.
pub fn run_label_propagation(
    conn: &Connection,
    reality_id: &str,
    max_iterations: usize,
) -> rusqlite::Result<usize> {
    // 1. Build adjacency from edge table
    let mut stmt = conn.prepare(
        "SELECT DISTINCT from_id, to_id FROM edge \
         WHERE edge_type IN ('calls', 'imports') \
         AND (reality_id = ?1 OR reality_id IS NULL)"
    )?;

    let edges: Vec<(String, String)> = stmt.query_map(params![reality_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?.filter_map(|r| r.ok()).collect();

    if edges.is_empty() {
        // No edges: delete any old cluster edges and return 0
        conn.execute(
            "DELETE FROM edge WHERE edge_type = 'belongs_to_cluster' AND reality_id = ?1",
            params![reality_id],
        )?;
        return Ok(0);
    }

    // 2. Extract unique node IDs
    let mut node_set = std::collections::HashSet::new();
    for (from, to) in &edges {
        node_set.insert(from.clone());
        node_set.insert(to.clone());
    }

    // Cap at 5000 nodes
    if node_set.len() > 5000 {
        // Take the first 5000 nodes (deterministic via BTreeSet)
        let sorted: std::collections::BTreeSet<String> = node_set.into_iter().collect();
        node_set = sorted.into_iter().take(5000).collect();
    }

    let nodes: Vec<String> = node_set.into_iter().collect();
    let node_index: HashMap<&str, usize> = nodes.iter().enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();

    // Build adjacency list (undirected for label propagation)
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (from, to) in &edges {
        if let (Some(&fi), Some(&ti)) = (node_index.get(from.as_str()), node_index.get(to.as_str())) {
            adjacency[fi].push(ti);
            adjacency[ti].push(fi);
        }
    }

    // 3. Initialize: each node gets label = its own index
    let mut labels: Vec<usize> = (0..nodes.len()).collect();

    // 4. Iterate
    let capped_iterations = max_iterations.min(20);
    for _iter in 0..capped_iterations {
        let mut changed = false;
        for node in 0..nodes.len() {
            let neighbors = &adjacency[node];
            if neighbors.is_empty() {
                continue;
            }
            // Find most frequent label among neighbors
            let mut freq: HashMap<usize, usize> = HashMap::new();
            for &neighbor in neighbors {
                *freq.entry(labels[neighbor]).or_insert(0) += 1;
            }
            let best_label = freq.into_iter()
                .max_by_key(|&(_, count)| count)
                .map(|(label, _)| label)
                .unwrap_or(labels[node]);

            if best_label != labels[node] {
                labels[node] = best_label;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // 5. Group nodes by final label → clusters
    let mut clusters: HashMap<usize, Vec<&str>> = HashMap::new();
    for (i, &label) in labels.iter().enumerate() {
        clusters.entry(label).or_default().push(&nodes[i]);
    }

    // Filter out singleton clusters (isolated nodes)
    let cluster_groups: Vec<Vec<&str>> = clusters.into_values()
        .filter(|group| group.len() > 1)
        .collect();

    let num_clusters = cluster_groups.len();

    // 6. Delete old cluster edges
    conn.execute(
        "DELETE FROM edge WHERE edge_type = 'belongs_to_cluster' AND reality_id = ?1",
        params![reality_id],
    )?;

    // 7. Insert new cluster edges
    let now = forge_core::time::now_iso();
    for (idx, group) in cluster_groups.iter().enumerate() {
        let cluster_id = format!("cluster:{}:{}", reality_id, idx);
        for &node_id in group {
            let edge_id = ulid::Ulid::new().to_string();
            conn.execute(
                "INSERT INTO edge (id, from_id, to_id, edge_type, reality_id, properties, created_at, valid_from) \
                 VALUES (?1, ?2, ?3, 'belongs_to_cluster', ?4, '{}', ?5, ?5)",
                params![edge_id, node_id, cluster_id, reality_id, now],
            )?;
        }
    }

    // 8. Return number of distinct clusters
    Ok(num_clusters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn
    }

    fn insert_edge(conn: &Connection, from: &str, to: &str, edge_type: &str, reality_id: &str) {
        let id = ulid::Ulid::new().to_string();
        let now = forge_core::time::now_iso();
        conn.execute(
            "INSERT INTO edge (id, from_id, to_id, edge_type, reality_id, properties, created_at, valid_from) \
             VALUES (?1, ?2, ?3, ?4, ?5, '{}', ?6, ?6)",
            params![id, from, to, edge_type, reality_id, now],
        ).unwrap();
    }

    #[test]
    fn test_label_propagation_simple_graph() {
        // 6 nodes in 2 connected components:
        // Component 1: A-B-C (triangle)
        // Component 2: D-E-F (triangle)
        let conn = setup_test_db();
        let rid = "test-reality-1";

        insert_edge(&conn, "A", "B", "calls", rid);
        insert_edge(&conn, "B", "C", "calls", rid);
        insert_edge(&conn, "A", "C", "imports", rid);

        insert_edge(&conn, "D", "E", "calls", rid);
        insert_edge(&conn, "E", "F", "calls", rid);
        insert_edge(&conn, "D", "F", "imports", rid);

        let clusters = run_label_propagation(&conn, rid, 20).unwrap();
        assert_eq!(clusters, 2, "two connected components should yield 2 clusters");
    }

    #[test]
    fn test_label_propagation_single_component() {
        // Fully connected: A-B, B-C, C-A → 1 cluster
        let conn = setup_test_db();
        let rid = "test-reality-2";

        insert_edge(&conn, "A", "B", "calls", rid);
        insert_edge(&conn, "B", "C", "calls", rid);
        insert_edge(&conn, "C", "A", "imports", rid);

        let clusters = run_label_propagation(&conn, rid, 20).unwrap();
        assert_eq!(clusters, 1, "single connected component should yield 1 cluster");
    }

    #[test]
    fn test_label_propagation_empty_graph() {
        let conn = setup_test_db();
        let rid = "test-reality-3";

        let clusters = run_label_propagation(&conn, rid, 20).unwrap();
        assert_eq!(clusters, 0, "no edges should yield 0 clusters");
    }

    #[test]
    fn test_label_propagation_max_iterations() {
        // Even with max_iterations=1, a simple graph should converge
        let conn = setup_test_db();
        let rid = "test-reality-4";

        insert_edge(&conn, "A", "B", "calls", rid);
        insert_edge(&conn, "B", "C", "calls", rid);

        let clusters = run_label_propagation(&conn, rid, 1).unwrap();
        // With only 1 iteration, result depends on convergence speed
        // but for a chain, it should produce at least 1 cluster
        assert!(clusters >= 1, "should produce at least 1 cluster, got {}", clusters);
    }

    #[test]
    fn test_label_propagation_stores_edges() {
        let conn = setup_test_db();
        let rid = "test-reality-5";

        insert_edge(&conn, "A", "B", "calls", rid);
        insert_edge(&conn, "B", "C", "calls", rid);
        insert_edge(&conn, "A", "C", "imports", rid);

        let clusters = run_label_propagation(&conn, rid, 20).unwrap();
        assert!(clusters >= 1);

        // Verify cluster edges exist in DB
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edge WHERE edge_type = 'belongs_to_cluster' AND reality_id = ?1",
            params![rid],
            |row| row.get(0),
        ).unwrap();

        assert!(count >= 3, "at least 3 cluster edges for 3 nodes in 1 cluster, got {}", count);

        // Verify the to_id follows the cluster:{reality_id}:{index} format
        let sample_to: String = conn.query_row(
            "SELECT to_id FROM edge WHERE edge_type = 'belongs_to_cluster' AND reality_id = ?1 LIMIT 1",
            params![rid],
            |row| row.get(0),
        ).unwrap();
        assert!(sample_to.starts_with(&format!("cluster:{}:", rid)),
            "cluster edge to_id should start with 'cluster:{rid}:', got: {sample_to}");
    }
}
