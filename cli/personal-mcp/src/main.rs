use std::process::Command;

use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::*,
    schemars, tool, tool_router, tool_handler,
    ServiceExt,
};
use tracing_subscriber::EnvFilter;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct KmBindArgs {
    /// Name for the binding (e.g., "zed: codex")
    pub name: String,
    /// App to open (e.g., "Zed")
    pub app: String,
    /// Path to open (e.g., "~/fork-i/openai/codex")
    pub path: String,
}

#[derive(Clone)]
pub struct PersonalMcp {
    #[allow(dead_code)]
    tool_router: rmcp::handler::server::router::tool::ToolRouter<PersonalMcp>,
}

#[tool_router]
impl PersonalMcp {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Create a Keyboard Maestro binding to open an app at a specific path")]
    fn km_create_open(
        &self,
        Parameters(args): Parameters<KmBindArgs>,
    ) -> Result<CallToolResult, McpError> {
        let path = expand_tilde(&args.path);

        let output = Command::new("km")
            .args(["create-open", &args.name, &args.app, &path])
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "Created KM binding '{}' to open {} at {}\n{}",
                        args.name, args.app, path, stdout
                    ))]))
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "km command failed: {}",
                        stderr
                    ))]))
                }
            }
            Err(e) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Failed to run km: {}",
                e
            ))])),
        }
    }
}

#[tool_handler]
impl ServerHandler for PersonalMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: None,
        }
    }
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let io = (tokio::io::stdin(), tokio::io::stdout());
    let service = PersonalMcp::new().serve(io).await?;
    service.waiting().await?;
    Ok(())
}
