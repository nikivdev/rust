//! ax - macOS Accessibility Tree Navigation CLI
//!
//! A tool for navigating and interacting with the macOS accessibility tree.
//! Designed for computer use training - collecting data to train models
//! that can navigate UI elements via natural language commands.
//!
//! Usage:
//!     # List all elements in focused app
//!     ax list
//!
//!     # Show element tree hierarchy
//!     ax tree
//!
//!     # Watch mode - stream elements
//!     ax watch --interval 500
//!
//!     # Click element by ID
//!     ax click 5
//!
//!     # Click element by label (fuzzy match)
//!     ax click --label "Submit"
//!
//!     # Type text into focused element
//!     ax type "hello world"
//!
//!     # Collect training data
//!     ax collect --output training.jsonl
//!
//!     # Interactive mode - navigate with keyboard
//!     ax interactive

mod element;
mod tree;
mod actions;
mod collector;

use clap::{Parser, Subcommand};
use anyhow::Result;
use colored::Colorize;

#[derive(Parser)]
#[command(name = "ax")]
#[command(author, version, about = "macOS accessibility tree navigation", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// List all accessible elements in the focused app
    List {
        /// Filter by role (button, link, textfield, etc.)
        #[arg(long, short)]
        role: Option<String>,

        /// Filter by label (case-insensitive substring match)
        #[arg(long, short)]
        label: Option<String>,

        /// Show only enabled elements
        #[arg(long)]
        enabled_only: bool,

        /// Maximum depth to traverse
        #[arg(long, short, default_value = "10")]
        depth: usize,
    },

    /// Show element tree hierarchy
    Tree {
        /// Maximum depth to show
        #[arg(long, short, default_value = "5")]
        depth: usize,

        /// Show all elements (not just interactive)
        #[arg(long)]
        all: bool,
    },

    /// Watch mode - continuously output elements
    Watch {
        /// Interval in milliseconds
        #[arg(long, short, default_value = "500")]
        interval: u64,

        /// Output to file instead of stdout
        #[arg(long, short)]
        output: Option<String>,
    },

    /// Click an element
    Click {
        /// Element ID to click
        #[arg(conflicts_with = "label")]
        id: Option<usize>,

        /// Click element by label (fuzzy match)
        #[arg(long, short)]
        label: Option<String>,

        /// Click element by role
        #[arg(long, short)]
        role: Option<String>,
    },

    /// Type text
    Type {
        /// Text to type
        text: String,

        /// Element ID to focus first (optional)
        #[arg(long)]
        element: Option<usize>,
    },

    /// Collect training data by recording user actions
    Collect {
        /// Output file for training samples
        #[arg(long, short, default_value = "training.jsonl")]
        output: String,

        /// Only record for specific app
        #[arg(long)]
        app: Option<String>,

        /// Auto-generate commands from element labels
        #[arg(long)]
        auto: bool,

        /// Dry run - don't save samples
        #[arg(long)]
        dry_run: bool,
    },

    /// Interactive mode - navigate with keyboard
    Interactive,

    /// Show info about focused element
    Focus,

    /// Get element at screen coordinates
    At {
        /// X coordinate
        x: i32,
        /// Y coordinate
        y: i32,
    },

    /// Perform action on element (for scripting)
    Do {
        /// Action type: click, press, focus, value
        action: String,
        /// Element ID
        id: usize,
        /// Value for 'value' action
        #[arg(long)]
        value: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Check accessibility permissions
    if !check_accessibility_permission() {
        eprintln!("{}", "Error: Accessibility permission not granted.".red());
        eprintln!("Please enable accessibility for this app in:");
        eprintln!("  System Preferences > Privacy & Security > Accessibility");
        std::process::exit(1);
    }

    match cli.command {
        Commands::List { role, label, enabled_only, depth } => {
            element::list_elements(role, label, enabled_only, depth, cli.json)?;
        }
        Commands::Tree { depth, all } => {
            tree::show_tree(depth, all)?;
        }
        Commands::Watch { interval, output } => {
            element::watch_elements(interval, output, cli.json)?;
        }
        Commands::Click { id, label, role } => {
            actions::click_element(id, label, role)?;
        }
        Commands::Type { text, element } => {
            actions::type_text(&text, element)?;
        }
        Commands::Collect { output, app, auto, dry_run } => {
            collector::collect_data(&output, app, auto, dry_run)?;
        }
        Commands::Interactive => {
            tree::interactive_mode()?;
        }
        Commands::Focus => {
            element::show_focused(cli.json)?;
        }
        Commands::At { x, y } => {
            element::element_at(x, y, cli.json)?;
        }
        Commands::Do { action, id, value } => {
            actions::perform_action(&action, id, value)?;
        }
    }

    Ok(())
}

/// Check if we have accessibility permission
fn check_accessibility_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        use accessibility::{AXUIElement, AXAttribute};
        use core_foundation::base::CFType;
        // Try to get focused element from system-wide - this requires permission
        let system = AXUIElement::system_wide();
        let focused_attr = AXAttribute::<CFType>::new(
            &core_foundation::string::CFString::from_static_string("AXFocusedUIElement")
        );
        // Try to read attribute - if we have permission it will work
        system.attribute(&focused_attr).is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
