use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use chrono::{Local, Utc};
use clap::Parser;
use regex::Regex;
use serde::{Deserialize, Serialize};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Daemon) => run_daemon(),
        Some(Commands::List) => list_intents(),
        Some(Commands::Trigger { name }) => trigger_intent(&name),
        Some(Commands::Propose { title, action, context }) => {
            propose_to_lin(&title, &action, context.as_deref())
        }
        Some(Commands::Context) => show_context(),
        Some(Commands::Watch) => watch_context(),
        None => run_daemon(),
    }
}

#[derive(Parser)]
#[command(name = "intent", version, about = "Context-aware intent daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Run as daemon (default)
    Daemon,
    /// List configured intents
    List,
    /// Manually trigger an intent by name
    Trigger { name: String },
    /// Propose an action to Lin (shows in notch UI)
    Propose {
        /// Title shown in Lin
        title: String,
        /// Shell command to run if accepted
        action: String,
        /// Optional context (e.g., repo path)
        #[arg(long)]
        context: Option<String>,
    },
    /// Show current context
    Context,
    /// Watch context changes in real-time
    Watch,
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    context: ContextConfig,
    #[serde(default)]
    intent: Vec<Intent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ContextConfig {
    /// Context source: "native" (AppleScript), "file" (JSON file)
    #[serde(default = "default_source")]
    source: String,
    /// Path to context file (when source = "file")
    #[serde(default = "default_context_file")]
    context_file: String,
    /// Poll interval in milliseconds
    #[serde(default = "default_poll_interval")]
    poll_interval_ms: u64,
}

fn default_source() -> String {
    "native".to_string()
}

fn default_context_file() -> String {
    "~/Library/Application Support/Lin/context.json".to_string()
}

fn default_poll_interval() -> u64 {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Intent {
    name: String,
    /// App bundle ID or name pattern (regex)
    #[serde(default)]
    app: Option<String>,
    /// Window title pattern (regex)
    #[serde(default)]
    window: Option<String>,
    /// Trigger type: "enter", "exit", "change"
    #[serde(default = "default_trigger")]
    trigger: String,
    /// Action type: "run" (execute immediately) or "propose" (send to Lin)
    #[serde(default = "default_action_type")]
    action_type: String,
    /// Shell command or proposal
    action: String,
    /// Title for proposals (used when action_type = "propose")
    #[serde(default)]
    title: Option<String>,
    /// Cooldown in seconds
    #[serde(default = "default_cooldown")]
    cooldown: u64,
}

fn default_trigger() -> String {
    "exit".to_string()
}

fn default_action_type() -> String {
    "propose".to_string()
}

fn default_cooldown() -> u64 {
    30
}

// ── Lin Proposal ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LinProposal {
    id: String,
    timestamp: i64,
    title: String,
    action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<String>,
    expires_at: i64,
}

fn lin_proposals_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/Lin/proposals.json")
}

fn propose_to_lin(title: &str, action: &str, context: Option<&str>) -> Result<()> {
    let path = lin_proposals_path();

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Load existing proposals
    let mut proposals: Vec<LinProposal> = if path.exists() {
        let content = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        vec![]
    };

    // Remove expired proposals
    let now = Utc::now().timestamp();
    proposals.retain(|p| p.expires_at > now);

    // Add new proposal
    let proposal = LinProposal {
        id: format!("{:x}", rand_id()),
        timestamp: now,
        title: title.to_string(),
        action: action.to_string(),
        context: context.map(String::from),
        expires_at: now + 300, // 5 minute expiry
    };

    eprintln!(
        "[{}] propose: {} -> {}",
        Local::now().format("%H:%M:%S"),
        proposal.title,
        proposal.action
    );
    proposals.push(proposal);

    // Write back
    let content = serde_json::to_string_pretty(&proposals)?;
    fs::write(&path, content)?;

    Ok(())
}

fn rand_id() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// ── Context ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SystemContext {
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    app_name: String,
    #[serde(default)]
    window_title: String,
    #[serde(default)]
    timestamp: u64,
}

impl SystemContext {
    /// Extract repo/project from window title (e.g., "~/lang/rust" from path)
    fn infer_project(&self) -> Option<String> {
        // Common patterns: "~/lang/rust", "/Users/nikiv/lang/rust", "lang/rust"
        let title = &self.window_title;

        // Look for path patterns
        if let Some(idx) = title.find("~/") {
            let rest = &title[idx..];
            let end = rest.find(|c: char| c == ' ' || c == ':' || c == '—').unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }

        if let Some(idx) = title.find("/Users/") {
            let rest = &title[idx..];
            let end = rest.find(|c: char| c == ' ' || c == ':' || c == '—').unwrap_or(rest.len());
            // Convert to ~/
            let path = &rest[..end];
            if let Some(home) = dirs::home_dir() {
                if let Some(suffix) = path.strip_prefix(&home.to_string_lossy().to_string()) {
                    return Some(format!("~{}", suffix));
                }
            }
            return Some(path.to_string());
        }

        None
    }

    /// Infer deploy command based on context
    fn infer_deploy_command(&self) -> Option<String> {
        let project = self.infer_project()?;

        // Map known repos to deploy commands
        let deploys = [
            ("lang/rust", "f deploy-intent"),
            ("lang/py", "f deploy"),
            ("org/linsa", "f build"),
            ("org/la", "f deploy"),
        ];

        for (pattern, cmd) in deploys {
            if project.contains(pattern) {
                return Some(cmd.to_string());
            }
        }

        // Default: try to find flow.toml and suggest generic deploy
        Some("f deploy".to_string())
    }
}

// ── Paths ─────────────────────────────────────────────────────────────────────

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("intent.toml")
}

fn expand_path(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

fn load_config() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        return Ok(Config {
            context: ContextConfig::default(),
            intent: vec![],
        });
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

fn load_context_from_file(path: &str) -> Result<SystemContext> {
    let expanded = expand_path(path);
    if !PathBuf::from(&expanded).exists() {
        return Ok(SystemContext::default());
    }
    let content = fs::read_to_string(&expanded)?;
    serde_json::from_str(&content).context("failed to parse context")
}

/// Fetch context using native macOS APIs via AppleScript
fn load_context_native() -> SystemContext {
    let script = r#"
        tell application "System Events"
            set frontApp to first application process whose frontmost is true
            set appName to name of frontApp
            set appId to bundle identifier of frontApp
            try
                set windowTitle to name of front window of frontApp
            on error
                set windowTitle to ""
            end try
        end tell
        return appId & "\n" & appName & "\n" & windowTitle
    "#;

    let output = Command::new("osascript")
        .args(["-e", script])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let lines: Vec<&str> = text.trim().split('\n').collect();
            SystemContext {
                app_id: lines.first().unwrap_or(&"").to_string(),
                app_name: lines.get(1).unwrap_or(&"").to_string(),
                window_title: lines.get(2).unwrap_or(&"").to_string(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            }
        }
        _ => SystemContext::default(),
    }
}

fn get_context(config: &ContextConfig) -> SystemContext {
    match config.source.as_str() {
        "file" => load_context_from_file(&config.context_file).unwrap_or_default(),
        "native" | _ => load_context_native(),
    }
}

// ── Intent Matching ───────────────────────────────────────────────────────────

struct IntentMatcher {
    app_regex: Option<Regex>,
    window_regex: Option<Regex>,
}

impl IntentMatcher {
    fn new(intent: &Intent) -> Result<Self> {
        let app_regex = intent
            .app
            .as_ref()
            .map(|p| Regex::new(p))
            .transpose()
            .context("invalid app pattern")?;
        let window_regex = intent
            .window
            .as_ref()
            .map(|p| Regex::new(p))
            .transpose()
            .context("invalid window pattern")?;
        Ok(Self {
            app_regex,
            window_regex,
        })
    }

    fn matches(&self, ctx: &SystemContext) -> bool {
        let app_match = self.app_regex.as_ref().map_or(true, |r| {
            r.is_match(&ctx.app_id) || r.is_match(&ctx.app_name)
        });
        let window_match = self
            .window_regex
            .as_ref()
            .map_or(true, |r| r.is_match(&ctx.window_title));
        app_match && window_match
    }
}

// ── Daemon ────────────────────────────────────────────────────────────────────

struct IntentState {
    matched_since: Option<Instant>,
    last_triggered: Option<Instant>,
    last_context: Option<SystemContext>,
}

fn run_daemon() -> Result<()> {
    eprintln!("intent: starting daemon");
    eprintln!("config: {}", config_path().display());

    let config = load_config()?;
    let poll_interval = Duration::from_millis(config.context.poll_interval_ms);

    eprintln!("source: {}", config.context.source);
    eprintln!("intents: {}", config.intent.len());
    eprintln!("proposals: {}", lin_proposals_path().display());

    if config.intent.is_empty() {
        eprintln!("no intents configured, watching context only");
    }

    // Compile matchers
    let matchers: Vec<_> = config
        .intent
        .iter()
        .map(|i| IntentMatcher::new(i))
        .collect::<Result<Vec<_>>>()?;

    // State tracking per intent
    let mut states: HashMap<String, IntentState> = config
        .intent
        .iter()
        .map(|i| {
            (
                i.name.clone(),
                IntentState {
                    matched_since: None,
                    last_triggered: None,
                    last_context: None,
                },
            )
        })
        .collect();

    let mut prev_context = SystemContext::default();

    loop {
        let ctx = get_context(&config.context);

        // Check each intent
        for (i, intent) in config.intent.iter().enumerate() {
            let matcher = &matchers[i];
            let state = states.get_mut(&intent.name).unwrap();

            let now_matches = matcher.matches(&ctx);
            let prev_matches = matcher.matches(&prev_context);

            // Update match tracking
            if now_matches {
                if state.matched_since.is_none() {
                    state.matched_since = Some(Instant::now());
                    eprintln!("[{}] match: {}", Local::now().format("%H:%M:%S"), intent.name);
                }
                state.last_context = Some(ctx.clone());
            } else if !now_matches && prev_matches {
                // Just exited - keep last_context for proposal
                eprintln!("[{}] exit: {}", Local::now().format("%H:%M:%S"), intent.name);
            }

            // Check trigger conditions
            let should_trigger = match intent.trigger.as_str() {
                "enter" => now_matches && !prev_matches,
                "exit" => !now_matches && prev_matches,
                "change" => now_matches != prev_matches,
                _ => false,
            };

            if !should_trigger {
                if !now_matches {
                    state.matched_since = None;
                }
                continue;
            }

            // Check cooldown
            if let Some(last) = state.last_triggered {
                if last.elapsed() < Duration::from_secs(intent.cooldown) {
                    state.matched_since = None;
                    continue;
                }
            }

            // Get context for this trigger (use last matched context for exit triggers)
            let trigger_ctx = if intent.trigger == "exit" {
                state.last_context.as_ref().unwrap_or(&prev_context)
            } else {
                &ctx
            };

            // Execute based on action_type
            match intent.action_type.as_str() {
                "run" => {
                    eprintln!("run: {} -> {}", intent.name, intent.action);
                    execute_action(&intent.action);
                }
                "propose" | _ => {
                    let title = intent.title.as_deref().unwrap_or(&intent.name);
                    let action = resolve_action(&intent.action, trigger_ctx);
                    let context = trigger_ctx.infer_project();
                    let _ = propose_to_lin(title, &action, context.as_deref());
                }
            }

            state.last_triggered = Some(Instant::now());
            state.matched_since = None;
            state.last_context = None;
        }

        prev_context = ctx;
        thread::sleep(poll_interval);
    }
}

/// Resolve action template with context variables
fn resolve_action(action: &str, ctx: &SystemContext) -> String {
    let mut result = action.to_string();

    // Replace {project} with inferred project path
    if let Some(project) = ctx.infer_project() {
        result = result.replace("{project}", &project);
    }

    // Replace {deploy} with inferred deploy command
    if let Some(deploy) = ctx.infer_deploy_command() {
        result = result.replace("{deploy}", &deploy);
    }

    result
}

fn execute_action(action: &str) {
    let result = Command::new("sh").args(["-c", action]).status();

    match result {
        Ok(status) => {
            if !status.success() {
                eprintln!("action failed: exit {}", status);
            }
        }
        Err(e) => {
            eprintln!("action error: {}", e);
        }
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn list_intents() -> Result<()> {
    let config = load_config()?;

    if config.intent.is_empty() {
        println!("no intents configured");
        println!("config: {}", config_path().display());
        return Ok(());
    }

    for intent in &config.intent {
        println!(
            "{}: {} [{}:{}] -> {}",
            intent.name,
            intent
                .app
                .as_deref()
                .or(intent.window.as_deref())
                .unwrap_or("*"),
            intent.action_type,
            intent.trigger,
            intent.action
        );
    }

    Ok(())
}

fn trigger_intent(name: &str) -> Result<()> {
    let config = load_config()?;

    let intent = config
        .intent
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| anyhow::anyhow!("intent not found: {}", name))?;

    eprintln!("triggering: {} -> {}", intent.name, intent.action);

    match intent.action_type.as_str() {
        "run" => execute_action(&intent.action),
        "propose" | _ => {
            let title = intent.title.as_deref().unwrap_or(&intent.name);
            propose_to_lin(title, &intent.action, None)?;
        }
    }

    Ok(())
}

fn show_context() -> Result<()> {
    let config = load_config()?;
    let ctx = get_context(&config.context);

    println!("source: {}", config.context.source);
    println!("app: {} ({})", ctx.app_name, ctx.app_id);
    println!("window: {}", ctx.window_title);

    if let Some(project) = ctx.infer_project() {
        println!("project: {}", project);
    }
    if let Some(deploy) = ctx.infer_deploy_command() {
        println!("deploy: {}", deploy);
    }

    Ok(())
}

fn watch_context() -> Result<()> {
    let config = load_config()?;
    let poll_interval = Duration::from_millis(config.context.poll_interval_ms);

    eprintln!("watching: source={}", config.context.source);

    let mut prev = SystemContext::default();

    loop {
        let ctx = get_context(&config.context);

        if ctx.app_name != prev.app_name || ctx.window_title != prev.window_title {
            let project = ctx.infer_project().unwrap_or_default();
            println!(
                "[{}] {} | {} | {}",
                Local::now().format("%H:%M:%S"),
                ctx.app_name,
                ctx.window_title,
                project
            );
        }

        prev = ctx;
        thread::sleep(poll_interval);
    }
}
