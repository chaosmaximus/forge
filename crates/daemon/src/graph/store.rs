use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;
use petgraph::visit::EdgeRef;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct EdgeData {
    pub edge_type: String,
    pub properties: Value,
}

pub struct GraphStore {
    graph: DiGraph<String, EdgeData>,
    id_to_node: HashMap<String, NodeIndex>,
}

impl Default for GraphStore {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphStore {
    pub fn new() -> Self {
        GraphStore {
            graph: DiGraph::new(),
            id_to_node: HashMap::new(),
        }
    }

    /// Ensure a node exists. Returns NodeIndex.
    pub fn ensure_node(&mut self, id: &str) -> NodeIndex {
        if let Some(&idx) = self.id_to_node.get(id) {
            idx
        } else {
            let idx = self.graph.add_node(id.to_string());
            self.id_to_node.insert(id.to_string(), idx);
            idx
        }
    }

    /// Add directed edge from_id → to_id with type and properties.
    /// Auto-creates nodes if they don't exist via ensure_node.
    pub fn add_edge(&mut self, from_id: &str, to_id: &str, edge_type: &str, properties: Value) {
        let from = self.ensure_node(from_id);
        let to = self.ensure_node(to_id);
        self.graph.add_edge(
            from,
            to,
            EdgeData {
                edge_type: edge_type.to_string(),
                properties,
            },
        );
    }

    /// Get 1-hop outgoing neighbors. Returns Vec<(neighbor_id, edge_type)>.
    pub fn neighbors(&self, id: &str) -> Vec<(String, String)> {
        let Some(&idx) = self.id_to_node.get(id) else {
            return vec![];
        };
        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|e| {
                let neighbor_id = self.graph[e.target()].clone();
                let edge_type = e.weight().edge_type.clone();
                (neighbor_id, edge_type)
            })
            .collect()
    }

    /// Get 1-hop incoming neighbors. Returns Vec<(source_id, edge_type)>.
    pub fn incoming(&self, id: &str) -> Vec<(String, String)> {
        let Some(&idx) = self.id_to_node.get(id) else {
            return vec![];
        };
        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|e| {
                let source_id = self.graph[e.source()].clone();
                let edge_type = e.weight().edge_type.clone();
                (source_id, edge_type)
            })
            .collect()
    }

    /// BFS: collect all node IDs within N hops. Excludes the starting node.
    pub fn neighborhood(&self, id: &str, hops: usize) -> Vec<String> {
        let Some(&start) = self.id_to_node.get(id) else {
            return vec![];
        };

        // Queue entries: (NodeIndex, current_depth)
        let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();
        let mut visited: HashSet<NodeIndex> = HashSet::new();

        visited.insert(start);
        queue.push_back((start, 0));

        let mut result = vec![];

        while let Some((node, depth)) = queue.pop_front() {
            if depth >= hops {
                continue;
            }
            for neighbor in self.graph.neighbors_directed(node, Direction::Outgoing) {
                if !visited.contains(&neighbor) {
                    visited.insert(neighbor);
                    result.push(self.graph[neighbor].clone());
                    queue.push_back((neighbor, depth + 1));
                }
            }
        }

        result
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_add_and_traverse() {
        let mut store = GraphStore::new();
        store.add_edge("d10", "d5", "supersedes", json!({}));
        store.add_edge("d5", "d0", "supersedes", json!({}));

        let neighbors = store.neighbors("d10");
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].0, "d5");
        assert_eq!(neighbors[0].1, "supersedes");
    }

    #[test]
    fn test_neighborhood_bfs() {
        let mut store = GraphStore::new();
        store.add_edge("a", "b", "links", json!({}));
        store.add_edge("b", "c", "links", json!({}));
        store.add_edge("c", "d", "links", json!({}));

        let hop1 = store.neighborhood("a", 1);
        assert_eq!(hop1.len(), 1, "1-hop should have 1 node");

        let hop2 = store.neighborhood("a", 2);
        assert_eq!(hop2.len(), 2, "2-hop should have 2 nodes");

        let hop3 = store.neighborhood("a", 3);
        assert_eq!(hop3.len(), 3, "3-hop should have 3 nodes");
    }

    #[test]
    fn test_incoming() {
        let mut store = GraphStore::new();
        store.add_edge("d5", "d0", "supersedes", json!({}));

        let incoming = store.incoming("d0");
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].0, "d5");
        assert_eq!(incoming[0].1, "supersedes");
    }

    #[test]
    fn test_counts() {
        let mut store = GraphStore::new();
        store.add_edge("a", "b", "links", json!({}));

        assert_eq!(store.node_count(), 2);
        assert_eq!(store.edge_count(), 1);
    }
}
