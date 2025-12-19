use std::collections::VecDeque;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(cmd) => run_command(cmd),
        None => interactive_select(),
    }
}

fn run_command(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Validate { path } => handle_validate(path.as_ref()),
        Commands::FocusCursorWindow(args) => run_focus_cursor_window(args),
        Commands::CleanNodeModules { path, dry_run } => clean_node_modules(&path, dry_run),
        Commands::Empty { path } => empty_dir(&path),
        Commands::Open { app, path } => open_in_app(&app, &path),
        Commands::WriteDoc { command } => match command {
            WriteDocCommands::Run { title } => write_doc(&title, true),
            WriteDocCommands::Paste { title } => write_doc(&title, false),
        },
        Commands::Windows { app } => list_app_windows(&app),
    }
}

const COMMANDS: &[(&str, &str)] = &[
    ("validate", "Validate a project directory against Flow conventions"),
    ("focus-cursor-window", "Focus the most recent Cursor window recorded in a state file"),
    ("clean-node-modules", "Recursively remove all node_modules directories under a path"),
    ("empty", "Remove all contents of a directory"),
    ("open", "Open a path in an app (focuses existing window if open)"),
    ("write-doc", "Convert title to slug and paste write docs/<slug> command"),
    ("windows", "List window titles for an app"),
];

fn interactive_select() -> Result<()> {
    let input: String = COMMANDS
        .iter()
        .map(|(name, desc)| format!("{name}: {desc}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut fzf = Command::new("fzf")
        .arg("--height=~50%")
        .arg("--layout=reverse")
        .arg("--border")
        .arg("--prompt=command> ")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to spawn fzf - is it installed?")?;

    if let Some(mut stdin) = fzf.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }

    let output = fzf.wait_with_output()?;

    if !output.status.success() {
        return Ok(());
    }

    let selection = String::from_utf8_lossy(&output.stdout);
    let cmd_name = selection.split(':').next().unwrap_or("").trim();

    if cmd_name.is_empty() {
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let status = Command::new(&exe).arg(cmd_name).status()?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

#[derive(Parser)]
#[command(name = "flow", version, about = "Flow CLI", propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate a project directory against Flow conventions.
    Validate {
        /// Path to the project directory (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Focus the most recent Cursor window recorded in a state file.
    FocusCursorWindow(FocusCursorWindowArgs),
    /// Recursively remove all node_modules directories under a path.
    CleanNodeModules {
        /// Root path to search for node_modules (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Perform a dry run without deleting anything.
        #[arg(long, short = 'n')]
        dry_run: bool,
    },
    /// Remove all contents of a directory (keeps the directory itself).
    Empty {
        /// Path to the directory to empty.
        path: PathBuf,
    },
    /// Open a path in an app (focuses existing window if already open).
    Open {
        /// App name (e.g., "Zed", "Cursor", "Code").
        app: String,
        /// Path to open.
        path: PathBuf,
    },
    /// Convert a title to a slug and paste "write docs/<slug>" into current app.
    WriteDoc {
        #[command(subcommand)]
        command: WriteDocCommands,
    },
    /// List window titles for an app.
    Windows {
        /// App name (e.g., "Zed", "Cursor", "Safari").
        app: String,
    },
}

#[derive(Args)]
struct FocusCursorWindowArgs {
    /// File that stores the last non-dot Cursor window title.
    #[arg(
        long = "state-file",
        env = "FLOW_CURSOR_LAST_WINDOW_FILE",
        value_name = "FILE"
    )]
    state_file: PathBuf,
}

#[derive(Subcommand)]
enum WriteDocCommands {
    /// Type the command and press return (default behavior).
    Run {
        /// Title to convert (e.g., "Hey there" -> "write docs/hey-there").
        title: String,
    },
    /// Just paste the text without pressing return.
    Paste {
        /// Title to convert (e.g., "Hey there" -> "write docs/hey-there").
        title: String,
    },
}

fn handle_validate(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }

    let dir = path
        .canonicalize()
        .with_context(|| format!("Unable to resolve directory {}", path.display()))?;

    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let mut issues = Vec::new();

    let gitignore_path = dir.join(".gitignore");
    if !gitignore_path.exists() {
        issues.push(format!(
            "Missing .gitignore file at {}",
            gitignore_path.display()
        ));
    } else {
        let gitignore_contents = fs::read_to_string(&gitignore_path)
            .with_context(|| format!("Unable to read {}", gitignore_path.display()))?;

        let has_core_comment = gitignore_contents
            .lines()
            .any(|line| line.trim() == "# core");

        if !has_core_comment {
            issues.push(format!(
                "{} missing required '# core' marker",
                gitignore_path.display()
            ));
        }
    }

    if issues.is_empty() {
        println!("Validation passed for {}", dir.display());
        Ok(())
    } else {
        for issue in &issues {
            eprintln!("- {issue}");
        }
        Err(anyhow!("Validation failed with {} issue(s)", issues.len()))
    }
}

fn clean_node_modules(path: &Path, dry_run: bool) -> Result<()> {
    let root = path
        .canonicalize()
        .with_context(|| format!("Unable to resolve path {}", path.display()))?;

    if !root.is_dir() {
        bail!("{} is not a directory", root.display());
    }

    println!("Scanning {}...", root.display());

    let (dirs_to_remove, scanned) = find_node_modules_bfs(&root);

    print!("\r\x1b[K");
    println!("Scanned {scanned} directories, found {} node_modules", dirs_to_remove.len());

    if dirs_to_remove.is_empty() {
        return Ok(());
    }

    if dry_run {
        println!("\nDry run - would remove:");
        for dir in &dirs_to_remove {
            println!("  {}", dir.display());
        }
        return Ok(());
    }

    let mut removed = 0;
    let mut failed = 0;
    let total = dirs_to_remove.len();

    println!("Removing {total} node_modules directories...");

    for (i, dir) in dirs_to_remove.iter().enumerate() {
        print!("\r  [{}/{}] removing...", i + 1, total);
        let _ = io::stdout().flush();

        match fs::remove_dir_all(dir) {
            Ok(()) => removed += 1,
            Err(e) => {
                eprintln!("\nFailed to remove {}: {e}", dir.display());
                failed += 1;
            }
        }
    }

    print!("\r\x1b[K");
    println!(
        "Removed {removed} director{}, {failed} failed",
        if removed == 1 { "y" } else { "ies" }
    );

    if failed > 0 {
        bail!("Failed to remove {failed} directories");
    }

    Ok(())
}

fn empty_dir(path: &Path) -> Result<()> {
    let dir = path
        .canonicalize()
        .with_context(|| format!("Unable to resolve path {}", path.display()))?;

    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let entries: Vec<_> = fs::read_dir(&dir)
        .with_context(|| format!("Unable to read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();

    if entries.is_empty() {
        println!("{} is already empty", dir.display());
        return Ok(());
    }

    print!(
        "Are you sure you want to empty {} ({} entries)? [y/N] ",
        dir.display(),
        entries.len()
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
        println!("Aborted.");
        return Ok(());
    }

    println!("Removing {} entries from {}...", entries.len(), dir.display());

    let mut removed = 0;
    let mut failed = 0;

    for entry in entries {
        let path = entry.path();
        let result = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };

        match result {
            Ok(()) => removed += 1,
            Err(e) => {
                eprintln!("Failed to remove {}: {e}", path.display());
                failed += 1;
            }
        }
    }

    println!("Removed {removed}, {failed} failed");

    if failed > 0 {
        bail!("Failed to remove {failed} entries");
    }

    Ok(())
}

fn find_node_modules_bfs(root: &Path) -> (Vec<PathBuf>, usize) {
    let mut found = Vec::new();
    let mut queue = VecDeque::new();
    let mut scanned = 0usize;

    queue.push_back(root.to_path_buf());

    while let Some(dir) = queue.pop_front() {
        scanned += 1;

        if scanned % 5000 == 0 {
            print!("\r  scanned {scanned} directories, found {} node_modules...", found.len());
            let _ = io::stdout().flush();
        }

        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();

            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !is_dir {
                continue;
            }

            if entry.file_name() == "node_modules" {
                found.push(path);
                // Don't descend into node_modules
            } else {
                queue.push_back(path);
            }
        }
    }

    (found, scanned)
}

fn run_focus_cursor_window(args: FocusCursorWindowArgs) -> Result<()> {
    let window_title = read_last_window_title(&args.state_file)?;

    println!(
        "Latest Cursor window from {}: {}",
        args.state_file.display(),
        window_title
    );

    let attempt = focus_cursor_window_by_title(&window_title)?;
    if attempt.focused {
        println!("Focused Cursor window \"{window_title}\"");
        return Ok(());
    }

    if let Some(reason) = attempt.reason {
        println!("{reason}");
    } else {
        println!("Unable to focus Cursor window \"{window_title}\"");
    }

    Ok(())
}

fn read_last_window_title(path: &Path) -> Result<String> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let trimmed = contents.trim();
    if !trimmed.is_empty() {
        return Ok(trimmed.to_owned());
    }

    if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
        let fallback = file_name.trim();
        if !fallback.is_empty() {
            return Ok(fallback.to_owned());
        }
    }

    bail!("{} did not contain a Cursor window title", path.display());
}

struct FocusCursorAttempt {
    focused: bool,
    reason: Option<String>,
}

impl FocusCursorAttempt {
    fn focused() -> Self {
        Self {
            focused: true,
            reason: None,
        }
    }

    fn info(reason: impl Into<String>) -> Self {
        Self {
            focused: false,
            reason: Some(reason.into()),
        }
    }
}

fn focus_cursor_window_by_title(title: &str) -> Result<FocusCursorAttempt> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        bail!("window title cannot be empty");
    }

    let script = format!(
        r#"set targetTitle to "{title}"
set matched to false

tell application "System Events"
	if not (exists application process "Cursor") then
		return "NOT_RUNNING"
	end if

	tell application process "Cursor"
		repeat with w in windows
			set winName to ""
			try
				set winName to name of w
			end try

			if winName is targetTitle then
				set matched to true
				try
					set frontmost to true
				end try
				try
					set value of attribute "AXMain" of w to true
				end try
				try
					perform action "AXRaise" of w
				end try
				exit repeat
			end if
		end repeat
	end tell
end tell

if matched then
	tell application "Cursor" to activate
	return "FOCUSED"
end if

return "NOT_FOUND""#,
        title = escape_apple_script_string(trimmed)
    );

    let result = run_osascript(&script)?;
    match result.as_str() {
        "FOCUSED" => match cursor_front_window_title() {
            Ok(current) => {
                let normalized_current = normalize_window_title(&current);
                let normalized_target = normalize_window_title(trimmed);
                if normalized_current == normalized_target {
                    Ok(FocusCursorAttempt::focused())
                } else if current.is_empty() {
                    Ok(FocusCursorAttempt::info(
                        "Cursor focused an unnamed window; please try again",
                    ))
                } else {
                    Ok(FocusCursorAttempt::info(format!(
                        "Cursor focused \"{current}\" instead"
                    )))
                }
            }
            Err(_) => Ok(FocusCursorAttempt::info(
                "Unable to verify Cursor window state",
            )),
        },
        "NOT_RUNNING" => Ok(FocusCursorAttempt::info("Cursor is not running")),
        "NOT_FOUND" => Ok(FocusCursorAttempt::info(format!(
            "No Cursor window titled \"{trimmed}\" was found"
        ))),
        other => {
            if other.is_empty() {
                bail!("focus Cursor window returned an empty response");
            }
            bail!("unexpected osascript response: {other}");
        }
    }
}

fn run_osascript(script: &str) -> Result<String> {
    let output = match Command::new("osascript").arg("-e").arg(script).output() {
        Ok(output) => output,
        Err(err) => {
            if err.kind() == io::ErrorKind::NotFound {
                return Err(anyhow!("osascript not found in PATH"));
            }
            return Err(err).context("failed to run osascript");
        }
    };

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let mut message = stderr;
        if message.is_empty() {
            message = stdout;
        } else if !stdout.is_empty() {
            message.push_str("; ");
            message.push_str(&stdout);
        }
        if message.is_empty() {
            message = "osascript exited with an error".to_string();
        }
        bail!(message);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn cursor_front_window_title() -> Result<String> {
    let script = r#"tell application "System Events"
	if not (exists application process "Cursor") then
		return ""
	end if

	tell application process "Cursor"
		repeat with w in windows
			try
				if value of attribute "AXMain" of w is true then
					return name of w
				end if
			end try
		end repeat

		if (count of windows) > 0 then
			try
				return name of window 1
			end try
		end if
	end tell
end tell

return ""#;

    Ok(run_osascript(script)?)
}

fn normalize_window_title(title: &str) -> String {
    title.trim().to_owned()
}

fn escape_apple_script_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn open_in_app(app: &str, path: &Path) -> Result<()> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("Unable to resolve path {}", path.display()))?;

    // Get the folder name to match in window titles
    let folder_name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Unable to get folder name from path"))?;

    // Try to focus existing window first
    let focused = focus_app_window(app, folder_name)?;

    if focused {
        println!("Focused {} window for {}", app, folder_name);
    } else {
        // No existing window, open the path
        println!("Opening {} in {}...", canonical.display(), app);
        let status = Command::new("open")
            .arg("-a")
            .arg(app)
            .arg(&canonical)
            .status()
            .context("failed to run open command")?;

        if !status.success() {
            bail!("open command failed with status {}", status);
        }
    }

    Ok(())
}

fn focus_app_window(app: &str, folder_name: &str) -> Result<bool> {
    let escaped_app = escape_apple_script_string(app);
    let escaped_folder = escape_apple_script_string(folder_name);

    let script = format!(
        r#"set targetFolder to "{folder}"
set appName to "{app}"

tell application "System Events"
    if not (exists application process appName) then
        return "NOT_RUNNING"
    end if

    tell application process appName
        repeat with w in windows
            set winName to ""
            try
                set winName to name of w
            end try

            ignoring case
                if winName ends with targetFolder then
                    try
                        set frontmost to true
                    end try
                    try
                        set value of attribute "AXMain" of w to true
                    end try
                    try
                        perform action "AXRaise" of w
                    end try
                    tell application appName to activate
                    return "FOCUSED"
                end if
            end ignoring
        end repeat
    end tell
end tell

return "NOT_FOUND""#,
        folder = escaped_folder,
        app = escaped_app
    );

    let result = run_osascript(&script)?;
    Ok(result == "FOCUSED")
}

fn write_doc(title: &str, press_return: bool) -> Result<()> {
    let slug = title_to_slug(title);
    let text = format!("write docs/{}", slug);

    let escaped = escape_apple_script_string(&text);
    let script = if press_return {
        format!(
            r#"tell application "System Events"
    keystroke "{}"
    keystroke return
end tell"#,
            escaped
        )
    } else {
        format!(
            r#"tell application "System Events"
    keystroke "{}"
    keystroke return using shift down
end tell"#,
            escaped
        )
    };

    run_osascript(&script)?;
    Ok(())
}

fn title_to_slug(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else if c.is_whitespace() || c == '-' || c == '_' {
                '-'
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn list_app_windows(app: &str) -> Result<()> {
    let escaped_app = escape_apple_script_string(app);

    let script = format!(
        r#"set appName to "{app}"
set windowList to {{}}

tell application "System Events"
    if not (exists application process appName) then
        return "NOT_RUNNING"
    end if

    tell application process appName
        repeat with w in windows
            try
                set winName to name of w
                if winName is not "" then
                    set end of windowList to winName
                end if
            end try
        end repeat
    end tell
end tell

set AppleScript's text item delimiters to linefeed
return windowList as text"#,
        app = escaped_app
    );

    let result = run_osascript(&script)?;

    if result == "NOT_RUNNING" {
        println!("{} is not running", app);
        return Ok(());
    }

    if result.is_empty() {
        println!("{} has no windows", app);
        return Ok(());
    }

    for line in result.lines() {
        println!("{}", line);
    }

    Ok(())
}
