use crate::protocol::ProtocolMessage;

#[derive(Debug, Clone)]
pub enum Action {
    Tick,
    Render,
    Resize(u16, u16),
    Quit,
    PanMap { dx: f64, dy: f64 },
    ZoomMap { factor: f64 },
    NewEvent(ProtocolMessage),
    Broadcast(String),
}
