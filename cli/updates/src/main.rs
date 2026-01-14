use anyhow::Result;
use chrono::Local;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use ratatui::{
    backend::CrosstermBackend,
    layout::Constraint,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Terminal,
};
use std::{
    collections::{HashMap, VecDeque},
    env,
    fs,
    io,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

const MAX_EVENTS: usize = 100;
const DEDUP_WINDOW_MS: u128 = 100;

#[derive(Clone)]
struct FileEvent {
    timestamp: String,
    operation: String,
    file_type: String,
    path: String,
    size: Option<u64>,
}

struct App {
    events: VecDeque<FileEvent>,
    selected: usize,
    recent: HashMap<String, (Instant, String)>, // path -> (time, operation)
}

impl App {
    fn new() -> Self {
        Self {
            events: VecDeque::new(),
            selected: 0,
            recent: HashMap::new(),
        }
    }

    fn add_event(&mut self, event: FileEvent) -> bool {
        let now = Instant::now();

        // Check for duplicate within time window
        if let Some((last_time, last_op)) = self.recent.get(&event.path) {
            if now.duration_since(*last_time).as_millis() < DEDUP_WINDOW_MS {
                // CREATE takes priority over MODIFY
                if *last_op == "CREATE" && event.operation == "MODIFY" {
                    return false; // Skip this MODIFY, we already have CREATE
                }
                // DELETE takes priority - remove from recent and allow
                if event.operation != "DELETE" && *last_op != "DELETE" {
                    return false; // Skip duplicate
                }
            }
        }

        self.recent.insert(event.path.clone(), (now, event.operation.clone()));
        self.events.push_front(event);
        if self.events.len() > MAX_EVENTS {
            self.events.pop_back();
        }
        true
    }

    fn move_selection(&mut self, delta: i32) {
        if self.events.is_empty() {
            return;
        }
        let new = (self.selected as i32 + delta).clamp(0, self.events.len() as i32 - 1);
        self.selected = new as usize;
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}b", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{}kb", bytes / 1024)
    } else {
        format!("{:.1}mb", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn main() -> Result<()> {
    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap());

    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path.display());
    }

    let (tx, rx) = mpsc::channel();

    let tx_clone = tx.clone();
    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx_clone.send(event);
            }
        },
        Config::default(),
    )?;

    watcher.watch(&path, RecursiveMode::Recursive)?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    loop {
        while let Ok(event) = rx.try_recv() {
            for event_path in event.paths {
                let relative_path = event_path
                    .strip_prefix(&env::current_dir().unwrap_or_default())
                    .unwrap_or(&event_path)
                    .display()
                    .to_string();

                // Editors do atomic saves: write temp → rename to original
                // This shows as CREATE but should be MODIFY if file existed before
                let operation = match event.kind {
                    notify::EventKind::Create(_) => {
                        if app.recent.contains_key(&relative_path) {
                            "MODIFY" // We've seen this file, so it's a replace
                        } else {
                            "CREATE"
                        }
                    }
                    notify::EventKind::Modify(_) => "MODIFY",
                    notify::EventKind::Remove(_) => "DELETE",
                    notify::EventKind::Access(_) => continue,
                    _ => continue,
                };

                let file_type = if event_path.is_dir() {
                    "folder"
                } else {
                    "file"
                };

                let size = if event_path.is_file() && operation != "DELETE" {
                    fs::metadata(&event_path).ok().map(|m| m.len())
                } else {
                    None
                };

                app.add_event(FileEvent {
                    timestamp: Local::now().format("%H:%M:%S%.3f").to_string(),
                    operation: operation.to_string(),
                    file_type: file_type.to_string(),
                    path: relative_path,
                    size,
                });
            }
        }

        terminal.draw(|f| {
            let area = f.area();

            // Header row
            let header = Row::new(vec![
                Cell::from("TIMESTAMP").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
                Cell::from("OPERATION").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
                Cell::from("TYPE").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
                Cell::from("PATH").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
                Cell::from("SIZE").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            ])
            .height(1)
            .bottom_margin(1);

            let rows: Vec<Row> = app
                .events
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let (op_color, op_bg) = match e.operation.as_str() {
                        "CREATE" => (Color::Black, Color::Green),
                        "MODIFY" => (Color::Black, Color::Yellow),
                        "DELETE" => (Color::Black, Color::Rgb(255, 100, 100)), // Light red, readable
                        _ => (Color::White, Color::DarkGray),
                    };

                    let type_icon = if e.file_type == "folder" { "󰉋" } else { "󰈙" };

                    let size_str = e.size.map(format_size).unwrap_or_else(|| "–".to_string());

                    let row = Row::new(vec![
                        Cell::from(e.timestamp.clone()).style(Style::default().fg(Color::DarkGray)),
                        Cell::from(format!(" {} ", e.operation)).style(
                            Style::default()
                                .fg(op_color)
                                .bg(op_bg)
                                .add_modifier(Modifier::BOLD)
                        ),
                        Cell::from(format!("{} {}", type_icon, e.file_type)).style(Style::default().fg(Color::Gray)),
                        Cell::from(e.path.clone()).style(Style::default().fg(Color::Green)),
                        Cell::from(size_str).style(Style::default().fg(Color::DarkGray)),
                    ]);

                    if i == app.selected {
                        row.style(Style::default().bg(Color::Rgb(30, 30, 30)))
                    } else {
                        row
                    }
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(14),  // TIMESTAMP
                    Constraint::Length(10),  // OPERATION
                    Constraint::Length(10),  // TYPE
                    Constraint::Min(30),     // PATH
                    Constraint::Length(10),  // SIZE
                ],
            )
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
            )
            .row_highlight_style(Style::default().bg(Color::Rgb(30, 30, 30)));

            f.render_widget(table, area);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
                    KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
                    KeyCode::Home => app.selected = 0,
                    KeyCode::End => {
                        if !app.events.is_empty() {
                            app.selected = app.events.len() - 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}
