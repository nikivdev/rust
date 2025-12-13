use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use regex::Regex;

const CONFIG_PATH: &str = "/Users/nikiv/config/i/karabiner/karabiner.edn";

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { layer, key, action } => add_rule(&layer, &key, &action),
        Commands::Comment { layer, key } => comment_rule(&layer, &key),
    }
}

#[derive(Parser)]
#[command(name = "karabiner", version, about = "Karabiner EDN config CLI", propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a key binding to a layer.
    ///
    /// Examples:
    ///   karabiner add v e "zed: LMCache"
    ///   karabiner add semicolon spacebar "open: Reflect"
    Add {
        /// Layer name (e.g., "v", "s", "semicolon", "quote").
        layer: String,
        /// Key to bind (e.g., "e", "spacebar", "semicolon").
        key: String,
        /// Keyboard Maestro macro name (e.g., "zed: LMCache").
        action: String,
    },

    /// Comment out a key binding (moves it after "; --" separator).
    ///
    /// Examples:
    ///   karabiner comment v e
    Comment {
        /// Layer name.
        layer: String,
        /// Key to comment out.
        key: String,
    },
}

fn get_config_path() -> PathBuf {
    PathBuf::from(CONFIG_PATH)
}

fn read_config() -> Result<String> {
    fs::read_to_string(get_config_path()).context("failed to read karabiner.edn")
}

fn write_config(content: &str) -> Result<()> {
    fs::write(get_config_path(), content).context("failed to write karabiner.edn")
}

/// Find a layer section in the config. Layer names map as:
/// - "v" -> finds {:des "vkey ..."
/// - "semicolon" -> finds {:des "colonkey ..." (semicolon key)
/// - etc.
fn find_layer_section(content: &str, layer: &str) -> Option<(usize, usize)> {
    // Map layer name to des pattern
    let des_pattern = match layer {
        "semicolon" => "colonkey",
        "quote" => "quotekey",
        other => {
            // For single letter layers like "v", "s", etc., look for "vkey", "skey"
            return find_layer_by_key(content, other);
        }
    };

    find_layer_by_des(content, des_pattern)
}

fn find_layer_by_key(content: &str, key: &str) -> Option<(usize, usize)> {
    // Look for {:des "<key>key pattern
    let pattern = format!(r#"\{{:des\s+"{}key"#, regex::escape(key));
    let re = Regex::new(&pattern).ok()?;

    let mat = re.find(content)?;
    let start = mat.start();

    // Find the end of this layer section (next {:des or end of :main array)
    find_section_end(content, start)
}

fn find_layer_by_des(content: &str, des_contains: &str) -> Option<(usize, usize)> {
    let pattern = format!(r#"\{{:des\s+"[^"]*{}[^"]*""#, regex::escape(des_contains));
    let re = Regex::new(&pattern).ok()?;

    let mat = re.find(content)?;
    let start = mat.start();

    find_section_end(content, start)
}

fn find_section_end(content: &str, start: usize) -> Option<(usize, usize)> {
    // Find matching closing brace for the section
    let section = &content[start..];
    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, c) in section.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match c {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' | '[' if !in_string => depth += 1,
            '}' | ']' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some((start, start + i + 1));
                }
            }
            _ => {}
        }
    }

    None
}

fn add_rule(layer: &str, key: &str, action: &str) -> Result<()> {
    let content = read_config()?;

    let (start, end) = find_layer_section(&content, layer)
        .ok_or_else(|| anyhow::anyhow!("layer '{}' not found", layer))?;

    let section = &content[start..end];

    // Check if key already exists as an active (non-commented) rule
    let key_pattern = format!(r#"(?m)^\s+\[:{}[\s\[]"#, regex::escape(key));
    let key_re = Regex::new(&key_pattern)?;

    if key_re.is_match(section) {
        bail!("key '{}' already exists in layer '{}'. Use 'comment' first.", key, layer);
    }

    // Find where to insert the new rule (after :rules [:X-mode line)
    let rules_pattern = r#":rules\s+\[:[a-z\-]+mode"#;
    let rules_re = Regex::new(rules_pattern)?;

    let rules_match = rules_re.find(section)
        .ok_or_else(|| anyhow::anyhow!("could not find :rules in layer '{}'", layer))?;

    let insert_pos = start + rules_match.end();
    let new_rule = format!("\n                 [:{} [:km \"{}\"]]", key, action);

    let mut new_content = String::with_capacity(content.len() + new_rule.len());
    new_content.push_str(&content[..insert_pos]);
    new_content.push_str(&new_rule);
    new_content.push_str(&content[insert_pos..]);

    write_config(&new_content)?;
    println!("added [:{} [:km \"{}\"]] to {}", key, action, layer);

    Ok(())
}

fn comment_rule(layer: &str, key: &str) -> Result<()> {
    let content = read_config()?;

    let (start, end) = find_layer_section(&content, layer)
        .ok_or_else(|| anyhow::anyhow!("layer '{}' not found", layer))?;

    let section = &content[start..end];

    // Find the active rule line (not commented)
    let pattern = format!(r#"(?m)^(\s+)\[:{}([^\n]*)\n"#, regex::escape(key));
    let re = Regex::new(&pattern)?;

    let cap = re.captures(section)
        .ok_or_else(|| anyhow::anyhow!("key '{}' not found in layer '{}'", key, layer))?;

    let full_match = cap.get(0).unwrap();
    let rule_content = cap.get(2).unwrap().as_str();

    let remove_start = start + full_match.start();
    let remove_end = start + full_match.end();

    // Look for "; --" separator in the section
    let separator_pattern = r#"(?m)^\s+; --\s*\n"#;
    let sep_re = Regex::new(separator_pattern)?;

    let commented_rule = format!(";[:{}{}", key, rule_content);

    let new_content = if let Some(sep_match) = sep_re.find(section) {
        // Found "; --", insert commented rule right after it
        let insert_pos = start + sep_match.end();

        let mut result = String::with_capacity(content.len() + commented_rule.len() + 20);
        result.push_str(&content[..remove_start]);
        result.push_str(&content[remove_end..insert_pos]);
        result.push_str("                 ");
        result.push_str(&commented_rule);
        result.push_str("\n");
        result.push_str(&content[insert_pos..]);
        result
    } else {
        // No "; --" separator, find last active rule and add separator + commented rule after it
        // Find the last active rule line before any commented rules
        let last_active_pattern = r#"(?m)^(\s+\[[^\n]+\n)(?=\s*;|\s*\]})"#;
        let last_re = Regex::new(last_active_pattern)?;

        if let Some(last_match) = last_re.find(section) {
            let insert_pos = start + last_match.end();

            let mut result = String::with_capacity(content.len() + commented_rule.len() + 40);
            result.push_str(&content[..remove_start]);
            result.push_str(&content[remove_end..insert_pos]);
            result.push_str("                 ; --\n");
            result.push_str("                 ");
            result.push_str(&commented_rule);
            result.push_str("\n");
            result.push_str(&content[insert_pos..]);
            result
        } else {
            bail!("could not find insertion point in layer '{}'", layer);
        }
    };

    write_config(&new_content)?;
    println!("commented [:{} ...] in {}", key, layer);

    Ok(())
}
