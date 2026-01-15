//! Spacetime event DAG (CRDT core).
//!
//! Minkowski-KV models database mutations as *immutable events* embedded in spacetime. Each event
//! references its causal predecessors (parents), forming an append-only directed acyclic graph.
//!
//! This structure behaves like an operation-based CRDT:
//! - Events are immutable and content-addressed; replicas exchange events (gossip) rather than
//!   shipping state.
//! - Merging is monotonic: receiving an event only adds nodes/edges.
//! - Conflicts are not “overwritten”; they become explicit concurrency in the graph.
//!
//! The key relativity-driven idea is how we interpret concurrency:
//! - If two writes are *timelike* related (inside each other's light cone), there is a physically
//!   enforceable causal order.
//! - If two writes are *spacelike* separated, there is no causal order implied by physics alone,
//!   and the DAG forks.
//!
//! The `heads` set is the database's **concurrent frontier**: the current “tips” of the graph that
//! have no known successors. When independent replicas write concurrently, heads temporarily
//! diverge; subsequent events (or application-level resolution) may join the branches.

use std::collections::BTreeSet;

use dashmap::DashMap;
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;
use thiserror::Error;

/// Immutable, content-addressed operation record.
///
/// Events are the unit of replication in Minkowski-KV: replicas gossip events and rebuild/merge
/// the DAG locally. Events are append-only; they are never mutated in place.
pub(crate) use crate::event::Event;

/// Stable identifier for an `Event` (typically a cryptographic hash of its contents).
///
/// We treat hashes as globally unique IDs for deduplication and for connecting parent links.
pub(crate) use crate::event::EventHash;

use crate::event::Operation;
use crate::spacetime::SpacetimeCoord;

#[derive(Debug, Error)]
pub enum DagError {
    /// An event references a parent that this replica does not yet have.
    ///
    /// In a gossip-based system this is normal: parents may arrive later, or may need to be
    /// requested explicitly. The current implementation is strict and rejects the event.
    #[error("Missing parent event")]
    MissingParent,
}

/// Append-only event DAG used as the CRDT backbone of the database.
///
/// Design goals:
/// - **Monotonic growth:** merges only add information.
/// - **Explicit concurrency:** spacelike-separated writes produce multiple heads rather than a
///   forced total order.
/// - **Fast lookup:** events are indexed by `EventHash` for deduplication and parent resolution.
///
/// Note: The DAG records causality via explicit parent links. Spacetime (Minkowski) reasoning is
/// used elsewhere to decide when events are allowed to arrive and whether an ordering is
/// meaningful; the DAG's job is to preserve the partial order once events are admitted.
pub struct SpacetimeDAG {
    /// The underlying graph: nodes are events, edges point to parents.
    pub graph: StableDiGraph<Event, ()>,
    /// Maps content hashes to node indices for O(1)-ish parent resolution.
    pub index_map: DashMap<EventHash, NodeIndex>,
    /// The current concurrency frontier (tips of the DAG).
    ///
    /// Intuition: if two replicas write concurrently (no known causal relation), both writes remain
    /// as heads until a later event references one or both as parents.
    pub heads: Vec<EventHash>,
}

impl SpacetimeDAG {
    /// Creates a new DAG with a single genesis event.
    ///
    /// Genesis anchors the graph so that every subsequent event has at least one ancestor.
    pub fn new() -> Self {
        let mut dag = Self {
            graph: StableDiGraph::new(),
            index_map: DashMap::new(),
            heads: Vec::new(),
        };

        let genesis = Event::new(
            BTreeSet::new(),
            SpacetimeCoord {
                t: 0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Operation::Genesis,
        );

        let genesis_hash = genesis.hash;
        let node = dag.graph.add_node(genesis);
        dag.index_map.insert(genesis_hash, node);
        dag.heads.push(genesis_hash);

        dag
    }

    /// Appends an event to the DAG.
    ///
    /// This is a pure append operation: if all parents are present, we insert the event and connect
    /// edges to its parents. We also update `heads` to reflect the new concurrency frontier.
    ///
    /// Current limitation:
    /// - Events that arrive “before their parents” are rejected. In a full gossip implementation we
    ///   would queue or request missing parents and retry.
    pub fn add_event(&mut self, event: Event) -> Result<(), DagError> {
        for parent in &event.parents {
            if !self.index_map.contains_key(parent) {
                return Err(DagError::MissingParent);
            }
        }

        let event_hash = event.hash;
        let node = self.graph.add_node(event);
        self.index_map.insert(event_hash, node);

        let parent_hashes: Vec<EventHash> = self.graph[node].parents.iter().cloned().collect();
        for parent in parent_hashes {
            if let Some(parent_idx) = self.index_map.get(&parent) {
                self.graph.add_edge(node, *parent_idx, ());
            }
        }

        self.heads.retain(|h| !self.graph[node].parents.contains(h));
        self.heads.push(event_hash);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_heads_are_tracked() {
        let mut dag = SpacetimeDAG::new();
        let genesis_hash = dag.heads[0];

        let mut parents = BTreeSet::new();
        parents.insert(genesis_hash);

        let earth_event = Event::new(
            parents.clone(),
            SpacetimeCoord {
                t: 0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Operation::Put("earth".to_string(), vec![1]),
        );

        let mars_event = Event::new(
            parents,
            SpacetimeCoord {
                t: 0,
                x: 5.4e10,
                y: 0.0,
                z: 0.0,
            },
            Operation::Put("mars".to_string(), vec![2]),
        );

        dag.add_event(earth_event).expect("earth should attach to genesis");
        dag.add_event(mars_event).expect("mars should attach to genesis");

        assert_eq!(dag.heads.len(), 2, "Two concurrent heads expected");
    }
}
