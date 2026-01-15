use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event as CEvent, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, ExecutableCommand};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::interval;

use crate::action::Action;
use crate::app::App;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    _task: JoinHandle<()>,
    pub action_rx: mpsc::Receiver<Action>,
}

impl Tui {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        io::stdout().execute(EnterAlternateScreen)?;

        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;

        let (action_tx, action_rx) = mpsc::channel(128);
        let input_tx = action_tx.clone();

        let handle = tokio::spawn(async move {
            let mut render_interval = interval(Duration::from_millis(16));
            let mut tick_interval = interval(Duration::from_millis(250));
            let mut events = EventStream::new();

            loop {
                tokio::select! {
                    _ = render_interval.tick() => {
                        let _ = action_tx.send(Action::Render).await;
                    }
                    _ = tick_interval.tick() => {
                        let _ = action_tx.send(Action::Tick).await;
                    }
                    maybe_evt = events.next() => {
                        if let Some(Ok(evt)) = maybe_evt {
                            if let Some(action) = map_event(evt) {
                                let _ = input_tx.send(action).await;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            terminal,
            _task: handle,
            action_rx,
        })
    }

    pub fn draw(&mut self, app: &App) -> Result<()> {
        self.terminal.draw(|f| {
            let chunks = ratatui::layout::Layout::default()
                .direction(ratatui::layout::Direction::Horizontal)
                .constraints([
                    ratatui::layout::Constraint::Percentage(60),
                    ratatui::layout::Constraint::Percentage(40),
                ])
                .split(f.area());

            let map_block = ratatui::widgets::Block::default()
                .title("Map")
                .borders(ratatui::widgets::Borders::ALL)
                .title_alignment(ratatui::layout::Alignment::Center);

            let log_block = ratatui::widgets::Block::default()
                .title("Log")
                .borders(ratatui::widgets::Borders::ALL)
                .title_alignment(ratatui::layout::Alignment::Center);

            f.render_widget(map_block, chunks[0]);
            f.render_widget(log_block, chunks[1]);

            let info = format!("offset=({:.2},{:.2}) scale={:.2} heads={}", app.viewport_offset.0, app.viewport_offset.1, app.viewport_scale, app.dag.heads.len());
            let paragraph = ratatui::widgets::Paragraph::new(info)
                .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL).title("Status"));
            f.render_widget(paragraph, chunks[1]);
        })?;

        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn map_event(evt: CEvent) -> Option<Action> {
    match evt {
        CEvent::Key(KeyEvent { code: KeyCode::Char('q'), modifiers, .. }) if modifiers.is_empty() => Some(Action::Quit),
        CEvent::Key(KeyEvent { code: KeyCode::Up, .. }) => Some(Action::PanMap { dx: 0.0, dy: -1.0 }),
        CEvent::Key(KeyEvent { code: KeyCode::Down, .. }) => Some(Action::PanMap { dx: 0.0, dy: 1.0 }),
        CEvent::Key(KeyEvent { code: KeyCode::Left, .. }) => Some(Action::PanMap { dx: -1.0, dy: 0.0 }),
        CEvent::Key(KeyEvent { code: KeyCode::Right, .. }) => Some(Action::PanMap { dx: 1.0, dy: 0.0 }),
        CEvent::Key(KeyEvent { code: KeyCode::Char('+'), modifiers, .. }) if modifiers.contains(KeyModifiers::SHIFT) => Some(Action::ZoomMap { factor: 1.1 }),
        CEvent::Key(KeyEvent { code: KeyCode::Char('-'), .. }) => Some(Action::ZoomMap { factor: 0.9 }),
        CEvent::Key(KeyEvent { code: KeyCode::Char(' '), .. }) => Some(Action::Broadcast("Hello from Node".to_string())),
        CEvent::Resize(w, h) => Some(Action::Resize(w, h)),
        _ => None,
    }
}
