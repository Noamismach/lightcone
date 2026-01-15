use std::cmp::Ordering;
use std::collections::BTreeSet;

use blake3::Hasher;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::spacetime::SpacetimeCoord;

/// Content-addressed identifier for an [`Event`].
///
/// In Minkowski-KV, events are immutable and replicated by gossip. Replicas use `EventHash` to:
/// - deduplicate received events (same hash => same content),
/// - connect parent links when building the causal DAG.
///
/// Implementation detail: this is currently a 32-byte BLAKE3 digest.
pub type EventHash = [u8; 32];

/// The semantic operation carried by an [`Event`].
///
/// This is the “what happened” component. The “when/where” (spacetime coordinates) and the causal
/// structure (parents) live on the surrounding [`Event`].
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Operation {
    /// Insert or overwrite a key with a value.
    Put(String, Vec<u8>),
    /// Remove a key.
    Delete(String),
    /// Placeholder for application-defined conflict resolution / join semantics.
    Merge,
    /// The root operation anchoring the DAG.
    Genesis,
}

/// Immutable, content-addressed database event.
///
/// Think of an `Event` as a CRDT “block”: once created, it is never mutated. Replicas exchange
/// events and rebuild a local view of the database by replaying/merging them.
///
/// Causality and the DAG:
/// - `parents` contains the hashes of the events this event *directly depends on*.
/// - These parent links form a directed acyclic graph (DAG) of causality.
/// - Concurrency is explicit: if two replicas emit events with the same parent set (or otherwise
///   without knowledge of each other), the DAG forks and both can remain as heads.
///
/// Hashing and deduplication:
/// - `hash` is derived from the event content (id, parents, coords, payload).
/// - On the wire, we only need enough information to recompute/identify events; receivers can use
///   the hash as a stable key for storage and dedup.
///
/// Note: `coords` provide the spacetime embedding used by the simulation to enforce a light-cone
/// arrival constraint. They do not, by themselves, impose a total order.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Event {
    /// Random event identifier (used as part of the content hash).
    pub id: Uuid,
    /// Hashes of direct causal predecessors.
    pub parents: BTreeSet<EventHash>,
    /// Spacetime coordinate of where/when this event was authored (in the chosen frame).
    pub coords: SpacetimeCoord,
    /// Application-level operation payload.
    pub payload: Operation,
    /// Content address of this event.
    ///
    /// This is skipped during serde serialization and recomputed on creation.
    #[serde(skip)]
    pub hash: EventHash,
}

impl Event {
    /// Constructs a new immutable event and computes its content hash.
    pub fn new(parents: BTreeSet<EventHash>, coords: SpacetimeCoord, payload: Operation) -> Self {
        let id = Uuid::new_v4();
        let hash = Self::compute_hash(&id, &parents, &coords, &payload);

        Self {
            id,
            parents,
            coords,
            payload,
            hash,
        }
    }

    fn compute_hash(
        id: &Uuid,
        parents: &BTreeSet<EventHash>,
        coords: &SpacetimeCoord,
        payload: &Operation,
    ) -> EventHash {
        let mut hasher = Hasher::new();

        hasher.update(id.as_bytes());

        for parent in parents {
            hasher.update(parent);
        }

        hasher.update(&coords.t.to_le_bytes());
        hasher.update(&coords.x.to_le_bytes());
        hasher.update(&coords.y.to_le_bytes());
        hasher.update(&coords.z.to_le_bytes());

        let payload_bytes = bincode::serialize(payload).expect("Failed to serialize payload for hashing");
        hasher.update(&payload_bytes);

        *hasher.finalize().as_bytes()
    }
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.coords.partial_cmp(&other.coords)
    }
}
