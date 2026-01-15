use std::collections::BTreeSet;
use std::sync::Arc;

use tokio::select;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::dag::SpacetimeDAG;
use crate::event::{Event, EventHash, Operation};
use crate::simulation::{Cluster, Message, NetworkPacket};
use crate::spacetime::SpacetimeCoord;

pub enum NodeCommand {
    CreateEvent {
        coords: SpacetimeCoord,
        payload: Operation,
    },
}

pub struct Node {
    pub id: Uuid,
    pub dag: Arc<Mutex<SpacetimeDAG>>,
    pub coords: SpacetimeCoord,
    pub inbox: mpsc::Receiver<Message>,
    pub cluster: Arc<Cluster>,
    pub cluster_handle: crate::simulation::NodeHandle,
    pub peers: Vec<Uuid>,
    pub command_rx: mpsc::Receiver<NodeCommand>,
}

impl Node {
    pub fn new(
        id: Uuid,
        coords: SpacetimeCoord,
        inbox: mpsc::Receiver<Message>,
        cluster: Arc<Cluster>,
        cluster_handle: crate::simulation::NodeHandle,
        peers: Vec<Uuid>,
    ) -> (Self, mpsc::Sender<NodeCommand>, Arc<Mutex<SpacetimeDAG>>) {
        let dag = Arc::new(Mutex::new(SpacetimeDAG::new()));
        let (command_tx, command_rx) = mpsc::channel(16);

        (
            Self {
                id,
                dag: dag.clone(),
                coords,
                inbox,
                cluster,
                cluster_handle,
                peers,
                command_rx,
            },
            command_tx,
            dag,
        )
    }

    pub async fn run(mut self) {
        println!("Node {} starting with coords ({}, {}, {})", self.id, self.coords.x, self.coords.y, self.coords.z);
        loop {
            select! {
                maybe_msg = self.inbox.recv() => {
                    if let Some(msg) = maybe_msg {
                        self.handle_message(msg).await;
                    } else {
                        break;
                    }
                }
                maybe_cmd = self.command_rx.recv() => {
                    if let Some(cmd) = maybe_cmd {
                        self.handle_command(cmd).await;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    async fn handle_message(&mut self, msg: Message) {
        match msg.payload {
            NetworkPacket::Gossip(events) => self.handle_gossip(events).await,
            NetworkPacket::AntiEntropyRequest(_) => {
                // Not implemented in this prototype.
            }
        }
    }

    async fn handle_command(&mut self, cmd: NodeCommand) {
        match cmd {
            NodeCommand::CreateEvent { coords, payload } => {
                self.create_event(coords, payload).await;
            }
        }
    }

    async fn create_event(&mut self, coords: SpacetimeCoord, payload: Operation) {
        let mut dag = self.dag.lock().await;
        let parents: BTreeSet<_> = dag.heads.iter().cloned().collect();
        let mut ancestors: Vec<Event> = Vec::new();

        // Collect parent events so peers can ingest without parent-missing failures.
        for parent_hash in &parents {
            if let Some(idx) = dag.index_map.get(parent_hash) {
                ancestors.push(dag.graph[*idx].clone());
            }
        }

        let event = Event::new(parents, coords, payload);
        let event_hash = event.hash;

        if let Err(err) = dag.add_event(event.clone()) {
            println!("Node {} failed to add local event ({}): {:?}", self.id, fmt_hash(&event_hash), err);
            return;
        }

        // Build gossip payload with ancestors first, then the new event.
        let mut gossip_events = ancestors;
        gossip_events.push(event.clone());

        drop(dag);

        for peer in &self.peers {
            let msg = Message {
                sender: self.id,
                receiver: *peer,
                send_time: coords.t,
                payload: NetworkPacket::Gossip(gossip_events.clone()),
            };

            if let Err(e) = self.cluster.route_message(msg) {
                println!("Node {} failed to route gossip to {}: {}", self.id, peer, e);
            }
        }

        println!("Node {} created event {:?} (hash {})", self.id, event.payload, fmt_hash(&event_hash));
    }

    async fn handle_gossip(&mut self, events: Vec<Event>) {
        let mut dag = self.dag.lock().await;
        let mut added = 0usize;
        for ev in events {
            println!("Processing incoming event: {}", fmt_hash(&ev.hash));
            match dag.add_event(ev) {
                Ok(_) => added += 1,
                Err(err) => println!("Node {} failed to add event: {:?}", self.id, err),
            }
        }
        println!(
            "Node {} received {} events. Current Heads: {}",
            self.id,
            added,
            dag.heads.len()
        );
    }
}

fn fmt_hash(hash: &EventHash) -> String {
    hash.iter().map(|b| format!("{:02x}", b)).collect::<String>()
}
