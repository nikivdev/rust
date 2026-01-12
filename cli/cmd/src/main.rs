use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Matcher,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write as IoWrite},
    path::PathBuf,
    process::{Command, Stdio},
};

#[derive(Parser)]
#[command(name = "cmd", about = "Fuzzy search CLI commands and options")]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// The CLI command to search (e.g., bun, cargo, git)
    #[arg(conflicts_with = "command")]
    cli: Option<String>,

    /// Force rescan even if cache exists
    #[arg(short, long)]
    refresh: bool,

    /// Just print the command, don't execute
    #[arg(short, long)]
    print_only: bool,

    /// List all entries without interactive UI
    #[arg(short, long)]
    list: bool,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Deep expand --help for a CLI and all subcommands, copy to clipboard or file
    Copy {
        /// The CLI command to expand (e.g., flow, cargo, git)
        command: String,

        /// Output file path (optional, copies to clipboard if not provided)
        path: Option<PathBuf>,

        /// Max depth for subcommand recursion (default: 3)
        #[arg(short, long, default_value = "3")]
        depth: usize,
    },
    /// AI-powered command matching using local LM Studio
    Ai {
        /// The CLI command to query (e.g., flow, cargo, git)
        command: String,

        /// LM Studio API port
        #[arg(long, default_value = "1234")]
        port: u16,

        /// Debounce delay in milliseconds
        #[arg(long, default_value = "2000")]
        debounce: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CommandInfo {
    version: String,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    /// Full command path (e.g., "bun run --watch")
    command: String,
    /// Short flag if any (e.g., "-r")
    short: Option<String>,
    /// Long flag if any (e.g., "--preload")
    long: Option<String>,
    /// Description of the command/flag
    description: String,
    /// Type: "subcommand" or "flag"
    entry_type: String,
}

impl Entry {
    fn display_text(&self) -> String {
        // Use just the subcommand part for cleaner display
        let cmd_display = self.command.split_whitespace().collect::<Vec<_>>().join(" ");

        match self.entry_type.as_str() {
            "subcommand" => format!("{} - {}", cmd_display, self.description),
            "flag" => {
                let flag_part = match (&self.short, &self.long) {
                    (Some(s), Some(l)) => format!("{}, {}", s, l),
                    (Some(s), None) => s.clone(),
                    (None, Some(l)) => l.clone(),
                    (None, None) => String::new(),
                };
                format!("{} {} - {}", cmd_display, flag_part, self.description)
            }
            _ => cmd_display,
        }
    }

    fn search_text(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.command,
            self.short.as_deref().unwrap_or(""),
            self.long.as_deref().unwrap_or(""),
            self.description,
            self.entry_type
        )
    }
}

fn get_cache_dir() -> Result<PathBuf> {
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cmd-fuzzy");
    fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir)
}

fn get_cache_path(command: &str) -> Result<PathBuf> {
    let safe_name = command.replace(['/', '\\'], "_");
    Ok(get_cache_dir()?.join(format!("{}.json", safe_name)))
}

fn get_version(command: &str) -> Result<String> {
    // Try --version first, then -V, then -v
    for flag in ["--version", "-V", "-v"] {
        if let Ok(output) = Command::new(command).arg(flag).output() {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout);
                let version = version.trim();
                if !version.is_empty() && version.len() < 200 {
                    return Ok(version.to_string());
                }
            }
        }
    }
    Ok("unknown".to_string())
}

fn get_help(command: &str, subcommands: &[&str]) -> Result<String> {
    let mut cmd = Command::new(command);
    for sub in subcommands {
        cmd.arg(sub);
    }
    cmd.arg("--help");

    let output = cmd.output().context("Failed to run command")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Some commands output help to stderr
    if stdout.len() > stderr.len() {
        Ok(stdout.to_string())
    } else {
        Ok(stderr.to_string())
    }
}

fn parse_help(command: &str, subcommands: &[&str], help_text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let base_cmd = if subcommands.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, subcommands.join(" "))
    };

    // Flag patterns
    // Standard: "  -r, --release    Build in release mode"
    // Long only: "      --verbose   Enable verbose output"
    // With value: "  -p, --port=<val>   Set port"
    let flag_re = Regex::new(
        r"^\s+(-[a-zA-Z])?\s*,?\s*(--[\w-]+(?:=<[^>]+>|=\S+)?|--[\w-]+)?\s{2,}(.+)$",
    )
    .unwrap();
    let flag_re2 = Regex::new(r"^\s+(--[\w-]+(?:=<[^>]+>)?)\s{2,}(.+)$").unwrap();
    let flag_re3 = Regex::new(r"^\s+(-[a-zA-Z])\s{2,}(.+)$").unwrap();

    let mut in_commands_section = false;
    let mut in_flags_section = false;

    // Keywords that indicate subcommands section
    let cmd_headers = [
        "commands:",
        "subcommands:",
        "available commands:",
        "main commands:",
    ];
    let flag_headers = [
        "flags:",
        "options:",
        "global options:",
        "common options:",
        "arguments:",
    ];

    for line in help_text.lines() {
        let trimmed = line.trim().to_lowercase();

        // Detect section headers
        if cmd_headers.iter().any(|h| trimmed.starts_with(h)) {
            in_commands_section = true;
            in_flags_section = false;
            continue;
        }
        if flag_headers.iter().any(|h| trimmed.starts_with(h)) {
            in_commands_section = false;
            in_flags_section = true;
            continue;
        }

        // Git-style section headers (e.g., "start a working area")
        if !trimmed.is_empty()
            && !line.starts_with(' ')
            && !trimmed.starts_with('-')
            && !trimmed.starts_with("usage")
        {
            // Likely a section header, might be commands section
            if trimmed.contains("command")
                || trimmed.contains("see also")
                || trimmed.contains("start")
                || trimmed.contains("work on")
                || trimmed.contains("examine")
                || trimmed.contains("grow")
                || trimmed.contains("collaborate")
            {
                in_commands_section = true;
                in_flags_section = false;
            }
            continue;
        }

        // Skip empty lines but don't reset section
        if trimmed.is_empty() {
            continue;
        }

        // Parse subcommands
        if in_commands_section {
            // Try to parse line as: spaces + command + spaces + [example] + spaces + description
            // Split by 2+ consecutive spaces
            let parts: Vec<&str> = line
                .split("  ")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            if parts.len() >= 2 {
                let name = parts[0];
                // Description is the last part that looks like prose (starts with uppercase or lowercase letter)
                let desc = parts
                    .iter()
                    .skip(1)
                    .filter(|p| {
                        p.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false)
                            && !p.starts_with("./")
                            && !p.starts_with("<")
                            && !p.starts_with("[")
                    })
                    .last()
                    .unwrap_or(&"");

                if !name.is_empty()
                    && name.chars().next().map(|c| c.is_alphanumeric()).unwrap_or(false)
                    && name != "help"
                    && !name.starts_with('-')
                    && !name.starts_with('<')
                    && name.len() > 1
                {
                    entries.push(Entry {
                        command: format!("{} {}", base_cmd, name),
                        short: None,
                        long: None,
                        description: desc.to_string(),
                        entry_type: "subcommand".to_string(),
                    });
                }
            }
        }

        // Parse flags
        if in_flags_section || line.trim_start().starts_with('-') {
            let mut matched = false;

            if let Some(caps) = flag_re.captures(line) {
                let short = caps
                    .get(1)
                    .map(|m| m.as_str().trim().to_string())
                    .filter(|s| !s.is_empty());
                let long = caps
                    .get(2)
                    .map(|m| m.as_str().trim().to_string())
                    .filter(|s| !s.is_empty());
                let desc = caps.get(3).map(|m| m.as_str().trim()).unwrap_or("");

                if short.is_some() || long.is_some() {
                    entries.push(Entry {
                        command: base_cmd.clone(),
                        short,
                        long,
                        description: desc.to_string(),
                        entry_type: "flag".to_string(),
                    });
                    matched = true;
                }
            }

            if !matched {
                if let Some(caps) = flag_re2.captures(line) {
                    let long = caps.get(1).map(|m| m.as_str().trim().to_string());
                    let desc = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");

                    entries.push(Entry {
                        command: base_cmd.clone(),
                        short: None,
                        long,
                        description: desc.to_string(),
                        entry_type: "flag".to_string(),
                    });
                    matched = true;
                }
            }

            if !matched {
                if let Some(caps) = flag_re3.captures(line) {
                    let short = caps.get(1).map(|m| m.as_str().trim().to_string());
                    let desc = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");

                    entries.push(Entry {
                        command: base_cmd.clone(),
                        short,
                        long: None,
                        description: desc.to_string(),
                        entry_type: "flag".to_string(),
                    });
                }
            }
        }
    }

    entries
}

fn extract_subcommand_names(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| e.entry_type == "subcommand")
        .filter_map(|e| e.command.split_whitespace().last().map(|s| s.to_string()))
        .collect()
}

fn scan_command(command: &str, max_depth: usize) -> Result<Vec<Entry>> {
    let mut all_entries = Vec::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    fn scan_recursive(
        command: &str,
        subcommands: &[&str],
        depth: usize,
        max_depth: usize,
        all_entries: &mut Vec<Entry>,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        if depth > max_depth {
            return Ok(());
        }

        let key = format!("{} {}", command, subcommands.join(" "));
        if visited.contains(&key) {
            return Ok(());
        }
        visited.insert(key);

        eprint!("\rScanning: {} {}...", command, subcommands.join(" "));
        io::stderr().flush().ok();

        let help_text = match get_help(command, subcommands) {
            Ok(text) => text,
            Err(_) => return Ok(()), // Skip if help fails
        };

        let entries = parse_help(command, subcommands, &help_text);
        let sub_names = extract_subcommand_names(&entries);

        all_entries.extend(entries);

        // Recursively scan subcommands
        for sub_name in sub_names {
            let mut new_subs: Vec<&str> = subcommands.to_vec();
            let sub_name_ref: &str = &sub_name;
            new_subs.push(sub_name_ref);

            // Need to own the strings for the recursive call
            let owned_subs: Vec<String> = new_subs.iter().map(|s| s.to_string()).collect();
            let refs: Vec<&str> = owned_subs.iter().map(|s| s.as_str()).collect();

            scan_recursive(command, &refs, depth + 1, max_depth, all_entries, visited)?;
        }

        Ok(())
    }

    scan_recursive(command, &[], 0, max_depth, &mut all_entries, &mut visited)?;
    eprintln!("\rScanned {} entries.                    ", all_entries.len());

    Ok(all_entries)
}

/// Collect deep help output for a command and all subcommands
fn collect_deep_help(command: &str, max_depth: usize) -> Result<String> {
    let mut output = String::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    fn collect_recursive(
        command: &str,
        subcommands: &[String],
        depth: usize,
        max_depth: usize,
        output: &mut String,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        if depth > max_depth {
            return Ok(());
        }

        let key = if subcommands.is_empty() {
            command.to_string()
        } else {
            format!("{} {}", command, subcommands.join(" "))
        };

        if visited.contains(&key) {
            return Ok(());
        }
        visited.insert(key.clone());

        eprint!("\rCollecting: {}...", key);
        io::stderr().flush().ok();

        // Build the command with subcommands
        let refs: Vec<&str> = subcommands.iter().map(|s| s.as_str()).collect();
        let help_text = match get_help(command, &refs) {
            Ok(text) => text,
            Err(_) => return Ok(()), // Skip if help fails
        };

        // Add section header
        let header = format!(
            "\n{}\n## {} --help\n{}\n\n",
            "=".repeat(80),
            key,
            "=".repeat(80)
        );
        output.push_str(&header);
        output.push_str(&help_text);
        output.push_str("\n");

        // Parse to find subcommands
        let entries = parse_help(command, &refs, &help_text);
        let sub_names = extract_subcommand_names(&entries);

        // Recursively collect subcommands
        for sub_name in sub_names {
            let mut new_subs = subcommands.to_vec();
            new_subs.push(sub_name);
            collect_recursive(command, &new_subs, depth + 1, max_depth, output, visited)?;
        }

        Ok(())
    }

    collect_recursive(command, &[], 0, max_depth, &mut output, &mut visited)?;
    eprintln!("\rCollected help from {} commands.        ", visited.len());

    Ok(output)
}

/// Query LM Studio to match a natural language query to a command.
fn query_lm_studio(
    query: &str,
    command: &str,
    entries: &[Entry],
    port: u16,
) -> Result<String> {
    // Build context from available commands
    let commands_list: Vec<String> = entries
        .iter()
        .filter(|e| e.entry_type == "subcommand")
        .map(|e| {
            if !e.description.is_empty() {
                format!("{} - {}", e.command, e.description)
            } else {
                e.command.clone()
            }
        })
        .collect();

    let commands_context = commands_list.join("\n");

    let system_prompt = format!(
        r#"You are a CLI command assistant. Given a natural language query, output ONLY the exact command to run.

Available commands for `{command}`:
{commands_context}

Rules:
1. Output ONLY the command, nothing else
2. Include any necessary arguments based on the query
3. If the query mentions specific values (files, names, etc), include them
4. Use the most appropriate command from the list
5. Do not explain, just output the command"#
    );

    let payload = serde_json::json!({
        "model": "qwen3-8b",
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": query}
        ],
        "temperature": 0.1,
        "max_tokens": 200,
        "stream": false
    });

    let url = format!("http://localhost:{}/v1/chat/completions", port);

    let response: serde_json::Value = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_json(&payload)
        .context("Failed to connect to LM Studio")?
        .into_json()
        .context("Failed to parse LM Studio response")?;

    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    // Clean up: remove any markdown code blocks or thinking tags
    let content = content
        .trim_start_matches("```")
        .trim_start_matches("bash")
        .trim_start_matches("sh")
        .trim_end_matches("```")
        .trim();

    // Remove <think>...</think> tags if present
    let content = if let Some(start) = content.find("<think>") {
        if let Some(end) = content.find("</think>") {
            format!("{}{}", &content[..start], &content[end + 8..])
        } else {
            content.to_string()
        }
    } else {
        content.to_string()
    };

    Ok(content.trim().to_string())
}

/// UI Mode - Search (fuzzy filter) or AI (natural language)
#[derive(PartialEq, Clone, Copy)]
enum UiMode {
    Search,
    Ai,
}

/// Result from the unified UI
enum UiResult {
    Entry(Entry),
    Command(String),
    Copied,
    Cancelled,
}

/// Run the unified search/AI UI with Tab toggle between modes.
fn run_unified_ui(
    command: &str,
    entries: Vec<Entry>,
    port: u16,
    debounce_ms: u64,
    start_in_ai_mode: bool,
) -> Result<Option<UiResult>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Shared state
    let mut input = String::new();
    let mut cursor_pos: usize = 0;
    let mut mode = if start_in_ai_mode { UiMode::Ai } else { UiMode::Search };

    // Search mode state
    let mut app = App::new(entries.clone());

    // AI mode state
    let mut ai_suggested_cmd = String::new();
    let mut ai_cursor_pos: usize = 0;
    let mut last_input_time = std::time::Instant::now();
    let mut pending_query = false;
    let mut ai_status = "Type your query (Tab=search mode)".to_string();
    let mut ai_loading = false;
    let mut edit_mode = false;

    let result;

    loop {
        // Check for AI debounce timeout BEFORE drawing
        if mode == UiMode::Ai && pending_query && !ai_loading {
            if last_input_time.elapsed().as_millis() >= debounce_ms as u128 {
                ai_loading = true;
                ai_status = "Querying AI...".to_string();
            }
        }

        terminal.draw(|f| {
            match mode {
                UiMode::Search => {
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(3), Constraint::Min(1)])
                        .split(f.area());

                    // Input box
                    let input_widget = Paragraph::new(app.input.as_str())
                        .style(Style::default().fg(Color::Yellow))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(format!(" Search ({} matches) [Tab=AI mode] ", app.filtered.len())),
                        );
                    f.render_widget(input_widget, chunks[0]);
                    f.set_cursor_position((chunks[0].x + cursor_pos as u16 + 1, chunks[0].y + 1));

                    // Results list
                    let items: Vec<ListItem> = app
                        .filtered
                        .iter()
                        .map(|(_, entry)| {
                            let style = match entry.entry_type.as_str() {
                                "subcommand" => Style::default().fg(Color::Cyan),
                                _ => Style::default().fg(Color::White),
                            };
                            let text = entry.display_text();
                            let max_len = chunks[1].width.saturating_sub(4) as usize;
                            let display = if text.len() > max_len {
                                format!("{}...", &text[..max_len.saturating_sub(3)])
                            } else {
                                text
                            };
                            ListItem::new(Line::from(vec![Span::styled(display, style)]))
                        })
                        .collect();

                    let list = List::new(items)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Results (Enter=run, Ctrl+O=copy, Esc=cancel) "),
                        )
                        .highlight_style(
                            Style::default()
                                .bg(Color::DarkGray)
                                .add_modifier(Modifier::BOLD),
                        )
                        .highlight_symbol("> ");
                    f.render_stateful_widget(list, chunks[1], &mut app.list_state);
                }
                UiMode::Ai => {
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(3), // Query input
                            Constraint::Length(3), // Suggested command
                            Constraint::Min(1),    // Status
                        ])
                        .split(f.area());

                    // Query/Edit input
                    let (input_title, input_display, input_cursor, input_color) = if edit_mode {
                        (
                            " Edit Command (Enter=run, Esc=back) ",
                            &ai_suggested_cmd,
                            ai_cursor_pos,
                            Color::Green,
                        )
                    } else {
                        (
                            " AI Query [Tab=search mode] ",
                            &input,
                            cursor_pos,
                            Color::Yellow,
                        )
                    };
                    let input_widget = Paragraph::new(input_display.as_str())
                        .style(Style::default().fg(input_color))
                        .block(Block::default().borders(Borders::ALL).title(input_title));
                    f.render_widget(input_widget, chunks[0]);
                    f.set_cursor_position((chunks[0].x + input_cursor as u16 + 1, chunks[0].y + 1));

                    // Suggested command
                    let (cmd_style, cmd_display) = if ai_loading {
                        (Style::default().fg(Color::Yellow), "Loading...".to_string())
                    } else if ai_suggested_cmd.is_empty() {
                        (Style::default().fg(Color::DarkGray), "(waiting for query...)".to_string())
                    } else {
                        (Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD), ai_suggested_cmd.clone())
                    };
                    let cmd_widget = Paragraph::new(cmd_display)
                        .style(cmd_style)
                        .block(Block::default().borders(Borders::ALL).title(" Suggested Command "));
                    f.render_widget(cmd_widget, chunks[1]);

                    // Status
                    let status_widget = Paragraph::new(ai_status.as_str())
                        .style(Style::default().fg(Color::White))
                        .block(Block::default().borders(Borders::ALL).title(" Status "));
                    f.render_widget(status_widget, chunks[2]);
                }
            }
        })?;

        // Process AI query after drawing (so loading state shows)
        if mode == UiMode::Ai && ai_loading {
            ai_loading = false;
            pending_query = false;

            match query_lm_studio(&input, command, &entries, port) {
                Ok(cmd) => {
                    ai_suggested_cmd = cmd;
                    ai_cursor_pos = ai_suggested_cmd.len();
                    ai_status = "Ready. Enter=run, Ctrl+E=edit, Esc=cancel".to_string();
                }
                Err(e) => {
                    ai_status = format!("Error: {}", e);
                }
            }
        }

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match mode {
                    UiMode::Search => {
                        match key.code {
                            KeyCode::Esc => {
                                result = Some(UiResult::Cancelled);
                                break;
                            }
                            KeyCode::Enter => {
                                if let Some(entry) = app.selected().cloned() {
                                    result = Some(UiResult::Entry(entry));
                                    break;
                                }
                            }
                            KeyCode::Tab => {
                                mode = UiMode::Ai;
                                ai_status = "Type your query (Tab=search mode)".to_string();
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                result = Some(UiResult::Cancelled);
                                break;
                            }
                            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Some(entry) = app.selected() {
                                    let cmd_str = entry.display_text();
                                    if let Ok(mut child) = Command::new("pbcopy")
                                        .stdin(Stdio::piped())
                                        .spawn()
                                    {
                                        if let Some(mut stdin) = child.stdin.take() {
                                            use std::io::Write;
                                            let _ = stdin.write_all(cmd_str.as_bytes());
                                        }
                                        let _ = child.wait();
                                    }
                                }
                                result = Some(UiResult::Copied);
                                break;
                            }
                            KeyCode::Up => app.move_selection(-1),
                            KeyCode::Down => app.move_selection(1),
                            KeyCode::PageUp => app.move_selection(-10),
                            KeyCode::PageDown => app.move_selection(10),
                            KeyCode::Left => {
                                if cursor_pos > 0 {
                                    cursor_pos -= 1;
                                }
                            }
                            KeyCode::Right => {
                                if cursor_pos < app.input.len() {
                                    cursor_pos += 1;
                                }
                            }
                            KeyCode::Char(c) => {
                                app.input.insert(cursor_pos, c);
                                cursor_pos += 1;
                                app.update_filter();
                            }
                            KeyCode::Backspace => {
                                if cursor_pos > 0 {
                                    app.input.remove(cursor_pos - 1);
                                    cursor_pos -= 1;
                                    app.update_filter();
                                }
                            }
                            _ => {}
                        }
                    }
                    UiMode::Ai => {
                        if edit_mode {
                            match key.code {
                                KeyCode::Esc => {
                                    edit_mode = false;
                                    ai_status = "Ready. Enter=run, Ctrl+E=edit, Esc=cancel".to_string();
                                }
                                KeyCode::Enter => {
                                    if !ai_suggested_cmd.is_empty() {
                                        result = Some(UiResult::Command(ai_suggested_cmd.clone()));
                                        break;
                                    }
                                }
                                KeyCode::Left => {
                                    if ai_cursor_pos > 0 {
                                        ai_cursor_pos -= 1;
                                    }
                                }
                                KeyCode::Right => {
                                    if ai_cursor_pos < ai_suggested_cmd.len() {
                                        ai_cursor_pos += 1;
                                    }
                                }
                                KeyCode::Char(c) => {
                                    ai_suggested_cmd.insert(ai_cursor_pos, c);
                                    ai_cursor_pos += 1;
                                }
                                KeyCode::Backspace => {
                                    if ai_cursor_pos > 0 {
                                        ai_suggested_cmd.remove(ai_cursor_pos - 1);
                                        ai_cursor_pos -= 1;
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            match key.code {
                                KeyCode::Esc => {
                                    result = Some(UiResult::Cancelled);
                                    break;
                                }
                                KeyCode::Enter => {
                                    if !ai_suggested_cmd.is_empty() {
                                        result = Some(UiResult::Command(ai_suggested_cmd.clone()));
                                        break;
                                    }
                                }
                                KeyCode::Tab => {
                                    mode = UiMode::Search;
                                }
                                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    if !ai_suggested_cmd.is_empty() {
                                        edit_mode = true;
                                        ai_cursor_pos = ai_suggested_cmd.len();
                                        ai_status = "Edit mode. Modify and press Enter.".to_string();
                                    }
                                }
                                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    result = Some(UiResult::Cancelled);
                                    break;
                                }
                                KeyCode::Left => {
                                    if cursor_pos > 0 {
                                        cursor_pos -= 1;
                                    }
                                }
                                KeyCode::Right => {
                                    if cursor_pos < input.len() {
                                        cursor_pos += 1;
                                    }
                                }
                                KeyCode::Char(c) => {
                                    input.insert(cursor_pos, c);
                                    cursor_pos += 1;
                                    last_input_time = std::time::Instant::now();
                                    pending_query = true;
                                    ai_status = "Typing... (will query after pause)".to_string();
                                }
                                KeyCode::Backspace => {
                                    if cursor_pos > 0 {
                                        input.remove(cursor_pos - 1);
                                        cursor_pos -= 1;
                                        if !input.is_empty() {
                                            last_input_time = std::time::Instant::now();
                                            pending_query = true;
                                            ai_status = "Typing... (will query after pause)".to_string();
                                        } else {
                                            pending_query = false;
                                            ai_suggested_cmd.clear();
                                            ai_status = "Type your query (Tab=search mode)".to_string();
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(result)
}

/// Try to get command info via --help-full (instant, no scanning needed).
fn try_help_full(command: &str) -> Option<CommandInfo> {
    let output = Command::new(command)
        .arg("--help-full")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).ok()
}

/// Get path to the help-full support cache file.
fn get_help_full_cache_path() -> Result<PathBuf> {
    Ok(get_cache_dir()?.join("help-full-commands.txt"))
}

/// Check if command is known to support --help-full (from cache).
fn supports_help_full(command: &str) -> bool {
    let base = command.rsplit('/').next().unwrap_or(command);

    let cache_path = match get_help_full_cache_path() {
        Ok(p) => p,
        Err(_) => return false,
    };

    if !cache_path.exists() {
        return false;
    }

    fs::read_to_string(&cache_path)
        .map(|content| content.lines().any(|line| line == base))
        .unwrap_or(false)
}

/// Mark a command as supporting --help-full.
fn mark_supports_help_full(command: &str) {
    let base = command.rsplit('/').next().unwrap_or(command);

    let cache_path = match get_help_full_cache_path() {
        Ok(p) => p,
        Err(_) => return,
    };

    // Read existing, add if not present, write back
    let mut commands: Vec<String> = cache_path
        .exists()
        .then(|| fs::read_to_string(&cache_path).ok())
        .flatten()
        .map(|c| c.lines().map(|s| s.to_string()).collect())
        .unwrap_or_default();

    if !commands.iter().any(|c| c == base) {
        commands.push(base.to_string());
        let _ = fs::write(&cache_path, commands.join("\n"));
    }
}

fn load_or_scan(command: &str, refresh: bool) -> Result<CommandInfo> {
    // Check if command is known to support --help-full
    if supports_help_full(command) {
        if let Some(info) = try_help_full(command) {
            return Ok(info);
        }
    }

    let cache_path = get_cache_path(command)?;

    // Check cache first
    if !refresh && cache_path.exists() {
        let data = fs::read_to_string(&cache_path)?;
        let cached: CommandInfo = serde_json::from_str(&data)?;

        let current_version = get_version(command)?;
        if cached.version == current_version {
            eprintln!("Using cached data for {} ({})", command, current_version);
            return Ok(cached);
        }
        eprintln!(
            "Version changed ({} -> {}), rescanning...",
            cached.version, current_version
        );
    }

    // Before scanning, try --help-full once (discover new commands that support it)
    if let Some(info) = try_help_full(command) {
        mark_supports_help_full(command);
        let data = serde_json::to_string_pretty(&info)?;
        fs::write(&cache_path, data)?;
        return Ok(info);
    }

    // Fall back to scanning
    eprintln!("Scanning {}...", command);
    let current_version = get_version(command)?;
    let entries = scan_command(command, 3)?;

    let info = CommandInfo {
        version: current_version,
        entries,
    };

    let data = serde_json::to_string_pretty(&info)?;
    fs::write(&cache_path, data)?;

    Ok(info)
}

struct App {
    input: String,
    entries: Vec<Entry>,
    filtered: Vec<(usize, Entry)>,
    list_state: ListState,
    matcher: Matcher,
}

impl App {
    fn new(entries: Vec<Entry>) -> Self {
        let filtered: Vec<(usize, Entry)> = entries.iter().cloned().enumerate().collect();
        let mut list_state = ListState::default();
        if !filtered.is_empty() {
            list_state.select(Some(0));
        }

        App {
            input: String::new(),
            entries,
            filtered,
            list_state,
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
        }
    }

    fn update_filter(&mut self) {
        if self.input.is_empty() {
            self.filtered = self.entries.iter().cloned().enumerate().collect();
        } else {
            let pattern = Pattern::parse(&self.input, CaseMatching::Ignore, Normalization::Smart);
            let mut scored: Vec<(i64, usize, Entry)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(idx, entry)| {
                    let haystack = entry.search_text();
                    let mut buf = Vec::new();
                    pattern
                        .score(
                            nucleo_matcher::Utf32Str::new(&haystack, &mut buf),
                            &mut self.matcher,
                        )
                        .map(|score| (score as i64, idx, entry.clone()))
                })
                .collect();

            scored.sort_by(|a, b| b.0.cmp(&a.0));
            self.filtered = scored.into_iter().map(|(_, idx, e)| (idx, e)).collect();
        }

        // Reset selection
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    fn selected(&self) -> Option<&Entry> {
        self.list_state
            .selected()
            .and_then(|i| self.filtered.get(i).map(|(_, e)| e))
    }

    fn move_selection(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }

        let current = self.list_state.selected().unwrap_or(0) as i32;
        let new = (current + delta).clamp(0, self.filtered.len() as i32 - 1) as usize;
        self.list_state.select(Some(new));
    }
}


fn build_command_string(entry: &Entry) -> String {
    match entry.entry_type.as_str() {
        "subcommand" => entry.command.clone(),
        "flag" => {
            let flag = entry
                .long
                .as_ref()
                .or(entry.short.as_ref())
                .cloned()
                .unwrap_or_default();
            format!("{} {}", entry.command, flag)
        }
        _ => entry.command.clone(),
    }
}

/// Resolve command to an executable path.
/// Falls back to ~/bin/<cmd> if `which` fails (e.g., for shell functions).
fn resolve_command(command: &str) -> Result<String> {
    // If it's already a path, use it directly
    if command.contains('/') {
        if PathBuf::from(command).exists() {
            return Ok(command.to_string());
        }
        anyhow::bail!("Command not found: {}", command);
    }

    // Try `which` first
    let which = Command::new("which")
        .arg(command)
        .output()
        .context("Failed to run which")?;

    if which.status.success() {
        let path = String::from_utf8_lossy(&which.stdout).trim().to_string();
        // Verify it's an actual file (not a function/alias output)
        if PathBuf::from(&path).is_file() {
            return Ok(command.to_string());
        }
    }

    // Fall back to ~/bin/<cmd>
    if let Some(home) = dirs::home_dir() {
        let bin_path = home.join("bin").join(command);
        if bin_path.is_file() {
            return Ok(bin_path.to_string_lossy().to_string());
        }
    }

    anyhow::bail!("Command not found: {} (not in PATH or ~/bin/)", command)
}

fn run_search(command: &str, refresh: bool, print_only: bool, list: bool) -> Result<()> {
    let resolved = resolve_command(command)?;

    let info = load_or_scan(&resolved, refresh)?;

    if info.entries.is_empty() {
        eprintln!("No commands or flags found for {}", command);
        return Ok(());
    }

    // List mode - just print all entries
    if list {
        for entry in &info.entries {
            println!("{}", entry.display_text());
        }
        return Ok(());
    }

    // Default LM Studio port and debounce
    let port = 1234;
    let debounce_ms = 2000;

    let result = run_unified_ui(&resolved, info.entries, port, debounce_ms, false)?;

    match result {
        Some(UiResult::Entry(entry)) => {
            let cmd_str = build_command_string(&entry);
            println!("{}", cmd_str);

            if !print_only {
                let parts: Vec<&str> = cmd_str.split_whitespace().collect();
                if !parts.is_empty() {
                    let status = Command::new(parts[0])
                        .args(&parts[1..])
                        .stdin(Stdio::inherit())
                        .stdout(Stdio::inherit())
                        .stderr(Stdio::inherit())
                        .status()?;

                    std::process::exit(status.code().unwrap_or(1));
                }
            }
        }
        Some(UiResult::Command(cmd_str)) => {
            println!("{}", cmd_str);

            if !print_only {
                let parts: Vec<&str> = cmd_str.split_whitespace().collect();
                if !parts.is_empty() {
                    let status = Command::new(parts[0])
                        .args(&parts[1..])
                        .stdin(Stdio::inherit())
                        .stdout(Stdio::inherit())
                        .stderr(Stdio::inherit())
                        .status()?;

                    std::process::exit(status.code().unwrap_or(1));
                }
            }
        }
        Some(UiResult::Copied) | Some(UiResult::Cancelled) | None => {}
    }

    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Handle subcommands first
    if let Some(cmd) = args.command {
        match cmd {
            Commands::Copy {
                command,
                path,
                depth,
            } => {
                let resolved = resolve_command(&command)?;

                eprintln!("Collecting deep help for '{}'...", resolved);
                let help_output = collect_deep_help(&resolved, depth)?;

                if let Some(path) = path {
                    fs::write(&path, &help_output)
                        .with_context(|| format!("Failed to write to {}", path.display()))?;
                    eprintln!("Wrote {} bytes to {}", help_output.len(), path.display());
                } else {
                    // Copy to clipboard using pbcopy (macOS)
                    let mut child = Command::new("pbcopy")
                        .stdin(Stdio::piped())
                        .spawn()
                        .context("Failed to run pbcopy")?;

                    if let Some(mut stdin) = child.stdin.take() {
                        use std::io::Write;
                        stdin.write_all(help_output.as_bytes())?;
                    }

                    child.wait()?;
                    eprintln!("Copied {} bytes to clipboard", help_output.len());
                }
            }
            Commands::Ai {
                command,
                port,
                debounce,
            } => {
                let resolved = resolve_command(&command)?;
                let info = load_or_scan(&resolved, false)?;

                if info.entries.is_empty() {
                    anyhow::bail!("No commands found for {}", command);
                }

                let result = run_unified_ui(&resolved, info.entries, port, debounce, true)?;

                match result {
                    Some(UiResult::Entry(entry)) => {
                        let cmd_str = build_command_string(&entry);
                        println!("{}", cmd_str);

                        let parts: Vec<&str> = cmd_str.split_whitespace().collect();
                        if !parts.is_empty() {
                            let status = Command::new(parts[0])
                                .args(&parts[1..])
                                .stdin(Stdio::inherit())
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit())
                                .status()?;

                            std::process::exit(status.code().unwrap_or(1));
                        }
                    }
                    Some(UiResult::Command(cmd_str)) => {
                        println!("{}", cmd_str);

                        let parts: Vec<&str> = cmd_str.split_whitespace().collect();
                        if !parts.is_empty() {
                            let status = Command::new(parts[0])
                                .args(&parts[1..])
                                .stdin(Stdio::inherit())
                                .stdout(Stdio::inherit())
                                .stderr(Stdio::inherit())
                                .status()?;

                            std::process::exit(status.code().unwrap_or(1));
                        }
                    }
                    Some(UiResult::Copied) | Some(UiResult::Cancelled) | None => {}
                }
            }
        }
        return Ok(());
    }

    // Default: search mode
    if let Some(cli) = args.cli {
        run_search(&cli, args.refresh, args.print_only, args.list)?;
    } else {
        anyhow::bail!("Usage: cmd <CLI> or cmd copy <CLI> [PATH]");
    }

    Ok(())
}
