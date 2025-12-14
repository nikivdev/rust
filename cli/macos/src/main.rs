use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
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
        Commands::Shortcuts { all } => list_shortcuts(all),
        Commands::Apps { limit } => list_apps(limit),
        Commands::ClipImg => clip_img(),
        Commands::Energy { limit } => list_energy(limit),
    }
}

#[derive(Parser)]
#[command(name = "macos", version, about = "macOS utilities", propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List keyboard shortcuts in use on this Mac
    Shortcuts {
        /// Show all shortcuts including disabled ones
        #[arg(long, short)]
        all: bool,
    },
    /// List running apps sorted by RAM usage
    Apps {
        /// Limit number of apps shown (shows all if not specified)
        #[arg(long, short)]
        limit: Option<usize>,
    },
    /// Save clipboard image to file and put file path in clipboard.
    ///
    /// Useful for pasting images into apps that only accept file paths
    /// (e.g., Claude Code in Zed).
    ClipImg,
    /// List apps/processes consuming most energy (by CPU usage)
    ///
    /// Useful for finding battery drains on flights.
    Energy {
        /// Limit number of processes shown (default: 15)
        #[arg(long, short)]
        limit: Option<usize>,
    },
}

fn list_shortcuts(show_all: bool) -> Result<()> {
    let mut shortcuts: BTreeMap<String, Vec<ShortcutInfo>> = BTreeMap::new();

    // System symbolic hotkeys
    if let Ok(system) = read_symbolic_hotkeys() {
        for s in system {
            if show_all || s.enabled {
                shortcuts
                    .entry("System".to_string())
                    .or_default()
                    .push(s);
            }
        }
    }

    // App-specific shortcuts from NSUserKeyEquivalents
    if let Ok(app_shortcuts) = read_app_shortcuts() {
        for (app, app_shortcuts_list) in app_shortcuts {
            for s in app_shortcuts_list {
                if show_all || s.enabled {
                    shortcuts.entry(app.clone()).or_default().push(s);
                }
            }
        }
    }

    // Custom services shortcuts
    if let Ok(services) = read_services_shortcuts() {
        for s in services {
            if show_all || s.enabled {
                shortcuts
                    .entry("Services".to_string())
                    .or_default()
                    .push(s);
            }
        }
    }

    if shortcuts.is_empty() {
        println!("No keyboard shortcuts found.");
        return Ok(());
    }

    for (category, mut list) in shortcuts {
        list.sort_by(|a, b| a.shortcut.cmp(&b.shortcut));
        println!("\n## {category}");
        println!();
        for info in list {
            let status = if info.enabled { "" } else { " (disabled)" };
            println!("  {:<24} {}{}", info.shortcut, info.action, status);
        }
    }

    println!();
    Ok(())
}

#[derive(Debug, Clone)]
struct ShortcutInfo {
    shortcut: String,
    action: String,
    enabled: bool,
}

fn read_symbolic_hotkeys() -> Result<Vec<ShortcutInfo>> {
    let plist_path = expand_tilde("~/Library/Preferences/com.apple.symbolichotkeys.plist");

    let output = Command::new("plutil")
        .args(["-convert", "xml1", "-o", "-", &plist_path])
        .output()
        .context("failed to run plutil")?;

    if !output.status.success() {
        anyhow::bail!("plutil failed to convert plist");
    }

    let value: plist::Value =
        plist::from_bytes(&output.stdout).context("failed to parse symbolic hotkeys plist")?;

    let mut results = Vec::new();

    if let Some(dict) = value.as_dictionary() {
        if let Some(hotkeys) = dict.get("AppleSymbolicHotKeys").and_then(|v| v.as_dictionary()) {
            for (key, val) in hotkeys {
                if let Some(info) = parse_symbolic_hotkey(key, val) {
                    results.push(info);
                }
            }
        }
    }

    Ok(results)
}

fn parse_symbolic_hotkey(id: &str, value: &plist::Value) -> Option<ShortcutInfo> {
    let dict = value.as_dictionary()?;

    let enabled = dict
        .get("enabled")
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);

    let params = dict.get("value")?.as_dictionary()?.get("parameters")?;
    let params_array = params.as_array()?;

    if params_array.len() < 3 {
        return None;
    }

    let key_code = params_array.get(1)?.as_signed_integer()? as u16;
    let modifiers = params_array.get(2)?.as_signed_integer()? as u32;

    let shortcut = format_shortcut(modifiers, key_code);
    let action = symbolic_hotkey_name(id);

    Some(ShortcutInfo {
        shortcut,
        action,
        enabled,
    })
}

fn read_app_shortcuts() -> Result<BTreeMap<String, Vec<ShortcutInfo>>> {
    let mut results: BTreeMap<String, Vec<ShortcutInfo>> = BTreeMap::new();

    // Read global NSUserKeyEquivalents
    if let Ok(global) = read_nsuserkey_equivalents("NSGlobalDomain") {
        if !global.is_empty() {
            results.insert("Global App Shortcuts".to_string(), global);
        }
    }

    // Get list of apps with custom shortcuts
    let output = Command::new("defaults")
        .args(["domains"])
        .output()
        .context("failed to run defaults domains")?;

    if output.status.success() {
        let domains = String::from_utf8_lossy(&output.stdout);
        for domain in domains.split(", ") {
            let domain = domain.trim();
            if domain.is_empty() || domain == "NSGlobalDomain" {
                continue;
            }

            if let Ok(shortcuts) = read_nsuserkey_equivalents(domain) {
                if !shortcuts.is_empty() {
                    let app_name = domain
                        .rsplit('.')
                        .next()
                        .unwrap_or(domain)
                        .to_string();
                    results.insert(app_name, shortcuts);
                }
            }
        }
    }

    Ok(results)
}

fn read_nsuserkey_equivalents(domain: &str) -> Result<Vec<ShortcutInfo>> {
    let output = Command::new("defaults")
        .args(["read", domain, "NSUserKeyEquivalents"])
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_nsuserkey_output(&stdout)
}

fn parse_nsuserkey_output(output: &str) -> Result<Vec<ShortcutInfo>> {
    let mut results = Vec::new();

    // Parse the defaults output format: { "Menu Item" = "shortcut"; }
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with('{') || line.starts_with('}') || line.is_empty() {
            continue;
        }

        // Parse: "Action Name" = "@$k";
        if let Some((action, shortcut)) = line.split_once(" = ") {
            let action = action.trim().trim_matches('"').to_string();
            let shortcut = shortcut
                .trim()
                .trim_end_matches(';')
                .trim()
                .trim_matches('"');

            let formatted = format_nsuserkey_shortcut(shortcut);

            results.push(ShortcutInfo {
                shortcut: formatted,
                action,
                enabled: true,
            });
        }
    }

    Ok(results)
}

fn format_nsuserkey_shortcut(raw: &str) -> String {
    let mut parts = Vec::new();

    for c in raw.chars() {
        match c {
            '@' => parts.push("Cmd"),
            '$' => parts.push("Shift"),
            '~' => parts.push("Opt"),
            '^' => parts.push("Ctrl"),
            _ => {
                let key = match c {
                    '\u{F700}' => "Up",
                    '\u{F701}' => "Down",
                    '\u{F702}' => "Left",
                    '\u{F703}' => "Right",
                    '\u{F704}' => "F1",
                    '\u{F705}' => "F2",
                    '\u{F706}' => "F3",
                    '\u{F707}' => "F4",
                    '\u{F708}' => "F5",
                    '\u{F709}' => "F6",
                    '\u{F70A}' => "F7",
                    '\u{F70B}' => "F8",
                    '\u{F70C}' => "F9",
                    '\u{F70D}' => "F10",
                    '\u{F70E}' => "F11",
                    '\u{F70F}' => "F12",
                    '\u{F728}' => "Delete",
                    '\u{F729}' => "Home",
                    '\u{F72B}' => "End",
                    '\u{F72C}' => "PageUp",
                    '\u{F72D}' => "PageDown",
                    '\r' | '\u{03}' => "Return",
                    '\t' => "Tab",
                    ' ' => "Space",
                    '\u{1B}' => "Esc",
                    _ => {
                        return format!(
                            "{}+{}",
                            parts.join("+"),
                            c.to_uppercase().collect::<String>()
                        );
                    }
                };
                return format!("{}+{}", parts.join("+"), key);
            }
        }
    }

    parts.join("+")
}

fn read_services_shortcuts() -> Result<Vec<ShortcutInfo>> {
    let plist_path = expand_tilde("~/Library/Preferences/pbs.plist");

    if !Path::new(&plist_path).exists() {
        return Ok(Vec::new());
    }

    let output = Command::new("plutil")
        .args(["-convert", "xml1", "-o", "-", &plist_path])
        .output()
        .context("failed to run plutil for services")?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let value: plist::Value =
        plist::from_bytes(&output.stdout).context("failed to parse services plist")?;

    let mut results = Vec::new();

    if let Some(dict) = value.as_dictionary() {
        if let Some(services) = dict
            .get("NSServicesStatus")
            .and_then(|v| v.as_dictionary())
        {
            for (key, val) in services {
                if let Some(service_dict) = val.as_dictionary() {
                    if let Some(shortcut) = service_dict
                        .get("key_equivalent")
                        .and_then(|v| v.as_string())
                    {
                        if !shortcut.is_empty() {
                            let enabled = service_dict
                                .get("enabled_context_menu")
                                .and_then(|v| v.as_boolean())
                                .unwrap_or(true);

                            let action = key
                                .split(" - ")
                                .last()
                                .unwrap_or(key)
                                .to_string();

                            results.push(ShortcutInfo {
                                shortcut: format_nsuserkey_shortcut(shortcut),
                                action,
                                enabled,
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

fn format_shortcut(modifiers: u32, key_code: u16) -> String {
    let mut parts = Vec::new();

    // Carbon modifier flags
    if modifiers & (1 << 17) != 0 {
        parts.push("Shift");
    }
    if modifiers & (1 << 18) != 0 {
        parts.push("Ctrl");
    }
    if modifiers & (1 << 19) != 0 {
        parts.push("Opt");
    }
    if modifiers & (1 << 20) != 0 {
        parts.push("Cmd");
    }

    let key = keycode_to_string(key_code);
    parts.push(&key);

    parts.join("+")
}

fn keycode_to_string(code: u16) -> String {
    match code {
        0 => "A",
        1 => "S",
        2 => "D",
        3 => "F",
        4 => "H",
        5 => "G",
        6 => "Z",
        7 => "X",
        8 => "C",
        9 => "V",
        11 => "B",
        12 => "Q",
        13 => "W",
        14 => "E",
        15 => "R",
        16 => "Y",
        17 => "T",
        18 => "1",
        19 => "2",
        20 => "3",
        21 => "4",
        22 => "6",
        23 => "5",
        24 => "=",
        25 => "9",
        26 => "7",
        27 => "-",
        28 => "8",
        29 => "0",
        30 => "]",
        31 => "O",
        32 => "U",
        33 => "[",
        34 => "I",
        35 => "P",
        36 => "Return",
        37 => "L",
        38 => "J",
        39 => "'",
        40 => "K",
        41 => ";",
        42 => "\\",
        43 => ",",
        44 => "/",
        45 => "N",
        46 => "M",
        47 => ".",
        48 => "Tab",
        49 => "Space",
        50 => "`",
        51 => "Delete",
        53 => "Escape",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        99 => "F3",
        100 => "F8",
        101 => "F9",
        103 => "F11",
        105 => "F13",
        106 => "F16",
        107 => "F14",
        109 => "F10",
        111 => "F12",
        113 => "F15",
        114 => "Help",
        115 => "Home",
        116 => "PageUp",
        117 => "ForwardDelete",
        118 => "F4",
        119 => "End",
        120 => "F2",
        121 => "PageDown",
        122 => "F1",
        123 => "Left",
        124 => "Right",
        125 => "Down",
        126 => "Up",
        _ => return format!("Key{}", code),
    }
    .to_string()
}

fn symbolic_hotkey_name(id: &str) -> String {
    match id {
        "7" => "Move focus to menu bar",
        "8" => "Move focus to Dock",
        "9" => "Move focus to active/next window",
        "10" => "Move focus to window toolbar",
        "11" => "Move focus to floating window",
        "12" => "Change the way Tab moves focus",
        "13" => "Turn zoom on or off",
        "15" => "Zoom in",
        "17" => "Turn VoiceOver on or off",
        "19" => "Zoom out",
        "21" => "Invert colors",
        "23" => "Turn image smoothing on or off",
        "25" => "Increase contrast",
        "26" => "Decrease contrast",
        "27" => "Move focus to next window",
        "28" => "Save picture of screen as file",
        "29" => "Copy picture of screen to clipboard",
        "30" => "Save picture of selected area as file",
        "31" => "Copy picture of selected area to clipboard",
        "32" => "Mission Control",
        "33" => "Application windows",
        "34" => "Show Desktop",
        "35" => "Move focus to the window drawer",
        "36" => "Dashboard",
        "37" => "Turn Dock hiding on/off",
        "52" => "Turn focus following on or off",
        "57" => "Spotlight search field",
        "60" => "Select previous input source",
        "61" => "Select next input source",
        "62" => "Show Spotlight window",
        "64" => "Show Spotlight search",
        "65" => "Switch to Desktop 1",
        "70" => "Switch to Desktop 2",
        "71" => "Switch to Desktop 3",
        "72" => "Switch to Desktop 4",
        "73" => "Show Notification Center",
        "75" => "Switch to Desktop 5",
        "76" => "Switch to Desktop 6",
        "77" => "Switch to Desktop 7",
        "78" => "Switch to Desktop 8",
        "79" => "Move left a space",
        "80" => "Move right a space",
        "81" => "Show Launchpad",
        "118" => "Switch to Desktop 9",
        "119" => "Switch to Desktop 10",
        "160" => "Show Accessibility controls",
        "162" => "Show Quick Note",
        "163" => "Screenshot and recording options",
        "164" => "Show/hide dictation",
        "175" => "Toggle Stage Manager",
        "179" => "Quick Look",
        _ => return format!("Hotkey {}", id),
    }
    .to_string()
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

// ============================================================================
// Apps command
// ============================================================================

#[derive(Debug)]
struct AppInfo {
    name: String,
    pid: u32,
    memory_bytes: u64,
}

fn list_apps(limit: Option<usize>) -> Result<()> {
    let mut apps = get_running_apps()?;

    // Sort by memory usage descending
    apps.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));

    if apps.is_empty() {
        println!("No running apps found.");
        return Ok(());
    }

    let total_apps = apps.len();
    let apps: Vec<_> = if let Some(limit) = limit {
        apps.into_iter().take(limit).collect()
    } else {
        apps
    };

    if apps.len() < total_apps {
        println!("Running apps by RAM usage (showing {}/{}):\n", apps.len(), total_apps);
    } else {
        println!("Running apps by RAM usage ({}):\n", total_apps);
    }

    for app in &apps {
        let mem = format_bytes(app.memory_bytes);
        println!("{:<32} {:>10}  (pid {})", app.name, mem, app.pid);
    }

    Ok(())
}

fn get_running_apps() -> Result<Vec<AppInfo>> {
    use std::collections::HashMap;

    // Run lsappinfo and ps in parallel
    let lsappinfo_handle = std::thread::spawn(|| {
        Command::new("lsappinfo")
            .args(["list", "-apps"])
            .output()
    });

    let ps_handle = std::thread::spawn(|| {
        Command::new("ps")
            .args(["-axo", "pid,rss"])
            .output()
    });

    // Wait for lsappinfo
    let lsappinfo_output = lsappinfo_handle
        .join()
        .map_err(|_| anyhow::anyhow!("lsappinfo thread panicked"))?
        .context("failed to run lsappinfo")?;

    let lsappinfo_stdout = String::from_utf8_lossy(&lsappinfo_output.stdout);

    // Parse lsappinfo output to get app name -> pid mapping
    // Format:  5) "Warp" ASN:0x0-0xe00e:
    //              pid = 644 type="Foreground" ...
    // Only include apps with type="Foreground" (actual GUI apps, not helpers)
    let mut app_pids: HashMap<String, u32> = HashMap::new();
    let mut current_app: Option<String> = None;

    for line in lsappinfo_stdout.lines() {
        let trimmed = line.trim();
        // App name line starts with number: 5) "Warp" ASN:...
        if let Some(paren_pos) = trimmed.find(')') {
            let after_paren = &trimmed[paren_pos + 1..].trim_start();
            if after_paren.starts_with('"') {
                if let Some(quote_end) = after_paren[1..].find('"') {
                    current_app = Some(after_paren[1..quote_end + 1].to_string());
                }
            }
        }
        // PID line: pid = 660 type="Foreground"
        // Only include Foreground apps (actual GUI apps with windows)
        if trimmed.starts_with("pid =") || trimmed.starts_with("pid=") {
            if let Some(app) = current_app.take() {
                // Check if this is a Foreground app
                if trimmed.contains("type=\"Foreground\"") {
                    // Extract pid value - it's after "pid =" and before next space
                    let pid_part = trimmed.strip_prefix("pid =").or_else(|| trimmed.strip_prefix("pid=")).unwrap_or("");
                    if let Some(pid_str) = pid_part.trim().split_whitespace().next() {
                        if let Ok(pid) = pid_str.parse::<u32>() {
                            app_pids.insert(app, pid);
                        }
                    }
                }
            }
        }
    }

    // Wait for ps
    let ps_output = ps_handle
        .join()
        .map_err(|_| anyhow::anyhow!("ps thread panicked"))?
        .context("failed to run ps")?;

    let mut mem_map: HashMap<u32, u64> = HashMap::new();
    let ps_stdout = String::from_utf8_lossy(&ps_output.stdout);
    for line in ps_stdout.lines().skip(1) {
        let mut parts = line.split_whitespace();
        if let (Some(pid_str), Some(rss_str)) = (parts.next(), parts.next()) {
            if let (Ok(pid), Ok(rss)) = (pid_str.parse::<u32>(), rss_str.parse::<u64>()) {
                mem_map.insert(pid, rss * 1024);
            }
        }
    }

    // Build result
    let mut apps: Vec<AppInfo> = app_pids
        .into_iter()
        .map(|(name, pid)| {
            let memory_bytes = mem_map.get(&pid).copied().unwrap_or(0);
            AppInfo {
                name,
                pid,
                memory_bytes,
            }
        })
        .collect();

    // Filter out apps with 0 memory (not actually running)
    apps.retain(|a| a.memory_bytes > 0);

    Ok(apps)
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

// ============================================================================
// ClipImg command
// ============================================================================

fn clip_img() -> Result<()> {
    use std::fs;

    let output_dir = expand_tilde("~/images/temp");
    fs::create_dir_all(&output_dir).context("failed to create output directory")?;

    // Use osascript to check if clipboard has image and get it as PNG
    let check_script = r#"
use framework "AppKit"
set pb to current application's NSPasteboard's generalPasteboard()
set imgTypes to {current application's NSPasteboardTypePNG, current application's NSPasteboardTypeTIFF, "public.jpeg"}
set hasImage to false
repeat with imgType in imgTypes
    if (pb's canReadItemWithDataConformingToTypes:{imgType}) then
        set hasImage to true
        exit repeat
    end if
end repeat
return hasImage as text
"#;

    let check_output = Command::new("osascript")
        .arg("-e")
        .arg(check_script)
        .output()
        .context("failed to check clipboard")?;

    let has_image = String::from_utf8_lossy(&check_output.stdout)
        .trim()
        .to_lowercase();

    if has_image != "true" {
        anyhow::bail!("clipboard does not contain an image");
    }

    // Generate hash for filename using current time
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let hash = format!("{:x}", timestamp);
    let filename = format!("{}.png", &hash[..12.min(hash.len())]);
    let output_path = format!("{}/{}", output_dir, filename);

    // Use pngpaste to save clipboard image (brew install pngpaste)
    // Fallback to osascript if pngpaste not available
    let pngpaste_result = Command::new("pngpaste")
        .arg(&output_path)
        .status();

    match pngpaste_result {
        Ok(status) if status.success() => {}
        _ => {
            // Fallback: use AppleScript + sips
            let script = format!(
                r#"
use framework "AppKit"
set pb to current application's NSPasteboard's generalPasteboard()
set imgData to pb's dataForType:(current application's NSPasteboardTypeTIFF)
if imgData is missing value then
    set imgData to pb's dataForType:(current application's NSPasteboardTypePNG)
end if
if imgData is missing value then
    error "No image data in clipboard"
end if
set bitmapRep to current application's NSBitmapImageRep's imageRepWithData:imgData
set pngData to bitmapRep's representationUsingType:(current application's NSBitmapImageFileTypePNG) properties:(missing value)
set outPath to POSIX path of "{}"
pngData's writeToFile:outPath atomically:true
return outPath
"#,
                output_path
            );

            let result = Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .context("failed to save clipboard image")?;

            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                anyhow::bail!("failed to save image: {}", stderr);
            }
        }
    }

    // Verify file was created
    if !Path::new(&output_path).exists() {
        anyhow::bail!("failed to create image file");
    }

    // Put file path in clipboard
    let pbcopy = Command::new("sh")
        .arg("-c")
        .arg(format!("echo -n '{}' | pbcopy", output_path))
        .status()
        .context("failed to copy path to clipboard")?;

    if !pbcopy.success() {
        anyhow::bail!("failed to copy path to clipboard");
    }

    println!("{}", output_path);
    Ok(())
}

// ============================================================================
// Energy command
// ============================================================================

#[derive(Debug)]
struct ProcessEnergy {
    name: String,
    pid: u32,
    cpu_percent: f64,
}

fn list_energy(limit: Option<usize>) -> Result<()> {
    let limit = limit.unwrap_or(15);

    // Use top to get accurate CPU snapshot (samples for 1 second)
    // -l 2 means 2 samples, second one has actual CPU averages
    // -n 100 limits to top 100 processes
    let output = Command::new("top")
        .args(["-l", "2", "-n", "100", "-stats", "pid,cpu,command"])
        .output()
        .context("failed to run top")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("top command failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Find the second "PID" header (start of second sample's process list)
    // top -l 2 outputs two samples, the second one has accurate CPU %
    let mut pid_headers = 0;
    let mut start_idx = None;
    let lines: Vec<&str> = stdout.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        // Header line contains both PID and CPU
        if line.contains("PID") && line.contains("CPU") {
            pid_headers += 1;
            if pid_headers == 2 {
                start_idx = Some(i + 1);
                break;
            }
        }
    }

    // Fallback to first sample if only one found
    let start = match start_idx {
        Some(idx) => idx,
        None if pid_headers == 1 => {
            lines.iter().position(|l| l.contains("PID") && l.contains("CPU"))
                .map(|i| i + 1)
                .ok_or_else(|| anyhow::anyhow!("failed to parse top output"))?
        }
        None => anyhow::bail!("failed to parse top output (no PID headers found)"),
    };

    // Parse process lines until we hit a non-process line
    let mut processes: Vec<ProcessEnergy> = Vec::new();
    for line in &lines[start..] {
        let line = line.trim();
        // Stop at empty or summary lines
        if line.is_empty() || line.starts_with("Processes:") {
            break;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            if let (Ok(pid), Ok(cpu)) = (parts[0].parse::<u32>(), parts[1].parse::<f64>()) {
                if cpu > 0.0 {
                    processes.push(ProcessEnergy {
                        name: parts[2..].join(" "),
                        pid,
                        cpu_percent: cpu,
                    });
                }
            }
        }
    }

    // Sort by CPU descending
    processes.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap());

    if processes.is_empty() {
        println!("No processes with significant CPU usage found.");
        return Ok(());
    }

    let total = processes.len();
    let processes: Vec<_> = processes.into_iter().take(limit).collect();

    println!("Top energy consumers (showing {}/{}):\n", processes.len(), total);
    println!("{:<8} {:>8}  {}", "PID", "CPU %", "PROCESS");
    println!("{}", "-".repeat(50));

    for p in &processes {
        println!("{:<8} {:>7.1}%  {}", p.pid, p.cpu_percent, p.name);
    }

    println!("\nTip: Use `kill <PID>` or quit apps to save battery.");
    Ok(())
}
