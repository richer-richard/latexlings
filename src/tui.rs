//! The watch/list TUI, in the spirit of rustlings' watch mode.

use crate::check_all;
use crate::info::{self, Exercise, MARKER};
use crate::verify::{self, verify, LintResult, Status};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const ACCENT: Color = Color::Green;

#[derive(PartialEq)]
enum UiMode {
    Watch,
    List,
    CheckAll,
}

#[derive(PartialEq, Clone, Copy)]
enum ListFilter {
    None,
    Done,
    Pending,
}

pub struct App {
    root: PathBuf,
    exercises: Vec<Exercise>,
    done: info::DoneState,
    current: usize,
    status: Option<Status>,
    lints: Option<LintResult>,
    show_hint: bool,
    scroll: u16,
    mode: UiMode,
    list_state: TableState,
    last_mtime: Option<SystemTime>,
    dirty: bool,
    flash: Option<String>,
    check_results: Vec<Option<Status>>,
    check_rx: Option<std::sync::mpsc::Receiver<crate::check_all::JobResult>>,
    list_filter: ListFilter,
    search: Option<String>,
    editor_config: crate::editor::EditorConfig,
    last_opened: Option<usize>,
}

impl App {
    pub fn new(
        root: PathBuf,
        exercises: Vec<Exercise>,
        editor_config: crate::editor::EditorConfig,
    ) -> Self {
        let done = info::load_done(&root);
        let current = exercises
            .iter()
            .position(|e| !done.contains(&e.name))
            .unwrap_or(exercises.len());
        let mut list_state = TableState::default();
        list_state.select(Some(current.min(exercises.len().saturating_sub(1))));
        Self {
            root,
            exercises,
            done,
            current,
            status: None,
            lints: None,
            show_hint: false,
            scroll: 0,
            mode: UiMode::Watch,
            list_state,
            last_mtime: None,
            dirty: true,
            flash: None,
            check_results: Vec::new(),
            check_rx: None,
            list_filter: ListFilter::None,
            search: None,
            editor_config,
            last_opened: None,
        }
    }

    fn all_done(&self) -> bool {
        self.current >= self.exercises.len()
    }

    fn cur(&self) -> Option<&Exercise> {
        self.exercises.get(self.current)
    }

    fn mtime(&self) -> Option<SystemTime> {
        self.cur()
            .and_then(|e| std::fs::metadata(e.path(&self.root)).ok())
            .and_then(|m| m.modified().ok())
    }

    fn advance(&mut self) {
        let next = self
            .exercises
            .iter()
            .enumerate()
            .skip(self.current + 1)
            .find(|(_, e)| !self.done.contains(&e.name))
            .map(|(i, _)| i)
            .or_else(|| {
                self.exercises
                    .iter()
                    .position(|e| !self.done.contains(&e.name))
            });
        self.current = next.unwrap_or(self.exercises.len());
        self.status = None;
        self.lints = None;
        self.show_hint = false;
        self.scroll = 0;
        self.last_mtime = None;
        self.dirty = !self.all_done();
    }

    fn visible_rows(&self) -> Vec<usize> {
        self.exercises
            .iter()
            .enumerate()
            .filter(|(_, e)| match self.list_filter {
                ListFilter::None => true,
                ListFilter::Done => self.done.contains(&e.name),
                ListFilter::Pending => !self.done.contains(&e.name),
            })
            .map(|(i, _)| i)
            .collect()
    }
}

pub fn run_watch(
    root: PathBuf,
    exercises: Vec<Exercise>,
    editor_config: crate::editor::EditorConfig,
) -> Result<()> {
    let mut terminal = ratatui::init();
    let app = App::new(root, exercises, editor_config);
    let result = event_loop(&mut terminal, app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> Result<()> {
    loop {
        if app.dirty && !app.all_done() {
            app.flash = Some("compiling…".into());
            terminal.draw(|f| draw(f, &mut app))?;
            let ex = app.cur().unwrap().clone();
            if app.last_opened != Some(app.current) {
                crate::editor::open(&app.editor_config, &ex.path(&app.root));
                app.last_opened = Some(app.current);
            }
            let (status, lints) = verify::verify_with_lints(&app.root, &ex);
            if status.is_done() {
                let mtime = app.mtime().map(info::truncate_to_secs);
                let previous_mtime = app.done.mtime(&ex.name);
                let is_new = app.done.insert(ex.name.clone(), mtime);
                if is_new || previous_mtime != mtime {
                    info::save_done(&app.root, &app.done)?;
                }
            }
            app.status = Some(status);
            app.lints = lints;
            app.flash = None;
            app.last_mtime = app.mtime();
            app.dirty = false;
        }
        terminal.draw(|f| draw(f, &mut app))?;

        if let Some(rx) = &app.check_rx {
            let mut disconnected = false;
            loop {
                match rx.try_recv() {
                    Ok(result) => app.check_results[result.index] = Some(result.status),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            if disconnected {
                app.check_rx = None;
            }
        }

        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let ctrl_c = key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL);
                    if ctrl_c {
                        return Ok(());
                    }
                    match app.mode {
                        UiMode::Watch => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Char('n') => {
                                if matches!(app.status, Some(Status::Done)) {
                                    app.advance();
                                }
                            }
                            KeyCode::Char('h') => app.show_hint = !app.show_hint,
                            KeyCode::Char('r') => app.dirty = true,
                            KeyCode::Char('l') => {
                                app.list_state.select(Some(
                                    app.current.min(app.exercises.len().saturating_sub(1)),
                                ));
                                app.mode = UiMode::List;
                            }
                            KeyCode::Char('c') => {
                                app.check_results = vec![None; app.exercises.len()];
                                let (jobs, cached) =
                                    check_all::plan_sweep(&app.root, &app.exercises, &app.done);
                                for (i, status) in cached {
                                    app.check_results[i] = Some(status);
                                }
                                let verifier: check_all::Verifier = std::sync::Arc::new(verify);
                                app.check_rx = Some(check_all::spawn(jobs, verifier));
                                app.mode = UiMode::CheckAll;
                            }
                            KeyCode::Up => app.scroll = app.scroll.saturating_sub(1),
                            KeyCode::Down => app.scroll = app.scroll.saturating_add(1),
                            KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
                            KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10),
                            _ => {}
                        },
                        UiMode::List if app.search.is_some() => match key.code {
                            KeyCode::Esc | KeyCode::Enter => app.search = None,
                            KeyCode::Backspace => {
                                if let Some(q) = &mut app.search {
                                    q.pop();
                                }
                            }
                            KeyCode::Char(c) => {
                                if let Some(q) = &mut app.search {
                                    q.push(c);
                                }
                                let rows = app.visible_rows();
                                if let Some(q) = &app.search {
                                    if let Some(pos) = rows
                                        .iter()
                                        .position(|&i| app.exercises[i].name.contains(q.as_str()))
                                    {
                                        app.list_state.select(Some(pos));
                                    }
                                }
                            }
                            _ => {}
                        },
                        UiMode::List => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('l') => {
                                app.mode = UiMode::Watch
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let i = app.list_state.selected().unwrap_or(0);
                                app.list_state.select(Some(i.saturating_sub(1)));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let rows = app.visible_rows();
                                let i = app.list_state.selected().unwrap_or(0);
                                app.list_state
                                    .select(Some((i + 1).min(rows.len().saturating_sub(1))));
                            }
                            KeyCode::Enter => {
                                let rows = app.visible_rows();
                                if let Some(i) =
                                    app.list_state.selected().and_then(|i| rows.get(i)).copied()
                                {
                                    app.current = i;
                                    if app.last_opened != Some(app.current) {
                                        crate::editor::open(
                                            &app.editor_config,
                                            &app.exercises[i].path(&app.root),
                                        );
                                        app.last_opened = Some(app.current);
                                    }
                                    app.status = None;
                                    app.lints = None;
                                    app.scroll = 0;
                                    app.show_hint = false;
                                    app.dirty = true;
                                    app.mode = UiMode::Watch;
                                }
                            }
                            KeyCode::Char('r') => {
                                let rows = app.visible_rows();
                                if let Some(i) =
                                    app.list_state.selected().and_then(|i| rows.get(i)).copied()
                                {
                                    let ex = app.exercises[i].clone();
                                    info::reset(&app.root, &ex)?;
                                    app.done.remove(&ex.name);
                                    info::save_done(&app.root, &app.done)?;
                                    if i == app.current {
                                        app.dirty = true;
                                    }
                                }
                            }
                            KeyCode::Char('d') => {
                                app.list_filter = if app.list_filter == ListFilter::Done {
                                    ListFilter::None
                                } else {
                                    ListFilter::Done
                                };
                                app.list_state.select(Some(0));
                            }
                            KeyCode::Char('p') => {
                                app.list_filter = if app.list_filter == ListFilter::Pending {
                                    ListFilter::None
                                } else {
                                    ListFilter::Pending
                                };
                                app.list_state.select(Some(0));
                            }
                            KeyCode::Char('/') => {
                                app.search = Some(String::new());
                            }
                            _ => {}
                        },
                        UiMode::CheckAll => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter
                                if app.check_rx.is_none() =>
                            {
                                // Sweep finished — reconcile and return to Watch.
                                for (i, status) in app.check_results.iter().enumerate() {
                                    if let Some(status) = status {
                                        if status.is_done() {
                                            let ex = &app.exercises[i];
                                            let mtime = std::fs::metadata(ex.path(&app.root))
                                                .and_then(|m| m.modified())
                                                .ok()
                                                .map(info::truncate_to_secs);
                                            app.done.insert(ex.name.clone(), mtime);
                                        }
                                    }
                                }
                                info::save_done(&app.root, &app.done)?;
                                app.current = app
                                    .exercises
                                    .iter()
                                    .position(|e| !app.done.contains(&e.name))
                                    .unwrap_or(app.exercises.len());
                                app.status = None;
                                app.lints = None;
                                app.dirty = !app.all_done();
                                app.mode = UiMode::Watch;
                            }
                            _ => {}
                        },
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        } else if app.mode == UiMode::Watch && !app.all_done() {
            // tick: watch the current file for changes
            let now = app.mtime();
            if now.is_some() && now != app.last_mtime {
                app.dirty = true;
            }
        }
    }
}

fn draw(frame: &mut Frame, app: &mut App) {
    let [header, main, footer] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(4), Constraint::Length(1)])
            .areas(frame.area());
    draw_progress(frame, header, app);
    match app.mode {
        UiMode::Watch => draw_watch(frame, main, app),
        UiMode::List => draw_list(frame, main, app),
        UiMode::CheckAll => draw_check_all(frame, main, app),
    }
    draw_footer(frame, footer, app);
}

fn draw_progress(frame: &mut Frame, area: Rect, app: &App) {
    let done = app.done.len();
    let total = app.exercises.len();
    let ratio = if total == 0 { 0.0 } else { done as f64 / total as f64 };
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " latexlings ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))
                .title(Line::from(format!(" {done}/{total} done ")).right_aligned()),
        )
        .gauge_style(Style::default().fg(ACCENT))
        .ratio(ratio)
        .label(format!("{:.0}%", ratio * 100.0));
    frame.render_widget(gauge, area);
}

fn draw_watch(frame: &mut Frame, area: Rect, app: &App) {
    if app.all_done() {
        let text = Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ✓ ALL EXERCISES COMPLETE",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("  You have typeset your way through the whole course."),
            Line::from("  Write something real now — a problem set, a paper, a resume."),
            Line::from(""),
            Line::from("  (l: browse exercises · r in the list resets one · q: quit)"),
        ]);
        frame.render_widget(
            Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }
    let ex = app.cur().unwrap();
    let title = format!(" {} [{}] ", ex.rel_path(), ex.mode.label());
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    match (&app.flash, &app.status) {
        (Some(msg), _) => {
            lines.push(Line::from(Span::styled(
                format!("  ⟳ {msg}"),
                Style::default().fg(Color::Yellow),
            )));
        }
        (None, Some(Status::Done)) => {
            lines.push(Line::from(Span::styled(
                "  ✓ exercise complete — press n to continue",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )));
        }
        (None, Some(Status::MarkerPresent)) => {
            lines.push(Line::from(Span::styled(
                "  ✓ compiles and passes all checks!",
                Style::default().fg(ACCENT),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "    Read the output, then delete the `% {MARKER}` line to finish."
            )));
        }
        (None, Some(Status::CompileFail(err))) => {
            lines.push(Line::from(Span::styled(
                "  ✗ pdflatex failed:",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for l in err.lines() {
                lines.push(Line::from(Span::styled(
                    format!("    {l}"),
                    Style::default().fg(Color::Red),
                )));
            }
        }
        (None, Some(Status::ChecksFail(notes, excerpt))) => {
            lines.push(Line::from(Span::styled(
                "  ✗ compiles, but the rendered output isn't right yet:",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for n in notes {
                lines.push(Line::from(Span::styled(
                    format!("    • {n}"),
                    Style::default().fg(Color::Yellow),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "    rendered text starts with:",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                format!("    {excerpt}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
        (None, Some(Status::ToolMissing(msg))) => {
            lines.push(Line::from(Span::styled(
                "  ✗ missing tool:",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            for l in msg.lines() {
                lines.push(Line::from(format!("    {l}")));
            }
        }
        (None, None) => {
            lines.push(Line::from("  waiting for first compile…"));
        }
    }
    if app.show_hint {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ── hint ────────────────────────────────",
            Style::default().fg(Color::Cyan),
        )));
        for l in ex.hint.lines() {
            lines.push(Line::from(Span::styled(
                format!("  {l}"),
                Style::default().fg(Color::Cyan),
            )));
        }
    }
    if let Some(lints) = &app.lints {
        if !lints.notes.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ── chktex notes ────────────────────────",
                Style::default().fg(Color::Cyan),
            )));
            for n in &lints.notes {
                lines.push(Line::from(Span::styled(
                    format!("  • {n}"),
                    Style::default().fg(Color::Cyan),
                )));
            }
        }
    }
    let para = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, Style::default().add_modifier(Modifier::BOLD))),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    frame.render_widget(para, area);
}

fn draw_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let rows: Vec<Row> = app
        .visible_rows()
        .into_iter()
        .map(|i| {
            let e = &app.exercises[i];
            let done = app.done.contains(&e.name);
            let icon = if done {
                Span::styled("✓", Style::default().fg(ACCENT))
            } else if i == app.current {
                Span::styled("→", Style::default().fg(Color::Yellow))
            } else {
                Span::raw("·")
            };
            Row::new(vec![
                Cell::from(icon),
                Cell::from(format!("{:>3}", i + 1)),
                Cell::from(e.name.clone()),
                Cell::from(e.dir.clone()),
                Cell::from(e.mode.label()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Length(24),
            Constraint::Length(22),
            Constraint::Length(6),
        ],
    )
    .header(
        Row::new(vec!["", "#", "exercise", "topic", "mode"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .row_highlight_style(Style::default().bg(Color::DarkGray))
    .block(Block::default().borders(Borders::ALL).title(" exercises "));
    frame.render_stateful_widget(table, area, &mut app.list_state);
}

fn draw_check_all(frame: &mut Frame, area: Rect, app: &App) {
    let done_now =
        app.check_results.iter().filter(|s| matches!(s, Some(st) if st.is_done())).count();
    let checked = app.check_results.iter().filter(|s| s.is_some()).count();
    let total = app.exercises.len();
    let title = if app.check_rx.is_some() {
        format!(" checking all — {checked}/{total} ")
    } else {
        format!(" check all complete — {done_now}/{total} done (press enter) ")
    };
    let mut spans: Vec<Span> = Vec::new();
    for status in &app.check_results {
        let (ch, color) = match status {
            None => ('░', Color::DarkGray),
            Some(s) if s.is_done() => ('█', ACCENT),
            Some(_) => ('█', Color::Red),
        };
        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
    }
    let para = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let keys = match app.mode {
        UiMode::Watch => "  n:next  h:hint  l:list  c:check-all  r:recompile  ↑↓:scroll  q:quit",
        UiMode::List => "  ↑↓/jk:move  enter:work on this  r:reset  d:done  p:pending  /:search  esc:back",
        UiMode::CheckAll => "  (running…)  q/esc/enter:back once done",
    };
    frame.render_widget(
        Paragraph::new(Span::styled(keys, Style::default().fg(Color::DarkGray))),
        area,
    );
}
