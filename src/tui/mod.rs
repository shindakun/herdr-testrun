//! The Tests pane: terminal setup, the control socket, and the event loop.
//! Layout and keys are in docs/PLAN.md.

mod app;
mod keys;
mod ui;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::execute;

use crate::herdr::PluginEnv;
use crate::sock;
use app::{App, Job, Msg};

/// Spinner rate and the longest the loop waits between redraws.
const TICK: Duration = Duration::from_millis(100);

/// `pane [--dir PATH]`. Runs the tests once on start, then waits for keys
/// and socket requests.
pub fn run(root: PathBuf, env: Option<PluginEnv>) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut app = App::new(root, env, tx.clone())?;

    let listener = sock::bind(&app.state.dir.join(sock::FILE))?;
    let sock_tx = tx.clone();
    sock::serve(listener, move |req| {
        let (reply_tx, reply_rx) = mpsc::channel();
        if sock_tx.send(Msg::Request(req, reply_tx)).is_err() {
            return sock::Response::err("pane is shutting down");
        }
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| sock::Response::err("pane did not answer"))
    });

    let input_tx = tx;
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if input_tx.send(Msg::Input(ev)).is_err() {
                break;
            }
        }
    });

    let mut terminal = ratatui::try_init().map_err(|e| format!("terminal: {e}"))?;
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    app.request(Job::manual(app::Scope::All));
    let result = event_loop(&mut terminal, &mut app, &rx);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    let _ = std::fs::remove_file(app.state.dir.join(sock::FILE));
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: &mpsc::Receiver<Msg>,
) -> Result<(), String> {
    while !app.quit {
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(|e| format!("draw: {e}"))?;
        match rx.recv_timeout(TICK) {
            Ok(Msg::Input(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                keys::key(app, key)
            }
            Ok(Msg::Input(Event::Mouse(m))) => keys::mouse(app, m),
            Ok(Msg::Input(_)) => {}
            Ok(msg) => app.handle(msg),
            Err(mpsc::RecvTimeoutError::Timeout) => app.tick(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // Coalesce a burst of output lines into one redraw.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    keys::key(app, key)
                }
                Msg::Input(Event::Mouse(m)) => keys::mouse(app, m),
                Msg::Input(_) => {}
                other => app.handle(other),
            }
        }
    }
    Ok(())
}
