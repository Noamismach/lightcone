use serde::{Deserialize, Serialize};

use crate::event::Event;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ProtocolMessage {
    Gossip(Event),
    Hello { coords: (f64, f64, f64) },
}
