//! Automatic ingestion: while an agent session is open, new and changed recordings
//! are extracted into the store in the background.
//!
//! Every minute (and whenever asked), the roots are walked. Each recording waiting for
//! extraction is handed to a child process (`nice -n 10 wisebucket ingest <root:path>`),
//! one at a time and newest first: a decoder crash stays in the child, and the child's
//! lock keeps two sessions from extracting the same file.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use wb_core::config::{self, Overrides};
use wb_data::store::{self, Entry, Store};

/// How often the roots are walked for new recordings.
const DISCOVERY_EVERY: Duration = Duration::from_secs(60);
/// Longest a root walk may take.
const WALK_LIMIT: Duration = Duration::from_secs(30);
/// Files changed more recently than this may still be being written: they wait.
const SETTLE: Duration = Duration::from_secs(5);
/// Longest one extraction may run. ponytail: a fixed bound; make it depend on the
/// file size if multi-hour extractions of huge bags turn out to be legitimate.
const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(3600);

/// The background ingestion of one session.
pub struct Worker {
    wake: Arc<Notify>,
    task: JoinHandle<()>,
}

impl Worker {
    pub fn spawn(overrides: Overrides) -> Self {
        let wake = Arc::new(Notify::new());
        let task = tokio::spawn(run(overrides, wake.clone()));
        Self { wake, task }
    }

    /// Looks for new recordings now instead of at the next tick.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Stops the worker; a running extraction is killed (its lock dies with it).
    pub fn stop(&self) {
        self.task.abort();
    }
}

async fn run(overrides: Overrides, wake: Arc<Notify>) {
    loop {
        if let Ok(cfg) = config::load(&overrides) {
            let walk_cfg = cfg.clone();
            match tokio::task::spawn_blocking(move || store::discover(&walk_cfg, WALK_LIMIT)).await
            {
                Ok(Err(e)) => tracing::warn!(error = %e, "discovery failed"),
                Err(e) => tracing::warn!(error = %e, "discovery panicked"),
                Ok(Ok(_)) => {}
            }
            let store = Store::for_config(&cfg);
            let mut tried = std::collections::HashSet::new();
            loop {
                if store.paused() {
                    tracing::warn!("extraction paused: little free disk space");
                    break;
                }
                let Some(next) = store
                    .pending(SETTLE)
                    .into_iter()
                    .find(|e| !tried.contains(&store::recording_id(&e.candidate)))
                else {
                    break;
                };
                tried.insert(store::recording_id(&next.candidate));
                extract(&overrides, &store, &next).await;
            }
        }
        tokio::select! {
            _ = wake.notified() => {}
            _ = tokio::time::sleep(DISCOVERY_EVERY) => {}
        }
    }
}

/// The global flags a child needs to see the same configuration and state.
fn child_args(overrides: &Overrides) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(c) = &overrides.config_path {
        args.extend(["--config".to_string(), c.display().to_string()]);
    }
    if let Some(s) = &overrides.state_dir {
        args.extend(["--state-dir".to_string(), s.display().to_string()]);
    }
    args
}

async fn extract(overrides: &Overrides, store: &Store, entry: &Entry) {
    let reference = store::recording_id(&entry.candidate);
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut args: Vec<String> = child_args(overrides);
    args.extend(["ingest".to_string(), reference.clone()]);
    // Low priority, so the agent's own work stays responsive.
    let nice = PathBuf::from("/usr/bin/nice");
    let mut cmd = if nice.is_file() {
        let mut c = tokio::process::Command::new(nice);
        c.args(["-n", "10"]).arg(&exe);
        c
    } else {
        tokio::process::Command::new(&exe)
    };
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    tracing::info!(recording = %reference, "extracting");
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "cannot start extraction");
            return;
        }
    };
    match tokio::time::timeout(EXTRACTION_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) if out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let outcome = stderr
                .trim()
                .lines()
                .last()
                .unwrap_or("")
                .trim_start_matches(['✔', ' ']);
            tracing::info!(recording = %reference, %outcome, "extraction finished");
        }
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let message = stderr.trim().lines().rev().take(20).collect::<Vec<_>>();
            let message = message.into_iter().rev().collect::<Vec<_>>().join("\n");
            tracing::warn!(recording = %reference, %message, "extraction failed");
            // The child records ordinary failures itself; this covers crashes.
            let _ = store.record_failure(
                &entry.candidate,
                &format!("extraction stopped ({}): {message}", out.status),
            );
        }
        Ok(Err(e)) => tracing::warn!(error = %e, "extraction process failed"),
        Err(_) => {
            tracing::warn!(recording = %reference, "extraction timed out");
            let _ = store.record_failure(
                &entry.candidate,
                &format!(
                    "extraction took longer than {} minutes and was stopped",
                    EXTRACTION_TIMEOUT.as_secs() / 60
                ),
            );
        }
    }
}
