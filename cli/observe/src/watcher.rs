use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

/// A file modification event
#[derive(Debug, Clone)]
pub struct FileEvent {
    pub path: String,
}

/// Watches directories for file changes
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<FileEvent>,
    recent: HashSet<String>,
    last_cleanup: Instant,
}

impl FileWatcher {
    pub fn new() -> Result<Self> {
        let (tx, rx): (Sender<FileEvent>, Receiver<FileEvent>) = channel();

        let watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only care about modifications and creates
                    if event.kind.is_modify() || event.kind.is_create() {
                        for path in event.paths {
                            // Skip hidden files and common noise
                            if should_ignore(&path) {
                                continue;
                            }

                            if let Some(path_str) = path.to_str() {
                                let _ = tx.send(FileEvent {
                                    path: path_str.to_string(),
                                });
                            }
                        }
                    }
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        )
        .context("create file watcher")?;

        Ok(Self {
            _watcher: watcher,
            rx,
            recent: HashSet::new(),
            last_cleanup: Instant::now(),
        })
    }

    pub fn watch(&mut self, path: &Path) -> Result<()> {
        self._watcher
            .watch(path, RecursiveMode::Recursive)
            .with_context(|| format!("watch {}", path.display()))?;
        Ok(())
    }

    pub fn poll(&mut self) -> Vec<FileEvent> {
        // Cleanup old entries every 5 seconds
        if self.last_cleanup.elapsed() > Duration::from_secs(5) {
            self.recent.clear();
            self.last_cleanup = Instant::now();
        }

        let mut events = Vec::new();

        // Drain all pending events
        while let Ok(event) = self.rx.try_recv() {
            // Deduplicate within window
            if !self.recent.contains(&event.path) {
                self.recent.insert(event.path.clone());
                events.push(event);
            }
        }

        events
    }
}

/// Paths to ignore
fn should_ignore(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    // Hidden files/dirs
    if path_str.contains("/.") {
        return true;
    }

    // Common noise patterns
    let noise = [
        "/target/",
        "/node_modules/",
        "/.git/",
        "/__pycache__/",
        "/.venv/",
        "/venv/",
        "/.cache/",
        "/dist/",
        "/build/",
        ".pyc",
        ".pyo",
        ".swp",
        ".swo",
        "~",
        ".DS_Store",
        ".lock",
        "Cargo.lock",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
    ];

    for pattern in noise {
        if path_str.contains(pattern) || path_str.ends_with(pattern) {
            return true;
        }
    }

    // Only track files, not directories
    if path.is_dir() {
        return true;
    }

    false
}
