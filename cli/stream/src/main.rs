mod config;
mod local;
mod remote;
mod session;
mod util;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use directories::ProjectDirs;

use config::{load_from, write_default_config};
use local::{build_command, spawn_local};
use remote::{RemoteHandle, build_start, build_status, build_stop, run_script};
use session::{SessionState, clear_session, load_session, write_session};
use util::{pid_alive, send_signal};

#[derive(Parser)]
#[command(
    name = "stream",
    version,
    about = "Headless macOS→Linux streaming helper"
)]
struct Cli {
    /// Path to the config file (defaults to ~/Library/Application Support/stream/config.toml on macOS).
    #[arg(long)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start streaming using the configured profile.
    Start(StartArgs),
    /// Stop local and remote streaming processes.
    Stop,
    /// Show current streaming status.
    Status(StatusArgs),
    /// Run as daemon with auto-restart on failure.
    Daemon(DaemonArgs),
    /// Manage the configuration file.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Validate dependencies and connectivity.
    Check(CheckArgs),
}

#[derive(Args)]
struct StartArgs {
    /// Profile to use (defaults to `default_profile` from the config).
    #[arg(long)]
    profile: Option<String>,
    /// Do not touch the remote receiver (assumes it's already running).
    #[arg(long)]
    skip_remote: bool,
    /// Print commands without executing them.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct StatusArgs {
    /// Check remote tmux session status via SSH.
    #[arg(long)]
    remote: bool,
}

#[derive(Args)]
struct CheckArgs {
    /// Profile to verify (defaults to `default_profile`).
    #[arg(long)]
    profile: Option<String>,
}

#[derive(Args)]
struct DaemonArgs {
    /// Profile to use (defaults to `default_profile` from the config).
    #[arg(long)]
    profile: Option<String>,
    /// Restart delay in seconds after failure.
    #[arg(long, default_value = "5")]
    restart_delay: u64,
    /// Maximum restart attempts (0 = unlimited).
    #[arg(long, default_value = "0")]
    max_restarts: u32,
    /// Do not touch the remote receiver.
    #[arg(long)]
    skip_remote: bool,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Write an example config to disk.
    Init {
        /// Overwrite the file if it already exists.
        #[arg(long)]
        force: bool,
    },
    /// Print the resolved config path.
    Path,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let dirs =
        ProjectDirs::from("dev", "nikiv", "stream").context("determine project directories")?;
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(|| dirs.config_dir().join("config.toml"));
    match cli.command {
        Commands::Start(args) => handle_start(args, &config_path, &dirs),
        Commands::Stop => handle_stop(&dirs),
        Commands::Status(args) => handle_status(args, &dirs, &config_path),
        Commands::Daemon(args) => handle_daemon(args, &config_path, &dirs),
        Commands::Config { command } => handle_config(command, &config_path),
        Commands::Check(args) => handle_check(args, &config_path),
    }
}

fn handle_config(cmd: ConfigCommand, config_path: &Path) -> Result<()> {
    match cmd {
        ConfigCommand::Init { force } => {
            if config_path.exists() && !force {
                bail!(
                    "{} already exists (pass --force to overwrite)",
                    config_path.display()
                );
            }
            if config_path.exists() && force {
                std::fs::remove_file(config_path)
                    .with_context(|| format!("remove {}", config_path.display()))?;
            }
            write_default_config(config_path)?;
            println!("Wrote {}", config_path.display());
        }
        ConfigCommand::Path => {
            println!("{}", config_path.display());
        }
    }
    Ok(())
}

fn handle_start(args: StartArgs, config_path: &Path, dirs: &ProjectDirs) -> Result<()> {
    if !config_path.exists() {
        bail!(
            "config {} not found; run `stream config init` first",
            config_path.display()
        );
    }

    let config = load_from(config_path)?;
    let (profile_name, profile) = config.profile(args.profile.as_deref())?;
    let state_path = session_file(dirs);
    if let Some(existing) = load_session(&state_path)? {
        if existing.local_running() {
            bail!(
                "found active session (PID {}) - run `stream stop` first",
                existing.local_pid
            );
        } else {
            clear_session(&state_path)?;
        }
    }

    let (remote_handle, remote_script) = if args.skip_remote {
        (None, None)
    } else {
        let (handle, script) = build_start(&profile.remote)?;
        (Some(handle), Some(script))
    };

    let spec = build_command(&profile.local, &profile.remote)?;

    if args.dry_run {
        if let Some(script) = &remote_script {
            println!("--- remote script preview ---\n{}\n", script.script);
        }
        println!("local command:\n{}\n", spec.preview);
        return Ok(());
    }

    if let (Some(handle), Some(script)) = (remote_handle.as_ref(), &remote_script) {
        println!(
            "Starting remote session {} on {}",
            handle.tmux_session, handle.host
        );
        run_script(handle, &script.script)?;
    }

    let log_dir = dirs.data_dir().join("logs");
    let launch = spawn_local(&spec, &log_dir)?;
    println!(
        "Started ffmpeg (PID {}) - logs at {}",
        launch.pid,
        launch.log_path.display()
    );

    let state = SessionState {
        profile: profile_name.clone(),
        started_at: Utc::now(),
        local_pid: launch.pid,
        log_path: launch.log_path,
        remote: remote_handle,
    };
    write_session(&state_path, &state)?;
    println!("Streaming is live via profile \"{}\"", profile_name);
    Ok(())
}

fn handle_stop(dirs: &ProjectDirs) -> Result<()> {
    let state_path = session_file(dirs);
    let Some(state) = load_session(&state_path)? else {
        println!("No active stream");
        return Ok(());
    };

    if state.local_running() {
        println!("Stopping local ffmpeg (PID {})", state.local_pid);
        send_signal(state.local_pid, libc::SIGTERM)?;
        wait_for_exit(state.local_pid, Duration::from_secs(3));
        if pid_alive(state.local_pid) {
            println!("Force killing PID {}", state.local_pid);
            send_signal(state.local_pid, libc::SIGKILL)?;
        }
    } else {
        println!("Local ffmpeg PID {} is no longer running", state.local_pid);
    }

    if let Some(remote) = &state.remote {
        println!(
            "Stopping remote session {} on {}",
            remote.tmux_session, remote.host
        );
        let script = build_stop(remote);
        if let Err(err) = run_script(remote, &script) {
            eprintln!("Warning: unable to stop remote session: {err}");
        }
    }

    clear_session(&state_path)?;
    println!("Stopped streaming session");
    Ok(())
}

fn handle_status(args: StatusArgs, dirs: &ProjectDirs, config_path: &Path) -> Result<()> {
    let state_path = session_file(dirs);
    let Some(state) = load_session(&state_path)? else {
        println!("No active stream");
        return Ok(());
    };

    println!("Profile: {}", state.profile);
    println!(
        "Local PID: {} ({})",
        state.local_pid,
        if state.local_running() {
            "running"
        } else {
            "stopped"
        }
    );
    println!("Log file: {}", state.log_path.display());

    if let Some(remote) = &state.remote {
        println!("Remote: {} session {}", remote.host, remote.tmux_session);
        if args.remote {
            let script = build_status(remote);
            match run_script(remote, &script) {
                Ok(()) => println!("Remote tmux session is running"),
                Err(err) => println!("Remote session check failed: {err}"),
            }
        }
    } else {
        println!("Remote: skipped for this session");
    }

    if !config_path.exists() {
        println!("Config file missing at {}", config_path.display());
    }

    Ok(())
}

fn handle_check(args: CheckArgs, config_path: &Path) -> Result<()> {
    if !config_path.exists() {
        bail!(
            "config {} not found; run `stream config init`",
            config_path.display()
        );
    }
    let config = load_from(config_path)?;
    let (profile_name, profile) = config.profile(args.profile.as_deref())?;
    println!("Validating profile \"{profile_name}\"");

    let local_spec = build_command(&profile.local, &profile.remote)?;
    println!("✔ ffmpeg resolved at {}", local_spec.program.display());

    which::which("ssh").context("ssh must be available in PATH")?;
    println!("✔ ssh found");

    let handle = RemoteHandle {
        host: profile.remote.host.clone(),
        user: profile.remote.user.clone(),
        port: profile.remote.port,
        tmux_session: profile.remote.tmux_session.clone(),
    };
    let status_script = r#"#!/usr/bin/env bash
set -euo pipefail
tmux -V >/dev/null 2>&1
"#;
    match run_script(&handle, status_script) {
        Ok(_) => println!("✔ remote tmux reachable"),
        Err(err) => println!("⚠ remote check failed: {err}"),
    }
    Ok(())
}

fn wait_for_exit(pid: u32, timeout: Duration) {
    let start = Instant::now();
    while pid_alive(pid) && start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn handle_daemon(args: DaemonArgs, config_path: &Path, dirs: &ProjectDirs) -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    if !config_path.exists() {
        bail!(
            "config {} not found; run `stream config init` first",
            config_path.display()
        );
    }

    let config = load_from(config_path)?;
    let (profile_name, profile) = config.profile(args.profile.as_deref())?;
    let state_path = session_file(dirs);
    let log_dir = dirs.data_dir().join("logs");

    // Setup signal handling for graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc_handler(move || {
        eprintln!("\nReceived shutdown signal, stopping...");
        r.store(false, Ordering::SeqCst);
    });

    // Start remote once if needed
    let remote_handle = if args.skip_remote {
        None
    } else {
        let (handle, script) = build_start(&profile.remote)?;
        eprintln!(
            "Starting remote session {} on {}",
            handle.tmux_session, handle.host
        );
        run_script(&handle, &script.script)?;
        Some(handle)
    };

    let mut restart_count = 0u32;

    eprintln!(
        "Daemon started for profile \"{}\" (Ctrl+C to stop)",
        profile_name
    );

    while running.load(Ordering::SeqCst) {
        // Check restart limit
        if args.max_restarts > 0 && restart_count >= args.max_restarts {
            eprintln!("Max restarts ({}) reached, exiting", args.max_restarts);
            break;
        }

        // Build and spawn ffmpeg
        let spec = build_command(&profile.local, &profile.remote)?;
        let launch = spawn_local(&spec, &log_dir)?;

        let state = SessionState {
            profile: profile_name.clone(),
            started_at: Utc::now(),
            local_pid: launch.pid,
            log_path: launch.log_path.clone(),
            remote: remote_handle.clone(),
        };
        write_session(&state_path, &state)?;

        if restart_count > 0 {
            eprintln!(
                "Restarted ffmpeg (PID {}, attempt {})",
                launch.pid, restart_count
            );
        } else {
            eprintln!("Started ffmpeg (PID {}) - logs at {}", launch.pid, launch.log_path.display());
        }

        // Wait for process to exit or shutdown signal
        while running.load(Ordering::SeqCst) && pid_alive(launch.pid) {
            std::thread::sleep(Duration::from_millis(500));
        }

        if !running.load(Ordering::SeqCst) {
            // Graceful shutdown requested
            if pid_alive(launch.pid) {
                eprintln!("Stopping ffmpeg (PID {})", launch.pid);
                send_signal(launch.pid, libc::SIGTERM)?;
                wait_for_exit(launch.pid, Duration::from_secs(3));
                if pid_alive(launch.pid) {
                    send_signal(launch.pid, libc::SIGKILL)?;
                }
            }
            break;
        }

        // Process died unexpectedly
        restart_count += 1;
        eprintln!(
            "ffmpeg exited unexpectedly, restarting in {}s...",
            args.restart_delay
        );

        // Wait before restart (but check for shutdown signal)
        let delay_end = Instant::now() + Duration::from_secs(args.restart_delay);
        while running.load(Ordering::SeqCst) && Instant::now() < delay_end {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // Cleanup
    if let Some(remote) = &remote_handle {
        eprintln!("Stopping remote session {} on {}", remote.tmux_session, remote.host);
        let script = build_stop(remote);
        let _ = run_script(remote, &script);
    }

    clear_session(&state_path)?;
    eprintln!("Daemon stopped");
    Ok(())
}

fn ctrlc_handler<F: Fn() + Send + 'static>(handler: F) {
    // Simple signal handler using libc
    unsafe {
        libc::signal(libc::SIGINT, handle_signal as usize);
        libc::signal(libc::SIGTERM, handle_signal as usize);
    }
    // Store handler in static (simplified - in real code use proper sync)
    *SIGNAL_HANDLER.lock().unwrap() = Some(Box::new(handler));
}

static SIGNAL_HANDLER: std::sync::Mutex<Option<Box<dyn Fn() + Send>>> =
    std::sync::Mutex::new(None);

extern "C" fn handle_signal(_: i32) {
    if let Ok(guard) = SIGNAL_HANDLER.lock() {
        if let Some(handler) = guard.as_ref() {
            handler();
        }
    }
}

fn session_file(dirs: &ProjectDirs) -> PathBuf {
    if let Some(state) = dirs.state_dir() {
        state.join("session.json")
    } else {
        dirs.data_dir().join("session.json")
    }
}
