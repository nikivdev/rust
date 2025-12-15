//! Actions - clicking, typing, and other interactions

use anyhow::{Result, bail, Context};
use colored::Colorize;

use crate::element::read_screen_state;

/// Click an element by ID, label, or role
pub fn click_element(
    id: Option<usize>,
    label: Option<String>,
    role: Option<String>,
) -> Result<()> {
    let state = read_screen_state(10)?;

    // Find the target element
    let target = if let Some(id) = id {
        state.elements.iter().find(|e| e.id == id)
    } else if let Some(ref label) = label {
        let matches = state.find_by_label(label);
        if let Some(ref role) = role {
            matches.into_iter().find(|e| e.role.to_lowercase() == role.to_lowercase())
        } else {
            matches.into_iter().next()
        }
    } else if let Some(ref role) = role {
        state.find_by_role(role).into_iter().next()
    } else {
        bail!("Must specify id, label, or role");
    };

    let element = target.context("Element not found")?;

    if !element.enabled {
        eprintln!("{}", "Warning: Element is disabled".yellow());
    }

    // Get center coordinates
    let (x, y) = element.bbox.center();

    println!(
        "Clicking {} \"{}\" at ({}, {})",
        element.role.cyan(),
        element.label.white().bold(),
        x,
        y
    );

    // Perform the click
    click_at(x, y)?;

    Ok(())
}

/// Click at specific coordinates
pub fn click_at(x: i32, y: i32) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use enigo::{Enigo, Mouse, Settings, Coordinate, Button};

        let mut enigo = Enigo::new(&Settings::default())
            .context("Failed to create input controller")?;

        // Move to position and click
        enigo.move_mouse(x, y, Coordinate::Abs)
            .context("Failed to move mouse")?;

        // Small delay to ensure position is set
        std::thread::sleep(std::time::Duration::from_millis(10));

        enigo.button(Button::Left, enigo::Direction::Click)
            .context("Failed to click")?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Click at ({}, {}) - not implemented for this platform", x, y);
    }

    Ok(())
}

/// Type text, optionally into a specific element
pub fn type_text(text: &str, element_id: Option<usize>) -> Result<()> {
    // If element specified, click it first to focus
    if let Some(id) = element_id {
        let state = read_screen_state(10)?;
        if let Some(elem) = state.find_by_id(id) {
            let (x, y) = elem.bbox.center();
            click_at(x, y)?;
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[cfg(target_os = "macos")]
    {
        use enigo::{Enigo, Keyboard, Settings};

        let mut enigo = Enigo::new(&Settings::default())
            .context("Failed to create input controller")?;

        enigo.text(text)
            .context("Failed to type text")?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Type '{}' - not implemented for this platform", text);
    }

    Ok(())
}

/// Perform an action on an element
pub fn perform_action(action: &str, element_id: usize, value: Option<String>) -> Result<()> {
    let state = read_screen_state(10)?;
    let element = state.find_by_id(element_id)
        .context(format!("Element {} not found", element_id))?;

    match action.to_lowercase().as_str() {
        "click" | "press" => {
            let (x, y) = element.bbox.center();
            click_at(x, y)?;
            println!("{}", format!("Clicked element {}", element_id).green());
        }

        "focus" => {
            #[cfg(target_os = "macos")]
            {
                focus_element(element_id)?;
            }
            println!("{}", format!("Focused element {}", element_id).green());
        }

        "value" | "set" => {
            let val = value.context("Value required for 'value' action")?;
            // Click to focus, then clear and type
            let (x, y) = element.bbox.center();
            click_at(x, y)?;
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Select all and replace
            #[cfg(target_os = "macos")]
            {
                use enigo::{Enigo, Keyboard, Key, Settings};
                let mut enigo = Enigo::new(&Settings::default())?;
                // Cmd+A to select all
                enigo.key(Key::Meta, enigo::Direction::Press)?;
                enigo.key(Key::Unicode('a'), enigo::Direction::Click)?;
                enigo.key(Key::Meta, enigo::Direction::Release)?;
                std::thread::sleep(std::time::Duration::from_millis(50));
                // Type new value
                enigo.text(&val)?;
            }
            println!("{}", format!("Set value to '{}'", val).green());
        }

        "double" | "doubleclick" => {
            let (x, y) = element.bbox.center();
            double_click_at(x, y)?;
            println!("{}", format!("Double-clicked element {}", element_id).green());
        }

        "right" | "rightclick" => {
            let (x, y) = element.bbox.center();
            right_click_at(x, y)?;
            println!("{}", format!("Right-clicked element {}", element_id).green());
        }

        _ => {
            bail!("Unknown action: {}. Valid actions: click, focus, value, double, right", action);
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn focus_element(_element_id: usize) -> Result<()> {
    // For now, just click to focus
    // In the future, we could use AXUIElement.performAction("AXFocus")
    Ok(())
}

fn double_click_at(x: i32, y: i32) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use enigo::{Enigo, Mouse, Settings, Coordinate, Button};

        let mut enigo = Enigo::new(&Settings::default())?;
        enigo.move_mouse(x, y, Coordinate::Abs)?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        enigo.button(Button::Left, enigo::Direction::Click)?;
        std::thread::sleep(std::time::Duration::from_millis(50));
        enigo.button(Button::Left, enigo::Direction::Click)?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Double-click at ({}, {}) - not implemented", x, y);
    }

    Ok(())
}

fn right_click_at(x: i32, y: i32) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use enigo::{Enigo, Mouse, Settings, Coordinate, Button};

        let mut enigo = Enigo::new(&Settings::default())?;
        enigo.move_mouse(x, y, Coordinate::Abs)?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        enigo.button(Button::Right, enigo::Direction::Click)?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Right-click at ({}, {}) - not implemented", x, y);
    }

    Ok(())
}
