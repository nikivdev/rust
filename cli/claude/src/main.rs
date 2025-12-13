use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use claude_code_sdk::{
    query, AssistantMessage, ClaudeCodeOptions, ContentBlock, Message, PermissionMode, TextBlock,
};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

const LIN_SERVER_URL: &str = "http://127.0.0.1:9050";

#[derive(Parser)]
#[command(name = "claude-rs")]
#[command(about = "CLI wrapper for Claude Code SDK")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a prompt (default command)
    #[command(name = "run", alias = "r")]
    Run {
        /// The task/prompt to send to Claude
        prompt: String,

        /// Working directory for Claude to operate in
        #[arg(short, long)]
        cwd: Option<String>,

        /// System prompt to use
        #[arg(short, long)]
        system: Option<String>,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,

        /// Maximum turns
        #[arg(long)]
        max_turns: Option<u32>,

        /// Bypass permission prompts (dangerously allow all tools)
        #[arg(long)]
        dangerously_skip_permissions: bool,

        /// Run as background job via lin server
        #[arg(short, long)]
        background: bool,
    },
    /// View logs for a background task
    Logs {
        /// Task ID to view logs for
        task_id: String,
    },
}

#[derive(Serialize)]
struct TaskRequest {
    task: TaskSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
}

#[derive(Serialize)]
struct TaskSpec {
    name: String,
    command: String,
}

#[derive(Deserialize)]
struct TaskLog {
    id: String,
    name: String,
    #[allow(dead_code)]
    command: String,
    #[allow(dead_code)]
    cwd: Option<String>,
    #[allow(dead_code)]
    started_at: u128,
    finished_at: Option<u128>,
    exit_code: Option<i32>,
    output: Vec<TaskOutputLine>,
}

#[derive(Deserialize)]
struct TaskOutputLine {
    #[allow(dead_code)]
    timestamp_ms: u128,
    #[allow(dead_code)]
    stream: String,
    line: String,
}

async fn view_logs(task_id: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/tasks/logs/{}", LIN_SERVER_URL, task_id))
        .send()
        .await
        .context("Failed to connect to lin server")?;

    if !response.status().is_success() {
        let error: serde_json::Value = response.json().await.unwrap_or_default();
        let msg = error
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("task not found");
        anyhow::bail!("{}", msg);
    }

    let log: TaskLog = response.json().await.context("Failed to parse task log")?;

    println!("Task: {} ({})", log.name, log.id);

    if let Some(code) = log.exit_code {
        let status = if code == 0 { "completed" } else { "failed" };
        println!("Status: {} (exit code {})", status, code);
    } else if log.finished_at.is_none() {
        println!("Status: running...");
    }

    println!();

    for line in &log.output {
        println!("{}", line.line);
    }

    Ok(())
}

struct RunOpts<'a> {
    prompt: &'a str,
    cwd: Option<&'a String>,
    system: Option<&'a String>,
    model: Option<&'a String>,
    max_turns: Option<u32>,
    dangerously_skip_permissions: bool,
}

async fn run_background(opts: RunOpts<'_>) -> Result<()> {
    let cwd = opts
        .cwd
        .cloned()
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()));

    // Build the claude command to run
    let mut cmd_parts = vec!["claude".to_string()];

    // Add the prompt (escaped for shell)
    cmd_parts.push(format!("\"{}\"", opts.prompt.replace('\"', "\\\"")));

    // Background jobs always skip permissions (no one to approve)
    cmd_parts.push("--dangerously-skip-permissions".to_string());
    if let Some(model) = &opts.model {
        cmd_parts.push(format!("--model {}", model));
    }
    if let Some(system) = &opts.system {
        cmd_parts.push(format!("--system-prompt \"{}\"", system.replace('\"', "\\\"")));
    }
    if let Some(max_turns) = opts.max_turns {
        cmd_parts.push(format!("--max-turns {}", max_turns));
    }

    let command = cmd_parts.join(" ");

    let task_request = TaskRequest {
        task: TaskSpec {
            name: format!("claude: {}", truncate(opts.prompt, 50)),
            command,
        },
        cwd,
    };

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/tasks/run", LIN_SERVER_URL))
        .json(&task_request)
        .send()
        .await
        .context("Failed to connect to lin server. Is it running?")?;

    if response.status().is_success() {
        let result: serde_json::Value = response.json().await?;
        if let Some(task_id) = result.get("task_id").and_then(|v| v.as_str()) {
            println!("Delegated to lin");
            println!("  View logs: claude-rs logs {}", task_id);
        } else {
            println!("Delegated to lin");
        }
    } else {
        let error: serde_json::Value = response.json().await.unwrap_or_default();
        anyhow::bail!(
            "Failed to queue task: {}",
            error.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error")
        );
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

async fn run_foreground(opts: RunOpts<'_>) -> Result<()> {
    let permission_mode = if opts.dangerously_skip_permissions {
        Some(PermissionMode::BypassPermissions)
    } else {
        None
    };

    let options = ClaudeCodeOptions {
        system_prompt: opts.system.cloned(),
        cwd: opts.cwd.map(|s| s.into()),
        model: opts.model.cloned(),
        max_turns: opts.max_turns,
        permission_mode,
        ..Default::default()
    };

    let mut stream = query(opts.prompt, Some(options)).await?;

    while let Some(message) = stream.next().await {
        match message {
            Message::Assistant(AssistantMessage { content }) => {
                for block in content {
                    if let ContentBlock::Text(TextBlock { text }) = block {
                        print!("{}", text);
                    }
                }
            }
            Message::Result(result) => {
                if result.is_error {
                    if let Some(err) = result.result {
                        eprintln!("\nError: {}", err);
                    }
                }
            }
            _ => {}
        }
    }

    println!();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Logs { task_id } => view_logs(&task_id).await,
        Command::Run {
            prompt,
            cwd,
            system,
            model,
            max_turns,
            dangerously_skip_permissions,
            background,
        } => {
            let opts = RunOpts {
                prompt: &prompt,
                cwd: cwd.as_ref(),
                system: system.as_ref(),
                model: model.as_ref(),
                max_turns,
                dangerously_skip_permissions,
            };
            if background {
                run_background(opts).await
            } else {
                run_foreground(opts).await
            }
        }
    }
}
