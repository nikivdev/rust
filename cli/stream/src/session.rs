use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::remote::RemoteHandle;
use crate::util::pid_alive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub profile: String,
    pub started_at: DateTime<Utc>,
    pub local_pid: u32,
    pub log_path: PathBuf,
    pub remote: Option<RemoteHandle>,
}

impl SessionState {
    pub fn local_running(&self) -> bool {
        pid_alive(self.local_pid)
    }
}

pub fn load_session(path: &Path) -> Result<Option<SessionState>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let session: SessionState =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(session))
}

pub fn write_session(path: &Path, state: &SessionState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(state).context("serialize session state to json")?;
    fs::write(path, raw).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn clear_session(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}
