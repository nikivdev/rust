use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub fn load_from(path: &Path) -> Result<Config> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    let cfg: Config =
        toml::from_str(&raw).with_context(|| format!("parse config {}", path.display()))?;
    Ok(cfg)
}

pub fn write_default_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    if path.exists() {
        anyhow::bail!("config {} already exists", path.display());
    }

    let example = include_str!("../config.example.toml");
    fs::write(path, example).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_profile_name")]
    pub default_profile: String,
    pub profiles: BTreeMap<String, Profile>,
}

impl Config {
    pub fn profile(&self, name: Option<&str>) -> Result<(String, &Profile)> {
        let resolved = name.unwrap_or(&self.default_profile);
        let profile = self
            .profiles
            .get(resolved)
            .with_context(|| format!("profile \"{resolved}\" not found in config"))?;
        Ok((resolved.to_string(), profile))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub description: Option<String>,
    pub remote: RemoteConfig,
    pub local: LocalConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    #[serde(default = "default_tmux_session")]
    pub tmux_session: String,
    #[serde(default = "default_ingest_port")]
    pub ingest_port: u16,
    #[serde(default = "default_remote_packet_size")]
    pub packet_size: u32,
    #[serde(default)]
    pub log_path: Option<PathBuf>,
    #[serde(default = "default_remote_runner")]
    pub runner: RemoteRunner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub ffmpeg_path: PathBuf,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default = "default_video_bitrate")]
    pub video_bitrate: String,
    #[serde(default)]
    pub maxrate: Option<String>,
    #[serde(default)]
    pub bufsize: Option<String>,
    #[serde(default = "default_audio_bitrate")]
    pub audio_bitrate: String,
    #[serde(default)]
    pub scale_filter: Option<String>,
    #[serde(default)]
    pub filters: Vec<String>,
    pub capture: CaptureSource,
    pub encoder: Encoder,
    #[serde(default)]
    pub transport: Option<Transport>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Process priority: -20 (highest) to 19 (lowest). Default: 10 (low priority).
    #[serde(default = "default_nice")]
    pub nice: i32,
    /// Use realtime scheduling (requires root or proper entitlements).
    #[serde(default)]
    pub realtime: bool,
    /// Probing size in bytes (lower = faster start, less accurate detection).
    #[serde(default = "default_probesize")]
    pub probesize: u32,
    /// Analysis duration in microseconds.
    #[serde(default = "default_analyzeduration")]
    pub analyzeduration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureSource {
    Avfoundation(AvfoundationCapture),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvfoundationCapture {
    pub video_device: String,
    pub audio_device: Option<String>,
    #[serde(default = "default_pixel_format")]
    pub pixel_format: String,
    #[serde(default = "default_capture_cursor")]
    pub capture_cursor: bool,
    #[serde(default)]
    pub capture_clicks: bool,
    #[serde(default)]
    pub thread_queue_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Encoder {
    H264VideoToolbox {
        quality: Option<String>,
        #[serde(default = "default_allow_sw")]
        allow_sw: bool,
    },
    HevcVideoToolbox {
        quality: Option<String>,
        #[serde(default = "default_allow_sw")]
        allow_sw: bool,
    },
    Libx264 {
        preset: String,
        #[serde(default)]
        tune: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    #[serde(rename = "srt")]
    Srt(SrtConfig),
    #[serde(rename = "custom")]
    Custom { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrtConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    #[serde(default = "default_srt_mode")]
    pub mode: String,
    #[serde(default = "default_srt_latency")]
    pub latency_ms: u32,
    #[serde(default = "default_packet_size")]
    pub packet_size: u32,
    pub passphrase: Option<String>,
    pub pbkeylen: Option<u32>,
    pub stream_id: Option<String>,
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteRunner {
    #[serde(rename = "ffmpeg")]
    Ffmpeg(RemoteFfmpeg),
    #[serde(rename = "headless_obs")]
    HeadlessObs(RemoteObs),
    Custom {
        command: String,
    },
}

impl Default for RemoteRunner {
    fn default() -> Self {
        default_remote_runner()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFfmpeg {
    #[serde(default)]
    pub ffmpeg_path: Option<PathBuf>,
    #[serde(default = "default_remote_format")]
    pub format: String,
    pub output: String,
    #[serde(default = "default_copy")]
    pub copy_video: bool,
    #[serde(default = "default_copy")]
    pub copy_audio: bool,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteObs {
    pub binary: PathBuf,
    pub profile: String,
    pub scene_collection: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub xvfb: bool,
}

fn default_profile_name() -> String {
    "main".to_string()
}

fn default_tmux_session() -> String {
    "streamd".to_string()
}

fn default_ingest_port() -> u16 {
    6000
}

fn default_remote_packet_size() -> u32 {
    1316
}

fn default_remote_runner() -> RemoteRunner {
    RemoteRunner::Ffmpeg(RemoteFfmpeg {
        ffmpeg_path: None,
        format: "mpegts".to_string(),
        output: "~/stream/current.ts".to_string(),
        copy_video: true,
        copy_audio: true,
        extra_args: Vec::new(),
    })
}

fn default_fps() -> u32 {
    60
}

fn default_video_bitrate() -> String {
    "9000k".to_string()
}

fn default_audio_bitrate() -> String {
    "160k".to_string()
}

fn default_pixel_format() -> String {
    "uyvy422".to_string()
}

fn default_capture_cursor() -> bool {
    true
}

fn default_allow_sw() -> bool {
    false
}

fn default_srt_mode() -> String {
    "caller".to_string()
}

fn default_srt_latency() -> u32 {
    20
}

fn default_packet_size() -> u32 {
    1316
}

fn default_remote_format() -> String {
    "mpegts".to_string()
}

fn default_copy() -> bool {
    true
}

fn default_nice() -> i32 {
    10 // Low priority by default to minimize system impact
}

fn default_probesize() -> u32 {
    32 // Very small probe for instant start
}

fn default_analyzeduration() -> u32 {
    0 // Skip analysis for known input
}
