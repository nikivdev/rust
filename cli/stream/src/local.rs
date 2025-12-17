use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use chrono::Local;

use crate::config::{
    AvfoundationCapture, CaptureSource, Encoder, LocalConfig, RemoteConfig, SrtConfig, Transport,
};
use crate::util::{join_shell_words, resolve_program};

pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub preview: String,
    pub nice: i32,
    pub realtime: bool,
}

pub struct LocalLaunch {
    pub pid: u32,
    pub log_path: PathBuf,
}

pub fn build_command(local: &LocalConfig, remote: &RemoteConfig) -> Result<CommandSpec> {
    let program = resolve_program(&local.ffmpeg_path)?;
    let mut args = Vec::new();
    args.push("-hide_banner".into());
    args.push("-loglevel".into());
    args.push("warning".into());

    // Fast startup: minimal probing for known capture source
    args.push("-probesize".into());
    args.push(local.probesize.to_string());
    args.push("-analyzeduration".into());
    args.push(local.analyzeduration.to_string());

    args.extend(build_capture_args(&local.capture, local.fps));

    let mut filter_chain = Vec::new();
    if let Some(scale) = &local.scale_filter {
        filter_chain.push(scale.clone());
    } else if let Some(res) = &local.resolution {
        filter_chain.push(format!("scale={res}"));
    }
    filter_chain.extend(local.filters.clone());
    if !filter_chain.is_empty() {
        args.push("-vf".into());
        args.push(filter_chain.join(","));
    }

    args.push("-c:v".into());
    match &local.encoder {
        Encoder::H264VideoToolbox { quality, allow_sw } => {
            args.push("h264_videotoolbox".into());
            if let Some(q) = quality {
                args.push("-quality".into());
                args.push(q.clone());
            }
            args.push("-allow_sw".into());
            args.push(if *allow_sw { "1" } else { "0" }.into());
        }
        Encoder::HevcVideoToolbox { quality, allow_sw } => {
            args.push("hevc_videotoolbox".into());
            if let Some(q) = quality {
                args.push("-quality".into());
                args.push(q.clone());
            }
            args.push("-allow_sw".into());
            args.push(if *allow_sw { "1" } else { "0" }.into());
        }
        Encoder::Libx264 { preset, tune } => {
            args.push("libx264".into());
            args.push("-preset".into());
            args.push(preset.clone());
            if let Some(tune) = tune {
                args.push("-tune".into());
                args.push(tune.clone());
            }
            args.push("-pix_fmt".into());
            args.push("yuv420p".into());
        }
    }

    args.push("-b:v".into());
    args.push(local.video_bitrate.clone());
    if let Some(maxrate) = &local.maxrate {
        args.push("-maxrate".into());
        args.push(maxrate.clone());
    }
    if let Some(bufsize) = &local.bufsize {
        args.push("-bufsize".into());
        args.push(bufsize.clone());
    }

    if capture_has_audio(&local.capture) {
        args.push("-c:a".into());
        args.push("aac".into());
        args.push("-b:a".into());
        args.push(local.audio_bitrate.clone());
    } else {
        args.push("-an".into());
    }

    args.extend(local.extra_args.clone());

    args.push("-f".into());
    args.push("mpegts".into());

    let output_url = match &local.transport {
        Some(Transport::Custom { url }) => url.clone(),
        Some(Transport::Srt(config)) => config.build_url(remote),
        None => SrtConfigDefaults::build_default(remote),
    };
    args.push(output_url);

    let nice_prefix = if local.nice != 0 {
        format!("nice -n {} ", local.nice)
    } else {
        String::new()
    };
    let preview = format!("{}{} {}", nice_prefix, program.display(), join_shell_words(&args));

    Ok(CommandSpec {
        program,
        args,
        preview,
        nice: local.nice,
        realtime: local.realtime,
    })
}

pub fn spawn_local(spec: &CommandSpec, log_dir: &Path) -> Result<LocalLaunch> {
    fs::create_dir_all(log_dir).with_context(|| format!("create {}", log_dir.display()))?;
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let log_path = log_dir.join(format!("stream-{timestamp}.log"));
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open {}", log_path.display()))?;
    let stderr = stdout.try_clone().context("clone log file handle")?;

    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    // Set process priority before exec
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let nice = spec.nice;
        unsafe {
            cmd.pre_exec(move || {
                // Set nice value (lower priority = higher nice value)
                if nice != 0 {
                    libc::setpriority(libc::PRIO_PROCESS, 0, nice);
                }
                Ok(())
            });
        }
    }

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", spec.program.display()))?;

    Ok(LocalLaunch {
        pid: child.id(),
        log_path,
    })
}

fn build_capture_args(capture: &CaptureSource, fps: u32) -> Vec<String> {
    match capture {
        CaptureSource::Avfoundation(spec) => avfoundation_args(spec, fps),
    }
}

fn avfoundation_args(capture: &AvfoundationCapture, fps: u32) -> Vec<String> {
    let mut args = Vec::new();
    args.push("-thread_queue_size".into());
    args.push(
        capture
            .thread_queue_size
            .map(|v| v.to_string())
            .unwrap_or_else(|| "512".into()),
    );
    args.push("-f".into());
    args.push("avfoundation".into());
    if capture.capture_cursor {
        args.push("-capture_cursor".into());
        args.push("1".into());
    }
    if capture.capture_clicks {
        args.push("-capture_mouse_clicks".into());
        args.push("1".into());
    }
    args.push("-pixel_format".into());
    args.push(capture.pixel_format.clone());
    args.push("-framerate".into());
    args.push(fps.to_string());
    let audio = capture
        .audio_device
        .clone()
        .unwrap_or_else(|| "none".into());
    args.push("-i".into());
    args.push(format!("{}:{}", capture.video_device, audio));
    args
}

fn capture_has_audio(capture: &CaptureSource) -> bool {
    match capture {
        CaptureSource::Avfoundation(spec) => spec
            .audio_device
            .as_ref()
            .map(|dev| dev != "none")
            .unwrap_or(false),
    }
}

struct SrtConfigDefaults;

impl SrtConfigDefaults {
    fn build_default(remote: &RemoteConfig) -> String {
        format!(
            "srt://{}:{}?mode=caller&latency=20&pkt_size={}",
            remote.host, remote.ingest_port, remote.packet_size
        )
    }
}

impl SrtConfig {
    pub fn build_url(&self, remote: &RemoteConfig) -> String {
        let host = self.host.clone().unwrap_or_else(|| remote.host.clone());
        let port = self.port.unwrap_or(remote.ingest_port);
        let mut params = vec![
            format!("mode={}", self.mode),
            format!("latency={}", self.latency_ms),
            format!("pkt_size={}", self.packet_size),
        ];
        if let Some(pass) = &self.passphrase {
            params.push(format!("passphrase={pass}"));
        }
        if let Some(pbkeylen) = self.pbkeylen {
            params.push(format!("pbkeylen={pbkeylen}"));
        }
        if let Some(stream_id) = &self.stream_id {
            params.push(format!("streamid={stream_id}"));
        }
        for (key, value) in &self.extra {
            params.push(format!("{key}={value}"));
        }
        format!("srt://{host}:{port}?{}", params.join("&"))
    }
}
