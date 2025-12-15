//! Tree visualization and interactive navigation

use anyhow::Result;
use colored::Colorize;
use std::io::{self, Write};

use crate::element::{read_screen_state, Element, ScreenState};

/// Display the accessibility tree
pub fn show_tree(max_depth: usize, show_all: bool) -> Result<()> {
    let state = read_screen_state(max_depth)?;

    println!("{} │ {} elements", state.focused_app.cyan(), state.elements.len());
    println!("{}", "═".repeat(60).dimmed());

    // Group elements by depth for tree view
    let mut last_depth = 0;
    for elem in &state.elements {
        if !show_all && elem.label.is_empty() {
            continue;
        }

        let indent = "  ".repeat(elem.depth);
        let connector = if elem.depth > last_depth {
            "├─"
        } else if elem.depth == last_depth {
            "├─"
        } else {
            "└─"
        };

        let role_str = format_role(&elem.role);
        let label_str = if elem.label.is_empty() {
            "(no label)".dimmed().to_string()
        } else {
            format!("\"{}\"", elem.label).white().bold().to_string()
        };

        let status = if elem.focused {
            " ◆".green().to_string()
        } else if !elem.enabled {
            " ○".dimmed().to_string()
        } else {
            "".to_string()
        };

        println!(
            "{}{} {} {} {}{}",
            indent,
            connector.dimmed(),
            format!("[{}]", elem.id).dimmed(),
            role_str,
            label_str,
            status
        );

        last_depth = elem.depth;
    }

    Ok(())
}

fn format_role(role: &str) -> String {
    match role {
        "button" => role.blue().to_string(),
        "link" => role.cyan().to_string(),
        "textfield" | "textarea" => role.green().to_string(),
        "checkbox" | "radio" => role.yellow().to_string(),
        "menuitem" => role.magenta().to_string(),
        "window" | "app" => role.red().bold().to_string(),
        "group" | "scroll" => role.dimmed().to_string(),
        _ => role.white().to_string(),
    }
}

/// Interactive mode - navigate with keyboard
pub fn interactive_mode() -> Result<()> {
    println!("{}", "Interactive Accessibility Navigator".cyan().bold());
    println!("{}", "═".repeat(50).dimmed());
    println!("Commands:");
    println!("  {}     - refresh and list elements", "r".green());
    println!("  {}     - show tree view", "t".green());
    println!("  {} N   - click element by ID", "c".green());
    println!("  {} N   - focus element by ID", "f".green());
    println!("  {} N   - show details for element N", "i".green());
    println!("  {} X Y - element at coordinates", "at".green());
    println!("  {}     - show focused element", "?".green());
    println!("  {}     - quit", "q".green());
    println!("{}", "─".repeat(50).dimmed());

    let mut current_state: Option<ScreenState> = None;
    let mut selected_id: Option<usize> = None;

    loop {
        // Show prompt
        let prompt = match selected_id {
            Some(id) => format!("[{}] > ", id),
            None => "> ".to_string(),
        };
        print!("{}", prompt.cyan());
        io::stdout().flush()?;

        // Read input
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
                println!("{}", "Goodbye!".green());
                break;
            }

            "r" | "refresh" | "list" => {
                current_state = Some(read_screen_state(10)?);
                let state = current_state.as_ref().unwrap();
                println!("\n{} │ {} elements", state.focused_app.cyan(), state.elements.len());
                println!("{}", "─".repeat(60).dimmed());
                for elem in &state.elements {
                    let marker = if Some(elem.id) == selected_id {
                        "→ ".green().to_string()
                    } else {
                        "  ".to_string()
                    };
                    println!("{}{}", marker, elem.display());
                }
                println!();
            }

            "t" | "tree" => {
                current_state = Some(read_screen_state(10)?);
                println!();
                show_tree_inline(current_state.as_ref().unwrap())?;
                println!();
            }

            "c" | "click" => {
                if parts.len() < 2 {
                    if let Some(id) = selected_id {
                        crate::actions::click_element(Some(id), None, None)?;
                        println!("{}", format!("Clicked element {}", id).green());
                    } else {
                        println!("{}", "Usage: c <id> or select an element first".yellow());
                    }
                } else if let Ok(id) = parts[1].parse::<usize>() {
                    crate::actions::click_element(Some(id), None, None)?;
                    println!("{}", format!("Clicked element {}", id).green());
                } else {
                    println!("{}", "Invalid element ID".red());
                }
            }

            "f" | "focus" => {
                if parts.len() < 2 {
                    println!("{}", "Usage: f <id>".yellow());
                } else if let Ok(id) = parts[1].parse::<usize>() {
                    crate::actions::perform_action("focus", id, None)?;
                    println!("{}", format!("Focused element {}", id).green());
                } else {
                    println!("{}", "Invalid element ID".red());
                }
            }

            "s" | "select" => {
                if parts.len() < 2 {
                    println!("{}", "Usage: s <id>".yellow());
                } else if let Ok(id) = parts[1].parse::<usize>() {
                    selected_id = Some(id);
                    println!("{}", format!("Selected element {}", id).green());
                } else {
                    println!("{}", "Invalid element ID".red());
                }
            }

            "i" | "info" => {
                let target_id = if parts.len() >= 2 {
                    parts[1].parse::<usize>().ok()
                } else {
                    selected_id
                };

                if let Some(id) = target_id {
                    if current_state.is_none() {
                        current_state = Some(read_screen_state(10)?);
                    }
                    let state = current_state.as_ref().unwrap();
                    if let Some(elem) = state.find_by_id(id) {
                        println!();
                        print_element_details(elem);
                        println!();
                    } else {
                        println!("{}", format!("Element {} not found", id).red());
                    }
                } else {
                    println!("{}", "Usage: i <id> or select an element first".yellow());
                }
            }

            "at" => {
                if parts.len() < 3 {
                    println!("{}", "Usage: at <x> <y>".yellow());
                } else if let (Ok(x), Ok(y)) = (parts[1].parse::<i32>(), parts[2].parse::<i32>()) {
                    if current_state.is_none() {
                        current_state = Some(read_screen_state(10)?);
                    }
                    let state = current_state.as_ref().unwrap();
                    if let Some(elem) = state.element_at(x, y) {
                        println!("{}", format!("Element at ({}, {}):", x, y).green());
                        println!("  {}", elem.display());
                        selected_id = Some(elem.id);
                    } else {
                        println!("{}", format!("No element at ({}, {})", x, y).yellow());
                    }
                } else {
                    println!("{}", "Invalid coordinates".red());
                }
            }

            "?" | "focused" => {
                if current_state.is_none() {
                    current_state = Some(read_screen_state(10)?);
                }
                let state = current_state.as_ref().unwrap();
                let focused = state.elements.iter().find(|e| e.focused);
                if let Some(elem) = focused {
                    println!("{}", "Focused element:".green());
                    println!("  {}", elem.display());
                    selected_id = Some(elem.id);
                } else {
                    println!("{}", "No focused element".yellow());
                }
            }

            "type" => {
                if parts.len() < 2 {
                    println!("{}", "Usage: type <text>".yellow());
                } else {
                    let text = parts[1..].join(" ");
                    crate::actions::type_text(&text, selected_id)?;
                    println!("{}", format!("Typed: {}", text).green());
                }
            }

            _ => {
                // Try parsing as element ID for quick selection
                if let Ok(id) = cmd.parse::<usize>() {
                    selected_id = Some(id);
                    if current_state.is_none() {
                        current_state = Some(read_screen_state(10)?);
                    }
                    let state = current_state.as_ref().unwrap();
                    if let Some(elem) = state.find_by_id(id) {
                        println!("{}", format!("Selected: {}", elem.display()).green());
                    }
                } else {
                    println!("{}", format!("Unknown command: {}", cmd).red());
                }
            }
        }
    }

    Ok(())
}

fn show_tree_inline(state: &ScreenState) -> Result<()> {
    println!("{}", state.focused_app.cyan().bold());

    let mut last_depth = 0;
    for elem in &state.elements {
        let indent = "  ".repeat(elem.depth);
        let connector = if elem.depth > last_depth { "├─" } else { "├─" };

        println!(
            "{}{} [{}] {} \"{}\"",
            indent,
            connector.dimmed(),
            elem.id,
            format_role(&elem.role),
            elem.label
        );

        last_depth = elem.depth;
    }

    Ok(())
}

fn print_element_details(elem: &Element) {
    println!("{}", "Element Details".cyan().bold());
    println!("{}", "─".repeat(40).dimmed());
    println!("  ID:       {}", elem.id);
    println!("  Role:     {}", format_role(&elem.role));
    println!("  Label:    \"{}\"", elem.label.white().bold());
    println!("  Position: ({}, {})", elem.bbox.x, elem.bbox.y);
    println!("  Size:     {}x{}", elem.bbox.width, elem.bbox.height);
    println!("  Center:   {:?}", elem.bbox.center());
    println!("  Enabled:  {}", if elem.enabled { "yes".green() } else { "no".red() });
    println!("  Focused:  {}", if elem.focused { "yes".green() } else { "no".dimmed() });

    if let Some(ref value) = elem.value {
        println!("  Value:    \"{}\"", value);
    }
    if let Some(ref desc) = elem.description {
        println!("  Help:     {}", desc.dimmed());
    }
    if !elem.actions.is_empty() {
        println!("  Actions:  {}", elem.actions.join(", "));
    }
}
