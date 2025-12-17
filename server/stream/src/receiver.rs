use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};
use tracing::info;

pub struct ReceiverHandle {
    child: Child,
}

impl ReceiverHandle {
    pub async fn stop(mut self) {
        // Send SIGTERM
        if let Err(e) = self.child.kill().await {
            tracing::warn!("Failed to kill ffmpeg: {e}");
        }
    }
}

/// Start ffmpeg to receive SRT stream and segment into files.
///
/// Output files: segment_dir/stream-YYYYMMDD-HHMMSS-NNN.ts
pub async fn start(
    ffmpeg_path: &Path,
    srt_port: u16,
    segment_dir: &Path,
    segment_duration: u32,
) -> Result<ReceiverHandle> {
    let srt_url = format!("srt://0.0.0.0:{srt_port}?mode=listener");
    let output_pattern = segment_dir.join("stream-%Y%m%d-%H%M%S-%%03d.ts");

    // ffmpeg command:
    // - Listen for SRT input
    // - Copy video and audio (no re-encoding)
    // - Segment into files with timestamps
    let mut cmd = Command::new(ffmpeg_path);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "warning",
        // Input
        "-i",
        &srt_url,
        // Copy codecs (no re-encoding)
        "-c",
        "copy",
        // Segment muxer
        "-f",
        "segment",
        "-segment_time",
        &segment_duration.to_string(),
        "-segment_format",
        "mpegts",
        // Use strftime for timestamp in filename
        "-strftime",
        "1",
        // Reset timestamps for each segment
        "-reset_timestamps",
        "1",
        // Output pattern
        output_pattern.to_str().unwrap(),
    ]);

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    info!("Starting ffmpeg receiver: {:?}", cmd);

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn ffmpeg at {}", ffmpeg_path.display()))?;

    Ok(ReceiverHandle { child })
}
