use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

use crate::protocol::ProtocolMessage;

/// A message that has been delayed by the simulated speed-of-light constraint.
///
/// In Minkowski-KV, network transport can deliver bytes “immediately” (especially on loopback),
/// but the simulation layer must still enforce relativistic causality. We do that by attaching an
/// `available_at` timestamp to each received message, based on the sender/receiver separation.
#[derive(Debug, Clone)]
pub struct PendingPacket {
    /// The earliest wall-clock instant at which this message is allowed to enter the application.
    pub available_at: Instant,
    /// The decoded protocol payload.
    pub msg: ProtocolMessage,
}

impl Eq for PendingPacket {}

impl PartialEq for PendingPacket {
    fn eq(&self, other: &Self) -> bool {
        self.available_at.eq(&other.available_at)
    }
}

impl PartialOrd for PendingPacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // `BinaryHeap` is a max-heap; we reverse the ordering to pop the earliest deadline first.
        other.available_at.partial_cmp(&self.available_at)
    }
}

impl Ord for PendingPacket {
    fn cmp(&self, other: &Self) -> Ordering {
        // `BinaryHeap` is a max-heap; we reverse the ordering to pop the earliest deadline first.
        other.available_at.cmp(&self.available_at)
    }
}

/// Enforces a speed-of-light delivery constraint on incoming protocol messages.
///
/// Conceptually, the QUIC layer models “reliable signal transmission”, while this layer models the
/// *causal* constraint: information can only propagate at (or below) $c$.
///
/// Given a sender/receiver separation $d$ (meters) and configured $c$ (m/s), we compute a minimum
/// propagation delay $\Delta t = d/c$ and buffer the message until:
///
/// $t_{arrival} = t_{received} + d/c$
///
/// Trade-offs:
/// - We use `Instant` (monotonic wall clock) rather than event timestamps. That keeps the
///   implementation deterministic with respect to local scheduling, but it means this layer models
///   *propagation delay* rather than *coordinate-time transforms*.
/// - This is a single-process, single-host simulation primitive; a real deployment would measure
///   distance and time in a shared frame (or use consensus/clock sync) and would not be able to
///   “cheat” by delaying already-received bytes.
pub struct PhysicsLayer {
    buffer: BinaryHeap<PendingPacket>,
    c: f64,
}

impl PhysicsLayer {
    /// Creates a new physics layer with a configured speed of light.
    ///
    /// The caller is expected to set `c` to a *simulation-friendly* value; using the physical
    /// constant ($\approx 3\times 10^8$ m/s) makes local tests look instantaneous at human scales.
    pub fn new(c: f64) -> Self {
        Self {
            buffer: BinaryHeap::new(),
            c,
        }
    }

    /// Ingests a message that was received “on the wire”, and schedules it for causal delivery.
    ///
    /// `dist` is the separation between sender and receiver in meters. The transport layer should
    /// compute this from node coordinates carried by the protocol (e.g., event coordinates).
    ///
    /// This method does not block; it records a deadline and returns immediately.
    pub fn ingest(&mut self, msg: ProtocolMessage, dist: f64) {
        let delay = dist / self.c;
        let delay = if delay.is_sign_negative() { 0.0 } else { delay };
        let available_at = Instant::now() + Duration::from_secs_f64(delay);
        self.buffer.push(PendingPacket { available_at, msg });
    }

    /// Drains all messages whose causal deadline has passed.
    ///
    /// This is the “light cone gate”: messages outside the receiver's light cone remain buffered
    /// until their propagation time has elapsed.
    pub fn drain(&mut self) -> Vec<ProtocolMessage> {
        let now = Instant::now();
        let mut ready = Vec::new();
        while let Some(top) = self.buffer.peek() {
            if top.available_at <= now {
                let pkt = self.buffer.pop().expect("peek followed by pop");
                ready.push(pkt.msg);
            } else {
                break;
            }
        }
        ready
    }

    /// Compatibility alias for the network loop.
    ///
    /// The name emphasizes intent: deliver only messages that have *arrived* according to the
    /// simulated causal model.
    pub fn drain_arrived(&mut self) -> Vec<ProtocolMessage> {
        self.drain()
    }
}
