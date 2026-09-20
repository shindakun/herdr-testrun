//! Key and mouse handling.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::app::{App, Job, Scope};

pub const HINT: &str = "r run  f failed  a agent  w watch  enter expand  o log  q quit";

pub fn key(app: &mut App, key: KeyEvent) {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.quit = true,
        (KeyCode::Char('r'), _) => {
            let msg = app.request(Job::manual(Scope::All));
            app.status = Some(msg);
        }
        (KeyCode::Char('f'), _) => {
            app.status = Some(match app.failed_scope() {
                Some(scope) => app.request(Job::manual(scope)),
                None => "nothing failed in the last run".into(),
            });
        }
        (KeyCode::Char('a'), _) => {
            app.send_to_agent();
        }
        (KeyCode::Char('w'), _) => app.cycle_watch(),
        (KeyCode::Char('o'), _) => app.open_log(),
        (KeyCode::Enter, _) => app.expanded = !app.expanded,
        (KeyCode::Char('j') | KeyCode::Down, _) => app.move_selection(1),
        (KeyCode::Char('k') | KeyCode::Up, _) => app.move_selection(-1),
        (KeyCode::Char('g') | KeyCode::Home, _) => app.select(0),
        (KeyCode::Char('G') | KeyCode::End, _) => app.select(usize::MAX),
        (KeyCode::PageDown, _) => app.move_selection(10),
        (KeyCode::PageUp, _) => app.move_selection(-10),
        _ => {}
    }
}

pub fn mouse(app: &mut App, m: MouseEvent) {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let a = app.list_area;
            if m.column >= a.x && m.column < a.x + a.width && m.row >= a.y && m.row < a.y + a.height
            {
                let index = app.list.offset() + (m.row - a.y) as usize;
                if index < app.rows.len() {
                    app.select(index);
                }
            }
        }
        MouseEventKind::ScrollDown => app.move_selection(1),
        MouseEventKind::ScrollUp => app.move_selection(-1),
        _ => {}
    }
}
