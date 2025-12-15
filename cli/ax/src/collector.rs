//! Data collection for training - records user actions with screen state

use anyhow::{Result, Context};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{self, Write, BufWriter};
use std::time::Instant;

use crate::element::{read_screen_state, Element, ScreenState};

/// A training sample: screen state + command + target element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingSample {
    pub screen_state: ScreenState,
    pub command: String,
    pub target_element_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_type: Option<String>,
}

/// Configuration for data collection
struct CollectorConfig {
    output_path: String,
    app_filter: Option<String>,
    auto_command: bool,
    dry_run: bool,
}

/// Main data collection loop
pub fn collect_data(
    output: &str,
    app: Option<String>,
    auto: bool,
    dry_run: bool,
) -> Result<()> {
    let config = CollectorConfig {
        output_path: output.to_string(),
        app_filter: app,
        auto_command: auto,
        dry_run,
    };

    println!("{}", "╔════════════════════════════════════════════════════════════╗".cyan());
    println!("{}", "║           ACCESSIBILITY TRAINING DATA COLLECTOR            ║".cyan());
    println!("{}", "╚════════════════════════════════════════════════════════════╝".cyan());
    println!();
    println!("Output: {}", config.output_path.green());
    if let Some(ref app) = config.app_filter {
        println!("App filter: {}", app.yellow());
    }
    println!("Auto-command: {}", if config.auto_command { "yes".green() } else { "no".dimmed() });
    println!("Dry run: {}", if config.dry_run { "yes".yellow() } else { "no".dimmed() });
    println!();
    println!("{}", "─".repeat(60).dimmed());
    println!("Commands:");
    println!("  {}      - refresh screen state", "r".green());
    println!("  {} <id>  - record sample for element", "s".green());
    println!("  {} <id>  - click element (and record)", "c".green());
    println!("  {}      - show stats", "stats".green());
    println!("  {}      - quit", "q".green());
    println!("{}", "─".repeat(60).dimmed());
    println!();

    let mut sample_count = 0;
    let mut current_state: Option<ScreenState> = None;
    let start_time = Instant::now();

    loop {
        print!("{} ", format!("[{}]>", sample_count).cyan());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();

        match cmd.as_str() {
            "q" | "quit" | "exit" => {
                println!();
                println!("{}", "═".repeat(60).dimmed());
                println!(
                    "Collected {} samples in {:.1}s",
                    sample_count.to_string().green(),
                    start_time.elapsed().as_secs_f64()
                );
                if !config.dry_run {
                    println!("Saved to: {}", config.output_path.green());
                }
                break;
            }

            "r" | "refresh" => {
                current_state = Some(read_screen_state(10)?);
                let state = current_state.as_ref().unwrap();

                // Check app filter
                if let Some(ref filter) = config.app_filter {
                    if !state.focused_app.to_lowercase().contains(&filter.to_lowercase()) {
                        println!(
                            "{}",
                            format!("Skipping: {} doesn't match filter '{}'",
                                state.focused_app, filter).yellow()
                        );
                        continue;
                    }
                }

                println!("\n{} │ {} elements", state.focused_app.cyan(), state.elements.len());
                println!("{}", "─".repeat(60).dimmed());
                for elem in &state.elements {
                    println!("{}", elem.display());
                }
                println!();
            }

            "s" | "sample" => {
                if parts.len() < 2 {
                    println!("{}", "Usage: s <element_id>".yellow());
                    continue;
                }

                let id: usize = match parts[1].parse() {
                    Ok(id) => id,
                    Err(_) => {
                        println!("{}", "Invalid element ID".red());
                        continue;
                    }
                };

                // Refresh state if needed
                if current_state.is_none() {
                    current_state = Some(read_screen_state(10)?);
                }
                let state = current_state.as_ref().unwrap();

                // Find element
                let element = match state.find_by_id(id) {
                    Some(e) => e,
                    None => {
                        println!("{}", format!("Element {} not found", id).red());
                        continue;
                    }
                };

                // Get command
                let command = if config.auto_command {
                    generate_command(element)
                } else {
                    println!("Element: {} \"{}\"", element.role.cyan(), element.label.white().bold());
                    print!("Command: ");
                    io::stdout().flush()?;

                    let mut cmd = String::new();
                    io::stdin().read_line(&mut cmd)?;
                    let cmd = cmd.trim().to_string();

                    if cmd.is_empty() {
                        println!("{}", "Skipped (no command)".yellow());
                        continue;
                    }
                    cmd
                };

                // Create and save sample
                let sample = TrainingSample {
                    screen_state: state.clone(),
                    command: command.clone(),
                    target_element_id: id,
                    action_type: Some("click".to_string()),
                };

                if !config.dry_run {
                    save_sample(&config.output_path, &sample)?;
                }

                sample_count += 1;
                println!(
                    "{} Sample #{}: \"{}\" -> {} \"{}\"",
                    "✓".green(),
                    sample_count,
                    command.white(),
                    element.role,
                    element.label
                );
            }

            "c" | "click" => {
                if parts.len() < 2 {
                    println!("{}", "Usage: c <element_id>".yellow());
                    continue;
                }

                let id: usize = match parts[1].parse() {
                    Ok(id) => id,
                    Err(_) => {
                        println!("{}", "Invalid element ID".red());
                        continue;
                    }
                };

                // Refresh and get element
                current_state = Some(read_screen_state(10)?);
                let state = current_state.as_ref().unwrap();

                let element = match state.find_by_id(id) {
                    Some(e) => e.clone(),
                    None => {
                        println!("{}", format!("Element {} not found", id).red());
                        continue;
                    }
                };

                // Get command
                let command = if config.auto_command {
                    generate_command(&element)
                } else {
                    println!("Element: {} \"{}\"", element.role.cyan(), element.label.white().bold());
                    print!("Command (or Enter to skip recording): ");
                    io::stdout().flush()?;

                    let mut cmd = String::new();
                    io::stdin().read_line(&mut cmd)?;
                    cmd.trim().to_string()
                };

                // Click the element
                crate::actions::click_element(Some(id), None, None)?;

                // Save if command provided
                if !command.is_empty() {
                    let sample = TrainingSample {
                        screen_state: state.clone(),
                        command: command.clone(),
                        target_element_id: id,
                        action_type: Some("click".to_string()),
                    };

                    if !config.dry_run {
                        save_sample(&config.output_path, &sample)?;
                    }

                    sample_count += 1;
                    println!(
                        "{} Sample #{}: \"{}\" -> {} \"{}\"",
                        "✓".green(),
                        sample_count,
                        command.white(),
                        element.role,
                        element.label
                    );
                }

                // Clear state since screen may have changed
                current_state = None;
            }

            "stats" => {
                let elapsed = start_time.elapsed();
                println!();
                println!("{}", "Collection Statistics".cyan().bold());
                println!("{}", "─".repeat(30).dimmed());
                println!("Samples collected: {}", sample_count.to_string().green());
                println!("Time elapsed: {:.1}s", elapsed.as_secs_f64());
                if sample_count > 0 {
                    println!(
                        "Rate: {:.2} samples/min",
                        sample_count as f64 / (elapsed.as_secs_f64() / 60.0)
                    );
                }
                println!();
            }

            "help" | "?" => {
                println!("Commands:");
                println!("  r          - refresh screen state");
                println!("  s <id>     - record sample for element");
                println!("  c <id>     - click element (and record)");
                println!("  stats      - show statistics");
                println!("  q          - quit");
            }

            _ => {
                // Try to parse as element ID for quick sample
                if let Ok(id) = cmd.parse::<usize>() {
                    if current_state.is_none() {
                        current_state = Some(read_screen_state(10)?);
                    }
                    let state = current_state.as_ref().unwrap();

                    if let Some(element) = state.find_by_id(id) {
                        println!("Element {}: {} \"{}\"", id, element.role.cyan(), element.label);
                        println!("  Use 's {}' to record or 'c {}' to click", id, id);
                    } else {
                        println!("{}", format!("Element {} not found", id).red());
                    }
                } else {
                    println!("{}", format!("Unknown command: {}", cmd).red());
                }
            }
        }
    }

    Ok(())
}

/// Generate a natural language command from element properties
fn generate_command(element: &Element) -> String {
    let templates = [
        format!("click {}", element.label.to_lowercase()),
        format!("click the {} button", element.label.to_lowercase()),
        format!("press {}", element.label.to_lowercase()),
        format!("select {}", element.label.to_lowercase()),
    ];

    // Use deterministic selection based on label hash
    let idx = element.label.len() % templates.len();
    templates[idx].clone()
}

/// Save a sample to the output file
fn save_sample(path: &str, sample: &TrainingSample) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("Failed to open output file")?;

    let mut writer = BufWriter::new(file);
    let json = serde_json::to_string(sample)?;
    writeln!(writer, "{}", json)?;

    Ok(())
}
