use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::CreateOpen { name, app, path, goku } => create_open_macro(&name, &app, &path, goku.as_deref()),
        Commands::List => list_macros(),
        Commands::Run { name } => run_macro(&name),
        Commands::Inspect { name } => inspect_macro(&name),
    }
}

#[derive(Parser)]
#[command(name = "km", version, about = "Keyboard Maestro CLI", propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create an "open" macro that focuses or opens an app with a path.
    ///
    /// Examples:
    ///   km create-open "zed: focus" Zed "~/org/1f/focus"
    ///   km create-open "zed: focus" Zed "~/org/1f/focus" --goku v.o
    CreateOpen {
        /// Macro name (e.g., "zed: focus").
        name: String,
        /// App name (e.g., "Zed").
        app: String,
        /// Path to open (e.g., "~/org/1f/focus").
        path: String,
        /// Goku binding in "layer.key" format (e.g., "v.o" for v-mode + o key).
        #[arg(long)]
        goku: Option<String>,
    },
    /// List all macros.
    List,
    /// Run a macro by name.
    Run {
        /// Macro name to run.
        name: String,
    },
    /// Inspect a macro's actions as JSON.
    Inspect {
        /// Macro name to inspect.
        name: String,
    },
}

const KARABINER_CONFIG: &str = "/Users/nikiv/config/i/karabiner/karabiner.edn";

fn create_open_macro(name: &str, app: &str, path: &str, goku: Option<&str>) -> Result<()> {
    // Check if macro already exists
    if macro_exists(name)? {
        bail!("macro '{}' already exists in Keyboard Maestro", name);
    }

    // Parse and validate goku binding if provided
    let goku_binding = if let Some(binding) = goku {
        let parts: Vec<&str> = binding.split('.').collect();
        if parts.len() != 2 {
            bail!("goku binding must be in 'layer.key' format (e.g., 'v.o')");
        }
        let layer = parts[0];
        let key = parts[1];

        // Check if key already bound in layer
        if goku_key_exists(layer, key)? {
            bail!("key '{}' already bound in layer '{}'. Use 'karabiner comment {} {}' first.", key, layer, layer, key);
        }

        Some((layer.to_string(), key.to_string()))
    } else {
        None
    };

    // Get folder name from path for matching
    let folder_name = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path);

    let plist = generate_open_macro_plist(name, app, path, folder_name);

    // Import via Keyboard Maestro
    import_macro_plist(&plist)?;

    println!("created macro: {}", name);

    // Add goku binding if provided
    if let Some((layer, key)) = goku_binding {
        add_goku_rule(&layer, &key, name)?;
        println!("added goku binding: {}.{} -> {}", layer, key, name);
    }

    Ok(())
}

fn macro_exists(name: &str) -> Result<bool> {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"tell application "Keyboard Maestro"
    try
        set m to first macro whose name is "{}"
        return "EXISTS"
    on error
        return "NOT_FOUND"
    end try
end tell"#,
        escaped
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("failed to run osascript")?;

    let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(result == "EXISTS")
}

fn goku_key_exists(layer: &str, key: &str) -> Result<bool> {
    let content = std::fs::read_to_string(KARABINER_CONFIG)
        .context("failed to read karabiner.edn")?;

    // Find the layer section
    let layer_pattern = if layer == "semicolon" {
        "colonkey".to_string()
    } else if layer == "quote" {
        "quotekey".to_string()
    } else {
        format!("{}key", layer)
    };

    // Look for {:des "<layer>key pattern
    let des_pattern = format!(r#"\{{:des\s+"{}[^"]*""#, regex::escape(&layer_pattern));
    let des_re = regex::Regex::new(&des_pattern)?;

    let section_start = match des_re.find(&content) {
        Some(m) => m.start(),
        None => return Ok(false), // Layer not found
    };

    // Find section end
    let section = &content[section_start..];
    let mut depth = 0;
    let mut in_string = false;
    let mut section_end = section.len();

    for (i, c) in section.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '{' | '[' if !in_string => depth += 1,
            '}' | ']' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    section_end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    let section_content = &section[..section_end];

    // Check for active (non-commented) rule with this key
    let key_pattern = format!(r#"(?m)^\s+\[:{}[\s\[]"#, regex::escape(key));
    let key_re = regex::Regex::new(&key_pattern)?;

    Ok(key_re.is_match(section_content))
}

fn add_goku_rule(layer: &str, key: &str, action: &str) -> Result<()> {
    // Call karabiner CLI to add the rule
    let status = Command::new("karabiner")
        .args(["add", layer, key, action])
        .status()
        .context("failed to run karabiner CLI")?;

    if !status.success() {
        bail!("karabiner add failed");
    }

    Ok(())
}

fn get_bundle_identifier(app: &str) -> Option<String> {
    let output = Command::new("mdls")
        .args(["-name", "kMDItemCFBundleIdentifier", "-raw", &format!("/Applications/{}.app", app)])
        .output()
        .ok()?;

    if output.status.success() {
        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if id != "(null)" && !id.is_empty() {
            return Some(id);
        }
    }
    None
}

fn generate_open_macro_plist(name: &str, app: &str, path: &str, folder_name: &str) -> String {
    let name_escaped = escape_xml(name);
    let app_escaped = escape_xml(app);
    let path_escaped = escape_xml(path);
    let folder_escaped = escape_xml(folder_name);
    let bundle_id = get_bundle_identifier(app).unwrap_or_default();
    let bundle_id_escaped = escape_xml(&bundle_id);
    let macro_uid = uuid::Uuid::new_v4().to_string().to_uppercase();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
	<dict>
		<key>Macros</key>
		<array>
			<dict>
				<key>Activate</key>
				<string>Normal</string>
				<key>Name</key>
				<string>{name}</string>
				<key>Triggers</key>
				<array/>
				<key>UID</key>
				<string>{macro_uid}</string>
				<key>Actions</key>
				<array>
					<dict>
						<key>MacroActionType</key>
						<string>IfThenElse</string>
						<key>TimeOutAbortsMacro</key>
						<true/>
						<key>Conditions</key>
						<dict>
							<key>ConditionList</key>
							<array>
								<dict>
									<key>ConditionType</key>
									<string>AnyWindow</string>
									<key>AnyWindowConditionType</key>
									<string>EndsWith</string>
									<key>AnyWindowTitle</key>
									<string>{folder}</string>
									<key>IsFrontApplication</key>
									<false/>
									<key>Application</key>
									<dict>
										<key>BundleIdentifier</key>
										<string>{bundle_id}</string>
										<key>Name</key>
										<string>{app}</string>
										<key>NewFile</key>
										<string>/Applications/{app}.app</string>
									</dict>
								</dict>
							</array>
							<key>ConditionListMatch</key>
							<string>All</string>
						</dict>
						<key>ThenActions</key>
						<array>
							<dict>
								<key>MacroActionType</key>
								<string>ManipulateWindow</string>
								<key>Action</key>
								<string>SelectWindow</string>
								<key>Targeting</key>
								<string>WindowNameContaining</string>
								<key>TargetingType</key>
								<string>Specific</string>
								<key>WindowName</key>
								<string>{folder}</string>
								<key>TargetApplication</key>
								<dict>
									<key>BundleIdentifier</key>
									<string>{bundle_id}</string>
									<key>Name</key>
									<string>{app}</string>
									<key>NewFile</key>
									<string>/Applications/{app}.app</string>
								</dict>
							</dict>
						</array>
						<key>ElseActions</key>
						<array>
							<dict>
								<key>MacroActionType</key>
								<string>ExecuteShellScript</string>
								<key>DisplayKind</key>
								<string>Window</string>
								<key>HonourFailureSettings</key>
								<true/>
								<key>IncludeStdErr</key>
								<false/>
								<key>Path</key>
								<string></string>
								<key>Source</key>
								<string>Nothing</string>
								<key>Text</key>
								<string>open -a /Applications/{app}.app {path}</string>
								<key>TimeOutAbortsMacro</key>
								<true/>
								<key>TrimResults</key>
								<true/>
								<key>TrimResultsNew</key>
								<true/>
								<key>UseText</key>
								<true/>
							</dict>
						</array>
					</dict>
				</array>
			</dict>
		</array>
		<key>Name</key>
		<string>Global Macro Group</string>
	</dict>
</array>
</plist>"#,
        name = name_escaped,
        app = app_escaped,
        path = path_escaped,
        folder = folder_escaped,
        bundle_id = bundle_id_escaped
    )
}

fn import_macro_plist(plist: &str) -> Result<()> {
    let temp_path = "/tmp/km_macro_import.kmmacros";
    std::fs::write(temp_path, plist).context("failed to write temp plist")?;

    let status = Command::new("open")
        .arg(temp_path)
        .status()
        .context("failed to open macro file")?;

    // Give Keyboard Maestro time to process
    std::thread::sleep(std::time::Duration::from_millis(1000));

    let _ = std::fs::remove_file(temp_path);

    if !status.success() {
        bail!("failed to open macro file");
    }

    Ok(())
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn list_macros() -> Result<()> {
    let script = r#"
try
    get application id "com.stairways.keyboardmaestro.engine"
on error
    return "Keyboard Maestro not found"
end try

tell application id "com.stairways.keyboardmaestro.engine"
    gethotkeys with asstring and getall
end tell
"#;

    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .context("failed to run osascript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("osascript failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse plist and print macro names
    if let Some(macros) = parse_macro_names(&stdout) {
        for (name, category) in macros {
            println!("{}\t{}", name, category);
        }
    } else {
        print!("{}", stdout);
    }

    Ok(())
}

fn parse_macro_names(plist_str: &str) -> Option<Vec<(String, String)>> {
    // Simple extraction of macro names from plist XML
    let mut macros = Vec::new();
    let mut current_category = String::new();
    let mut in_macros_array = false;
    let mut next_is_name = false;
    let mut next_is_category_name = false;

    for line in plist_str.lines() {
        let trimmed = line.trim();

        if trimmed == "<key>name</key>" && !in_macros_array {
            next_is_category_name = true;
        } else if next_is_category_name && trimmed.starts_with("<string>") {
            if let Some(name) = extract_string_value(trimmed) {
                current_category = name;
            }
            next_is_category_name = false;
        } else if trimmed == "<key>macros</key>" {
            in_macros_array = true;
        } else if in_macros_array && trimmed == "</array>" {
            in_macros_array = false;
        } else if in_macros_array && trimmed == "<key>name</key>" {
            next_is_name = true;
        } else if next_is_name && trimmed.starts_with("<string>") {
            if let Some(name) = extract_string_value(trimmed) {
                macros.push((name, current_category.clone()));
            }
            next_is_name = false;
        }
    }

    if macros.is_empty() {
        None
    } else {
        Some(macros)
    }
}

fn extract_string_value(line: &str) -> Option<String> {
    let start = line.find("<string>")? + 8;
    let end = line.find("</string>")?;
    Some(line[start..end].to_string())
}

fn run_macro(name: &str) -> Result<()> {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"tell application "Keyboard Maestro Engine" to do script "{}""#,
        escaped
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("failed to run osascript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to run macro: {}", stderr);
    }

    println!("Ran macro: {}", name);
    Ok(())
}

fn inspect_macro(name: &str) -> Result<()> {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");

    // Get macro XML from Keyboard Maestro
    let script = format!(
        r#"
tell application "Keyboard Maestro"
    set targetMacro to first macro whose name is "{}"
    set macroXML to targetMacro's xml
    return macroXML
end tell
"#,
        escaped
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("failed to run osascript")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to get macro: {}", stderr);
    }

    let xml = String::from_utf8_lossy(&output.stdout);

    // Parse XML and extract actions as JSON
    let actions = parse_macro_actions(&xml)?;
    println!("{}", serde_json::to_string_pretty(&actions)?);

    Ok(())
}

fn parse_macro_actions(xml: &str) -> Result<Vec<serde_json::Value>> {
    // Convert plist XML to JSON using plutil
    let temp_path = "/tmp/km_macro_inspect.plist";
    std::fs::write(temp_path, xml).context("failed to write temp plist")?;

    let output = Command::new("plutil")
        .args(["-convert", "json", "-o", "-", temp_path])
        .output()
        .context("failed to run plutil")?;

    let _ = std::fs::remove_file(temp_path);

    if !output.status.success() {
        bail!("plutil failed to convert plist");
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("failed to parse JSON from plutil")?;

    // Extract Actions array from the macro
    let actions = json
        .get("Actions")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(actions)
}
