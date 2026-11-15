//! Interactive TUI project selector using ratatui.
//!
//! Provides a terminal-based UI for browsing and selecting Governor-managed
//! projects. Used when `governor start` is called without a project name.

use crate::project::{discover_projects, Project, ProjectError};
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    DefaultTerminal, Frame,
};
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TuiError {
    #[error("project discovery failed: {0}")]
    Project(#[from] ProjectError),
    #[error("terminal error: {0}")]
    Terminal(#[from] io::Error),
    #[error("no project selected (cancelled)")]
    Cancelled,
}

/// Run the interactive project selector.
///
/// Returns the selected project, or `TuiError::Cancelled` if the user quits.
pub fn select_project() -> Result<Project, TuiError> {
    let projects = discover_projects()?;
    if projects.is_empty() {
        return Err(TuiError::Project(ProjectError::NoProjectsFound));
    }

    let mut terminal = ratatui::init();
    terminal.clear()?;

    let result = run_app(&mut terminal, &projects);

    ratatui::restore();
    result
}

/// Application state for the TUI.
struct App<'a> {
    projects: &'a [Project],
    list_state: ListState,
    selected_index: Option<usize>,
}

impl<'a> App<'a> {
    fn new(projects: &'a [Project]) -> Self {
        let mut list_state = ListState::default();
        if !projects.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            projects,
            list_state,
            selected_index: None,
        }
    }

    fn next(&mut self) {
        let i = self.list_state.selected().map_or(0, |i| {
            if i >= self.projects.len().saturating_sub(1) {
                0
            } else {
                i + 1
            }
        });
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = self.list_state.selected().map_or(0, |i| {
            if i == 0 {
                self.projects.len().saturating_sub(1)
            } else {
                i - 1
            }
        });
        self.list_state.select(Some(i));
    }

    fn select_current(&mut self) {
        self.selected_index = self.list_state.selected();
    }
}

fn run_app(terminal: &mut DefaultTerminal, projects: &[Project]) -> Result<Project, TuiError> {
    let mut app = App::new(projects);

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Some(idx) = app.selected_index {
            return Ok(app.projects[idx].clone());
        }

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Err(TuiError::Cancelled),
                KeyCode::Char('j') | KeyCode::Down => app.next(),
                KeyCode::Char('k') | KeyCode::Up => app.previous(),
                KeyCode::Enter => app.select_current(),
                _ => {}
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // ── Title ────────────────────────────────────────────────────
    let title_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Governor v2 — Project Selector ")
        .title_alignment(ratatui::layout::Alignment::Center);

    let inner = title_block.inner(area);
    f.render_widget(title_block, area);

    // ── Split into list + details ────────────────────────────────
    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(inner);

    // ── Project list ─────────────────────────────────────────────
    let items: Vec<ListItem> = app
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let lock_icon = if p.is_locked() { "🔒" } else { "  " };
            let has_impl = p.implementation_file.is_some();
            let impl_icon = if has_impl { "📄" } else { "  " };
            let text = format!(" {} {}  {}", lock_icon, impl_icon, p.name);

            if Some(i) == app.list_state.selected() {
                ListItem::new(Line::from(vec![
                    Span::styled(" ▶ ", Style::default().fg(Color::Cyan)),
                    Span::styled(text, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                ]))
            } else {
                ListItem::new(Line::from(vec![
                    Span::raw("   "),
                    Span::raw(text),
                ]))
            }
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Projects "))
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(list, chunks[0], &mut app.list_state);

    // ── Details panel ────────────────────────────────────────────
    let detail_text = if let Some(idx) = app.list_state.selected() {
        let p = &app.projects[idx];
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Project: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(&p.name),
            ]),
            Line::from(vec![
                Span::styled("Path:    ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(p.root.display().to_string()),
            ]),
            Line::from(vec![
                Span::styled("Status:  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                if p.is_locked() {
                    Span::styled("LOCKED (session active)", Style::default().fg(Color::Red))
                } else {
                    Span::styled("available", Style::default().fg(Color::Green))
                },
            ]),
            Line::from(vec![
                Span::styled("Contract:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                if p.implementation_file.is_some() {
                    Span::styled("IMPLEMENTATION.md present", Style::default().fg(Color::Green))
                } else {
                    Span::styled("no implementation contract", Style::default().fg(Color::Yellow))
                },
            ]),
        ];

        // Try to show first line of STATE.md as a preview.
        if let Ok(state) = p.read_state() {
            let first_line = state.lines().next().unwrap_or("(empty)");
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Preview: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(truncate(first_line, 60)),
            ]));
        }

        Text::from(lines)
    } else {
        Text::from("No projects found.")
    };

    let details = Paragraph::new(detail_text)
        .block(Block::default().borders(Borders::ALL).title(" Details "))
        .style(Style::default().fg(Color::Gray));

    f.render_widget(details, chunks[1]);

    // ── Help bar ─────────────────────────────────────────────────
    let help = Paragraph::new(Line::from(vec![
        Span::styled(" j/k or ↑/↓ ", Style::default().fg(Color::Cyan)),
        Span::raw("navigate  "),
        Span::styled("Enter ", Style::default().fg(Color::Cyan)),
        Span::raw("select  "),
        Span::styled("q/Esc ", Style::default().fg(Color::Cyan)),
        Span::raw("quit"),
    ]))
    .style(Style::default().fg(Color::DarkGray));

    let help_area = Rect {
        x: area.x + 2,
        y: area.y + area.height.saturating_sub(1),
        width: area.width.saturating_sub(4),
        height: 1,
    };
    f.render_widget(help, help_area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max).collect::<String>())
    } else {
        s.to_string()
    }
}