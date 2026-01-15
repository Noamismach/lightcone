//! Minkowski-KV demo entrypoint.
//!
//! This binary wires together a small, intentionally “physics-forward” scenario:
//! - Two nodes (by default ports `5000` and `5001`) are labeled **Earth** and **Mars**.
//! - They are placed at different spatial coordinates, and we deliberately slow down the simulated
//!   speed of light so propagation delays are observable at human timescales.
//!
//! Architectural overview:
//! - A QUIC transport (`network`) moves bytes and decodes messages.
//! - A physics gate (`physics`) enforces relativistic causality by buffering messages until their
//!   earliest allowed arrival time $t_{arrival} = t_{received} + d/c$.
//! - The UI/application loop (`tui` + `app`) consumes *arrived* messages and updates the CRDT DAG.
//!
//! The emphasis is that “consistency” is not tied to wall-clock time; it is tied to causal
//! structure (parents) and spacetime separation (Minkowski interval / light cone constraints).

mod spacetime;
mod event;
mod dag;
mod simulation;
mod node;
mod action;
mod app;
mod tui;
mod protocol;
mod physics;
mod network;

use anyhow::Result;
use std::env;
use std::sync::Arc;
use std::collections::BTreeSet;

use crate::action::Action;
use crate::app::App;
use crate::tui::Tui;
use crate::network::{make_server_endpoint, Network, NetworkHandle};
use crate::physics::PhysicsLayer;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use crate::event::{Event, Operation};
use crate::spacetime::SpacetimeCoord;
use crate::protocol::ProtocolMessage;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Ensure the terminal is restored even if we panic inside the TUI loop.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let mut tui = Tui::new()?;
    let mut app = App::new();

    let port = env::args()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(5000);

    let (id, my_coords) = match port {
        5000 => ("Earth".to_string(), SpacetimeCoord { t: 0, x: 0.0, y: 0.0, z: 0.0 }),
        5001 => ("Mars".to_string(), SpacetimeCoord { t: 0, x: 300.0, y: 0.0, z: 0.0 }),
        _ => (format!("Node-{port}"), SpacetimeCoord { t: 0, x: 0.0, y: 0.0, z: 0.0 }),
    };

    // Simulation knob: slow down c so that interplanetary latency is visible.
    // (At physical c, 300 meters would be ~1 microsecond.)
    const SPEED_OF_LIGHT: f64 = 100.0;

    let endpoint = make_server_endpoint(&format!("127.0.0.1:{port}"))?;
    let net_handle = NetworkHandle::new(endpoint.clone());

    let (net_tx, mut net_rx) = mpsc::unbounded_channel();
    let physics = Arc::new(Mutex::new(PhysicsLayer::new(SPEED_OF_LIGHT)));

    let network = Network::new(endpoint, physics.clone(), net_tx, (my_coords.x, my_coords.y));

    // Run the network actor concurrently with the UI/application loop.
    // The network task only forwards messages once they have *causally arrived* (via PhysicsLayer).
    tokio::spawn(async move {
        let _ = network.run().await;
    });

    while !app.should_quit {
        tokio::select! {
            Some(action) = tui.action_rx.recv() => {
                match action {
                    Action::Render => {
                        tui.draw(&app)?;
                    }
                    Action::Broadcast(text) => {
                        let target_port = if port == 5000 { 5001 } else { 5000 };

                        let parents: BTreeSet<_> = app.dag.heads.iter().cloned().collect();
                        let event = Event::new(parents, my_coords.clone(), Operation::Put(id.clone(), text.clone().into_bytes()));

                        if let Err(e) = app.dag.add_event(event.clone()) {
                            eprintln!("local dag add error: {e:?}");
                        }

                        if let Err(e) = net_handle.send_gossip(target_port, ProtocolMessage::Gossip(event.clone())).await {
                            eprintln!("broadcast error: {e:?}");
                        }

                        app.update(Action::NewEvent(ProtocolMessage::Gossip(event)));
                    }
                    other => app.update(other),
                }
            }
            Some(net_action) = net_rx.recv() => {
                app.update(net_action);
            }
        }
    }

    Ok(())
}
