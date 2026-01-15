use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc::Sender;
use uuid::Uuid;

use crate::event::{Event, EventHash};
use crate::spacetime::{speed_of_light, SpacetimeCoord};

#[derive(Debug, Clone)]
pub enum NetworkPacket {
    Gossip(Vec<Event>),
    AntiEntropyRequest(Vec<EventHash>),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub sender: Uuid,
    pub receiver: Uuid,
    pub send_time: u128,
    pub payload: NetworkPacket,
}

#[derive(Clone)]
pub struct NodeHandle {
    pub tx: Sender<Message>,
    pub coords: SpacetimeCoord,
}

pub struct Cluster {
    pub nodes: HashMap<Uuid, NodeHandle>,
}

impl Cluster {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    pub fn register_node(&mut self, id: Uuid, tx: Sender<Message>, coords: SpacetimeCoord) {
        self.nodes.insert(id, NodeHandle { tx, coords });
    }

    pub fn route_message(&self, msg: Message) -> Result<(), String> {
        let sender = self
            .nodes
            .get(&msg.sender)
            .ok_or_else(|| "Unknown sender".to_string())?;
        let receiver = self
            .nodes
            .get(&msg.receiver)
            .ok_or_else(|| "Unknown receiver".to_string())?;

        let dx = receiver.coords.x - sender.coords.x;
        let dy = receiver.coords.y - sender.coords.y;
        let dz = receiver.coords.z - sender.coords.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();

        let c = speed_of_light();
        let delay = distance / c;
        let delay_dur = Duration::from_secs_f64(delay);

        let tx = receiver.tx.clone();
        tokio::spawn(async move {
            println!("[Sim] Packet traveling... Delay: {delay} sec");
            tokio::time::sleep(delay_dur).await;
            let _ = tx.send(msg).await;
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spacetime::set_speed_of_light;
    use tokio::time::{sleep, timeout};

    #[tokio::test]
    async fn mars_delay() {
        // Slow down light for the simulation to keep test wall time reasonable.
        set_speed_of_light(100.0);

        let (tx_earth, _rx_earth) = tokio::sync::mpsc::channel(8);
        let (tx_mars, mut rx_mars) = tokio::sync::mpsc::channel(8);

        let earth_id = Uuid::new_v4();
        let mars_id = Uuid::new_v4();

        let earth_coords = SpacetimeCoord {
            t: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let mars_coords = SpacetimeCoord {
            t: 0,
            x: 1000.0,
            y: 0.0,
            z: 0.0,
        };

        let mut cluster = Cluster::new();
        cluster.register_node(earth_id, tx_earth, earth_coords);
        cluster.register_node(mars_id, tx_mars, mars_coords);

        let msg = Message {
            sender: earth_id,
            receiver: mars_id,
            send_time: 0,
            payload: NetworkPacket::Gossip(vec![]),
        };

        cluster.route_message(msg).expect("routing should succeed");

        let early = timeout(Duration::from_secs(1), rx_mars.recv()).await;
        assert!(early.is_err(), "Message should not arrive before light delay");

        sleep(Duration::from_secs(11)).await;
        let received = rx_mars.recv().await;
        assert!(received.is_some(), "Message should arrive after simulated delay");
    }
}
