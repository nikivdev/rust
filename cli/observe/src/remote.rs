use std::borrow::Cow;
use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use shell_escape::unix::escape;

use crate::config::{RemoteConfig, RemoteFfmpeg, RemoteObs, RemoteRunner};
use crate::util::{join_shell_words, render_remote_target};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteHandle {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub tmux_session: String,
}

pub struct RemoteScript {
    pub script: String,
}

pub fn build_start(remote: &RemoteConfig) -> Result<(RemoteHandle, RemoteScript)> {
    let runner = render_runner(remote)?;
    let handle = RemoteHandle {
        host: remote.host.clone(),
        user: remote.user.clone(),
        port: remote.port,
        tmux_session: remote.tmux_session.clone(),
    };
    let cache_dir = r#"${XDG_CACHE_HOME:-$HOME/.cache}/stream"#;
    let session = escape(Cow::Borrowed(&remote.tmux_session));
    let script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

session={session}
cache_dir={cache_dir}
mkdir -p "$cache_dir"
script_path="$cache_dir/${{session}}.sh"

if tmux has-session -t "$session" >/dev/null 2>&1; then
  exit 0
fi

cat <<'STREAMSCRIPT' >"$script_path"
#!/usr/bin/env bash
set -euo pipefail
{inner}
STREAMSCRIPT
chmod +x "$script_path"

tmux new-session -d -s "$session" "$script_path"
"#,
        session = session,
        cache_dir = cache_dir,
        inner = runner.inner_script(remote)
    );

    Ok((handle, RemoteScript { script }))
}

pub fn build_stop(handle: &RemoteHandle) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
session={}
if tmux has-session -t "$session" >/dev/null 2>&1; then
  tmux send-keys -t "$session" C-c >/dev/null 2>&1 || true
  sleep 0.5
  tmux kill-session -t "$session" >/dev/null 2>&1 || true
fi
"#,
        escape(Cow::Borrowed(&handle.tmux_session))
    )
}

pub fn build_status(handle: &RemoteHandle) -> String {
    format!(
        r#"#!/usr/bin/env bash
session={}
if tmux has-session -t "$session" >/dev/null 2>&1; then
  exit 0
fi
exit 1
"#,
        escape(Cow::Borrowed(&handle.tmux_session))
    )
}

pub fn run_script(handle: &RemoteHandle, script: &str) -> Result<()> {
    let mut cmd = Command::new("ssh");
    if let Some(port) = handle.port {
        cmd.arg("-p").arg(port.to_string());
    }
    let dest = render_remote_target(handle.user.as_deref(), &handle.host);
    cmd.arg(dest);
    cmd.arg("bash");
    cmd.arg("-s");
    cmd.arg("--");
    let mut child = cmd
        .stdin(Stdio::piped())
        .spawn()
        .context("spawn ssh for remote script")?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(script.as_bytes())
            .context("write remote script")?;
    }
    let status = child.wait().context("wait for ssh")?;
    if !status.success() {
        bail!("remote script exited with {}", status);
    }
    Ok(())
}

struct RunnerScript {
    body: String,
    env_lines: Vec<String>,
}

impl RunnerScript {
    fn inner_script(&self, remote: &RemoteConfig) -> String {
        let mut lines = Vec::new();
        lines.extend(self.env_lines.iter().cloned());
        if let Some(log_path) = &remote.log_path {
            lines.push(format!(
                "exec >>{} 2>&1",
                escape(Cow::Borrowed(log_path.to_string_lossy().as_ref()))
            ));
        }
        lines.push(format!("exec {}", self.body));
        lines.join("\n")
    }
}

fn render_runner(remote: &RemoteConfig) -> Result<RunnerScript> {
    match &remote.runner {
        RemoteRunner::Ffmpeg(ffmpeg) => Ok(render_ffmpeg(remote, ffmpeg)),
        RemoteRunner::HeadlessObs(obs) => Ok(render_obs(remote, obs)),
        RemoteRunner::Custom { command } => Ok(RunnerScript {
            body: command.clone(),
            env_lines: Vec::new(),
        }),
    }
}

fn render_ffmpeg(remote: &RemoteConfig, ffmpeg: &RemoteFfmpeg) -> RunnerScript {
    let mut words = Vec::new();
    let bin = ffmpeg
        .ffmpeg_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "ffmpeg".to_string());
    words.push(bin);
    words.push("-hide_banner".into());
    words.push("-loglevel".into());
    words.push("warning".into());
    words.push("-fflags".into());
    words.push("nobuffer".into());
    words.push("-flags".into());
    words.push("low_delay".into());
    words.push("-i".into());
    words.push(format!(
        "srt://:{port}?mode=listener&latency=20&pkt_size={pkt}",
        port = remote.ingest_port,
        pkt = remote.packet_size
    ));
    if ffmpeg.copy_video {
        words.push("-c:v".into());
        words.push("copy".into());
    }
    if ffmpeg.copy_audio {
        words.push("-c:a".into());
        words.push("copy".into());
    }
    words.extend(ffmpeg.extra_args.clone());
    words.push("-f".into());
    words.push(ffmpeg.format.clone());
    words.push(ffmpeg.output.clone());

    RunnerScript {
        body: join_shell_words(&words),
        env_lines: Vec::new(),
    }
}

fn render_obs(_remote: &RemoteConfig, obs: &RemoteObs) -> RunnerScript {
    let mut words = Vec::new();
    if obs.xvfb {
        words.push("xvfb-run".into());
        words.push("--auto-servernum".into());
        words.push("--server-num".into());
        words.push("99".into());
    }
    words.push(obs.binary.display().to_string());
    words.push("--headless".into());
    words.push("--minimize-to-tray".into());
    words.push("--profile".into());
    words.push(obs.profile.clone());
    words.push("--collection".into());
    words.push(obs.scene_collection.clone());
    words.push("--startstreaming".into());
    words.extend(obs.extra_args.clone());

    let env_lines = obs
        .env
        .iter()
        .map(|(key, value)| {
            format!(
                "export {key}={value}",
                key = key,
                value = escape(Cow::Borrowed(value))
            )
        })
        .collect();

    RunnerScript {
        body: join_shell_words(&words),
        env_lines,
    }
}
