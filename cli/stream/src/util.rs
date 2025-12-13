use std::borrow::Cow;
use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use shell_escape::unix::escape;

pub fn resolve_program(path: &Path) -> Result<PathBuf> {
    if path.components().count() == 1 && path.to_string_lossy().contains('/') == false {
        let program = path
            .to_str()
            .context("program name contains invalid UTF-8")?;
        let resolved = which::which(program).with_context(|| format!("locate binary {program}"))?;
        return Ok(resolved);
    }

    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .context("determine current directory")?
            .join(path)
    };

    if !candidate.exists() {
        bail!("binary {} not found", candidate.display());
    }

    Ok(candidate)
}

pub fn pid_alive(pid: u32) -> bool {
    let res = unsafe { libc::kill(pid as i32, 0) };
    if res == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::ESRCH) => false,
        Some(libc::EPERM) => true,
        _ => false,
    }
}

pub fn send_signal(pid: u32, signal: libc::c_int) -> Result<()> {
    let rc = unsafe { libc::kill(pid as i32, signal) };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    bail!("failed to signal PID {pid}: {err}");
}

pub fn join_shell_words(words: &[String]) -> String {
    words
        .iter()
        .map(|w| escape(Cow::Borrowed(w.as_str())).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn render_remote_target(user: Option<&str>, host: &str) -> String {
    if let Some(user) = user {
        format!("{user}@{host}")
    } else {
        host.to_string()
    }
}
