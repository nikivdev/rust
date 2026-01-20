use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Debug)]
struct Trace {
    timestamp: u64,
    cwd: String,
    cmd: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn trail_dir() -> PathBuf {
    dirs::home_dir().unwrap().join(".trail")
}

fn hash_path(path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn save_trace(trace: &Trace) {
    let dir = trail_dir();
    let by_path_dir = dir.join("by-path");
    fs::create_dir_all(&by_path_dir).ok();

    let json = serde_json::to_string(trace).unwrap();

    // Save as last global
    fs::write(dir.join("last.json"), &json).ok();

    // Save by path
    let path_hash = hash_path(&trace.cwd);
    fs::write(by_path_dir.join(format!("{}.json", path_hash)), &json).ok();
}

fn load_last_global() -> Option<Trace> {
    let path = trail_dir().join("last.json");
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn load_last_for_path(cwd: &str) -> Option<Trace> {
    let path_hash = hash_path(cwd);
    let path = trail_dir().join("by-path").join(format!("{}.json", path_hash));
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn run_command(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("Usage: trail run <command> [args...]");
        return 1;
    }

    let cwd = env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let cmd_str = args.join(" ");

    let mut child = Command::new(&args[0])
        .args(&args[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("Failed to run '{}': {}", args[0], e);
            std::process::exit(127);
        });

    let stdout_content: String;
    let stderr_content: String;

    // Read stdout in real-time and capture
    let stdout = child.stdout.take();
    let stdout_handle = thread::spawn(move || {
        let mut captured = String::new();
        if let Some(out) = stdout {
            let reader = BufReader::new(out);
            for line in reader.lines() {
                if let Ok(line) = line {
                    println!("{}", line);
                    captured.push_str(&line);
                    captured.push('\n');
                }
            }
        }
        captured
    });

    // Read stderr in real-time and capture
    let stderr = child.stderr.take();
    let stderr_handle = thread::spawn(move || {
        let mut captured = String::new();
        if let Some(err) = stderr {
            let reader = BufReader::new(err);
            for line in reader.lines() {
                if let Ok(line) = line {
                    eprintln!("{}", line);
                    captured.push_str(&line);
                    captured.push('\n');
                }
            }
        }
        captured
    });

    let status = child.wait().unwrap();
    stdout_content = stdout_handle.join().unwrap_or_default();
    stderr_content = stderr_handle.join().unwrap_or_default();

    let exit_code = status.code().unwrap_or(-1);

    let trace = Trace {
        timestamp,
        cwd,
        cmd: cmd_str,
        exit_code,
        stdout: stdout_content,
        stderr: stderr_content,
    };

    save_trace(&trace);
    exit_code
}

fn show_last(global: bool, show_stderr: bool) {
    let cwd = env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let trace = if global {
        load_last_global()
    } else {
        load_last_for_path(&cwd)
    };

    match trace {
        Some(t) => {
            if show_stderr {
                print!("{}", t.stderr);
            } else {
                print!("{}", t.stdout);
            }
        }
        None => {
            eprintln!("No trace found");
        }
    }
}

fn show_info(global: bool) {
    let cwd = env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let trace = if global {
        load_last_global()
    } else {
        load_last_for_path(&cwd)
    };

    match trace {
        Some(t) => {
            eprintln!("cmd:    {}", t.cmd);
            eprintln!("cwd:    {}", t.cwd);
            eprintln!("exit:   {}", t.exit_code);
            eprintln!("stdout: {} bytes", t.stdout.len());
            eprintln!("stderr: {} bytes", t.stderr.len());
        }
        None => {
            eprintln!("No trace found");
        }
    }
}

fn print_help() {
    eprintln!("trail - command output tracing");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  trail run <cmd>    Run command and capture output");
    eprintln!("  trail last         Show last stdout from current dir");
    eprintln!("  trail last -g      Show last stdout globally");
    eprintln!("  trail err          Show last stderr from current dir");
    eprintln!("  trail err -g       Show last stderr globally");
    eprintln!("  trail info         Show info about last trace");
    eprintln!("  trail init <shell> Print shell integration");
}

fn print_shell_init(shell: &str) {
    match shell {
        "fish" => {
            // Fish integration - wrap command execution
            print!(
                r#"# Trail shell integration for fish
# Add to ~/.config/fish/config.fish

function trail_run --wraps=trail
    command trail run $argv
end

# Optional: alias common commands to auto-trace
# alias make="trail run make"
# alias cargo="trail run cargo"
# alias npm="trail run npm"
"#
            );
        }
        "zsh" | "bash" => {
            print!(
                r#"# Trail shell integration for {shell}
# Add to ~/.{shell}rc

trail_run() {{
    command trail run "$@"
}}

# Optional: alias common commands to auto-trace
# alias make="trail run make"
# alias cargo="trail run cargo"
# alias npm="trail run npm"
"#,
                shell = shell
            );
        }
        _ => {
            eprintln!("Unsupported shell: {}", shell);
            eprintln!("Supported: fish, zsh, bash");
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_help();
        return;
    }

    match args[0].as_str() {
        "-h" | "--help" | "help" => print_help(),
        "run" => {
            let exit_code = run_command(&args[1..]);
            std::process::exit(exit_code);
        }
        "last" => {
            let global = args.get(1).map(|s| s == "-g" || s == "--global").unwrap_or(false);
            show_last(global, false);
        }
        "err" => {
            let global = args.get(1).map(|s| s == "-g" || s == "--global").unwrap_or(false);
            show_last(global, true);
        }
        "info" => {
            let global = args.get(1).map(|s| s == "-g" || s == "--global").unwrap_or(false);
            show_info(global);
        }
        "init" => {
            if let Some(shell) = args.get(1) {
                print_shell_init(shell);
            } else {
                eprintln!("Usage: trail init <shell>");
                eprintln!("Supported: fish, zsh, bash");
            }
        }
        _ => {
            eprintln!("Unknown command: {}", args[0]);
            print_help();
        }
    }
}
