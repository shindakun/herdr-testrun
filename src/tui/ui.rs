//! Drawing: header, the row list, the detail panel, and a status line.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use super::app::{App, Row};
use super::keys::HINT;

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const DETAIL_LINES: u16 = 8;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let detail_height = if app.expanded {
        Constraint::Percentage(50)
    } else {
        Constraint::Length(DETAIL_LINES + 1)
    };
    let [header, list, detail, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        detail_height,
        Constraint::Length(1),
    ])
    .areas(area);

    frame.render_widget(Paragraph::new(header_line(app)), header);
    draw_list(frame, app, list);
    draw_detail(frame, app, detail);
    draw_footer(frame, app, footer);
}

fn header_line(app: &App) -> Line<'static> {
    let name = app
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| app.root.display().to_string());
    let mut spans = vec![
        Span::styled(" Tests ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" {name}  ")),
    ];
    if app.running.is_some() {
        spans.push(Span::styled(
            format!("{} running", SPINNER[app.tick % SPINNER.len()]),
            Style::default().fg(Color::Yellow),
        ));
    } else if app.results.is_empty() {
        spans.push(Span::styled(
            "no run yet",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        let (passed, failed, skipped) = app.totals();
        let builds = app
            .results
            .iter()
            .filter(|r| r.build_error.is_some())
            .count();
        if builds > 0 {
            spans.push(Span::styled(
                format!(
                    "! {builds} build error{}  ",
                    if builds == 1 { "" } else { "s" }
                ),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        }
        if failed > 0 {
            spans.push(Span::styled(
                format!("✗ {failed} failed  "),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        } else if builds == 0 {
            spans.push(Span::styled(
                "✓ all passed  ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::raw(format!("{passed} passed  ")));
        if skipped > 0 {
            spans.push(Span::raw(format!("{skipped} skipped  ")));
        }
        spans.push(Span::styled(
            format!("{:.1}s", app.total_duration().as_secs_f64()),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if app.queued.is_some() {
        spans.push(Span::styled(
            "  +1 queued",
            Style::default().fg(Color::DarkGray),
        ));
    }
    let watch = app.watch_label();
    if !watch.is_empty() {
        spans.push(Span::styled(
            format!("  {watch}"),
            Style::default().fg(Color::Cyan),
        ));
    }
    Line::from(spans)
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default().borders(Borders::TOP);
    app.list_area = block.inner(area);
    let many = app.results.len() > 1;
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| ListItem::new(row_line(app, *row, many)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, area, &mut app.list);
}

fn row_line(app: &App, row: Row, many: bool) -> Line<'static> {
    match row {
        Row::Build(i) => {
            let r = &app.results[i];
            Line::from(vec![
                Span::styled(" ! ", Style::default().fg(Color::Red)),
                Span::raw(format!("{}: ", r.adapter)),
                Span::raw(
                    r.build_error
                        .as_deref()
                        .and_then(|e| e.lines().find(|l| !l.starts_with('#')))
                        .unwrap_or("build error")
                        .to_string(),
                ),
            ])
        }
        Row::Failure(i) => {
            let f = &app.failures[i];
            let mut spans = vec![Span::styled(" ✗ ", Style::default().fg(Color::Red))];
            if many {
                spans.push(Span::styled(
                    format!("{} ", f.adapter),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans.push(Span::raw(f.name.clone()));
            if let Some(file) = &f.file {
                let loc = match f.line {
                    Some(l) => format!("  {}:{l}", file.display()),
                    None => format!("  {}", file.display()),
                };
                spans.push(Span::styled(loc, Style::default().fg(Color::DarkGray)));
            }
            Line::from(spans)
        }
        Row::Summary => {
            let (passed, _, skipped) = app.totals();
            let mut text = format!("{passed} passed");
            if skipped > 0 {
                text.push_str(&format!(", {skipped} skipped"));
            }
            Line::from(vec![
                Span::styled(" ✓ ", Style::default().fg(Color::Green)),
                Span::styled(text, Style::default().fg(Color::DarkGray)),
            ])
        }
    }
}

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::TOP);
    let inner = block.inner(area);
    let rows = inner.height as usize;
    let text: Text = if app.running.is_some() {
        // The tail of the stream, as many lines as fit.
        let skip = app.output.len().saturating_sub(rows);
        app.output
            .iter()
            .skip(skip)
            .map(|l| Line::raw(l.trim_end().to_string()))
            .collect::<Vec<_>>()
            .into()
    } else {
        match app.selected_row() {
            Some(Row::Build(i)) => {
                Text::raw(app.results[i].build_error.clone().unwrap_or_default())
            }
            Some(Row::Failure(i)) => {
                let f = &app.failures[i];
                let mut lines = vec![Line::styled(
                    format!("{}: {}", f.adapter, f.name),
                    Style::default().add_modifier(Modifier::BOLD),
                )];
                lines.extend(f.output.lines().map(|l| Line::raw(l.to_string())));
                Text::from(lines)
            }
            Some(Row::Summary) => app
                .results
                .iter()
                .map(|r| {
                    Line::raw(format!(
                        "{}: {} passed, {} failed, {} skipped in {:.1}s",
                        r.adapter,
                        r.passed,
                        r.failed,
                        r.skipped,
                        r.duration.as_secs_f64()
                    ))
                })
                .collect::<Vec<_>>()
                .into(),
            None => Text::styled("press r to run", Style::default().fg(Color::DarkGray)),
        }
    };
    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// The status message on the left, the key hint on the right. A message
/// that leaves no room for the hint stands alone.
fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let left = format!(" {}", app.status.as_deref().unwrap_or_default());
    let width = area.width as usize;
    let used = left.chars().count() + HINT.len() + 2;
    let mut spans = vec![Span::raw(left.clone())];
    if used <= width {
        spans.push(Span::raw(" ".repeat(width - used + 1)));
        spans.push(Span::styled(HINT, Style::default().fg(Color::DarkGray)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
