use crate::action::Action;
use crate::dag::SpacetimeDAG;

pub struct App {
    pub should_quit: bool,
    pub dag: SpacetimeDAG,
    pub viewport_offset: (f64, f64),
    pub viewport_scale: f64,
}

impl App {
    pub fn new() -> Self {
        Self {
            should_quit: false,
            dag: SpacetimeDAG::new(),
            viewport_offset: (0.0, 0.0),
            viewport_scale: 1.0,
        }
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::PanMap { dx, dy } => {
                self.viewport_offset.0 += dx;
                self.viewport_offset.1 += dy;
                println!("Pan to ({:.2},{:.2})", self.viewport_offset.0, self.viewport_offset.1);
            }
            Action::ZoomMap { factor } => {
                self.viewport_scale *= factor;
                println!("Zoom to {:.2}", self.viewport_scale);
            }
            Action::NewEvent(msg) => {
                println!("Received network event: {:?}", msg);
            }
            Action::Broadcast(text) => {
                println!("Broadcast requested with message: {text}");
            }
            _ => {}
        }
    }
}
