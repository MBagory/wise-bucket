//! Automatic ingestion: while an agent session is open, new and changed recordings
//! are extracted into the store in the background.
//!
//! The roots are walked at startup, when `server_info` asks, and on file events. A file
//! renamed into place (a finished download) is extracted at once; a file written in
//! place waits until it has not changed for a few seconds. Each recording is handed to
//! a child process (`nice -n 10 wisebucket ingest <root:path>`), one at a time and
//! newest first: a decoder crash stays in the child, and the child's lock keeps two
//! sessions from extracting the same file.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use wb_core::config::{self, Overrides};
use wb_core::roots;
use wb_core::roots::Candidate;
use wb_data::store::{self, Store};

/// How often the roots are walked when file events are not available.
const DISCOVERY_EVERY: Duration = Duration::from_secs(60);
/// Longest a root walk may take.
const WALK_LIMIT: Duration = Duration::from_secs(30);
/// Files written in place and changed more recently than this may still be being
/// written: they wait.
const SETTLE: Duration = Duration::from_secs(5);
/// Longest one extraction may run. ponytail: a fixed bound; make it depend on the
/// file size if multi-hour extractions of huge bags turn out to be legitimate.
const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(3600);

/// Why the worker should look at the roots.
#[derive(Debug)]
enum Trigger {
    /// `server_info` was called.
    Now,
    /// A file or folder was renamed into place: it is complete.
    Ready(PathBuf),
    /// Something else changed in a root.
    Changed,
}

/// The background ingestion of one session.
pub struct Worker {
    tx: UnboundedSender<Trigger>,
    task: JoinHandle<()>,
}

impl Worker {
    pub fn spawn(overrides: Overrides) -> Self {
        let (tx, rx) = unbounded_channel();
        let task = tokio::spawn(run(overrides, tx.clone(), rx));
        Self { tx, task }
    }

    /// Looks for new recordings now.
    pub fn wake(&self) {
        let _ = self.tx.send(Trigger::Now);
    }

    /// Stops the worker; a running extraction is killed (its lock dies with it).
    pub fn stop(&self) {
        self.task.abort();
    }
}

/// What a file event means for the worker. `ignore` holds the worker's own output
/// (store, logs): a root containing the state directory would otherwise wake itself.
fn classify(ev: &Event, ignore: &[PathBuf]) -> Option<Trigger> {
    // Reads (inotify reports opens) include the extraction's own.
    if matches!(ev.kind, EventKind::Access(_)) {
        return None;
    }
    let paths: Vec<&PathBuf> = ev
        .paths
        .iter()
        .filter(|p| !ignore.iter().any(|i| p.starts_with(i)))
        .collect();
    if paths.is_empty() && !ev.paths.is_empty() {
        return None;
    }
    // inotify gives both sides of a rename, FSEvents one event per side: the side
    // that exists is the new name.
    if matches!(ev.kind, EventKind::Modify(ModifyKind::Name(_)))
        && let Some(p) = paths.iter().find(|p| p.exists())
    {
        return Some(Trigger::Ready(p.to_path_buf()));
    }
    Some(Trigger::Changed)
}

/// Watches `roots` recursively. `None` (with a warning) when some root cannot be watched.
fn watch(
    roots: &[PathBuf],
    ignore: Vec<PathBuf>,
    tx: UnboundedSender<Trigger>,
) -> Option<RecommendedWatcher> {
    let handler = move |res: notify::Result<Event>| {
        let trigger = match res {
            Ok(ev) => classify(&ev, &ignore),
            Err(_) => Some(Trigger::Changed),
        };
        if let Some(t) = trigger {
            let _ = tx.send(t);
        }
    };
    let mut watcher = match notify::recommended_watcher(handler) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "cannot watch the data roots; walking them every minute");
            return None;
        }
    };
    for root in roots {
        if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
            tracing::warn!(root = %root.display(), error = %e, "cannot watch a data root; walking the roots every minute");
            return None;
        }
    }
    Some(watcher)
}

async fn run(
    overrides: Overrides,
    tx: UnboundedSender<Trigger>,
    mut rx: UnboundedReceiver<Trigger>,
) {
    let mut watcher: Option<RecommendedWatcher> = None;
    let mut watched: Option<(Vec<PathBuf>, PathBuf)> = None;
    let mut ready: HashSet<PathBuf> = HashSet::new();
    let mut paused = false;
    loop {
        let wait = pass(
            &overrides,
            &tx,
            &mut watcher,
            &mut watched,
            &ready,
            &mut paused,
        )
        .await;
        ready.clear();

        let mut deadline = wait.map(|d| Instant::now() + d);
        loop {
            let trigger = match deadline {
                Some(d) => tokio::select! {
                    t = rx.recv() => t,
                    _ = tokio::time::sleep_until(d) => break,
                },
                None => rx.recv().await,
            };
            match trigger {
                None => return,
                Some(Trigger::Now) => break,
                Some(Trigger::Ready(p)) => {
                    ready.insert(p);
                    break;
                }
                // Look once the change has had time to settle; later events in the
                // meantime are covered by that same pass.
                Some(Trigger::Changed) => {
                    let t = Instant::now() + SETTLE;
                    deadline = Some(deadline.map_or(t, |d| d.min(t)));
                }
            }
        }
        // A burst of events gives one pass.
        while let Ok(t) = rx.try_recv() {
            if let Trigger::Ready(p) = t {
                ready.insert(p);
            }
        }
    }
}

/// Walks the roots and extracts what is ready. Returns when to look again without an
/// event: `SETTLE`-bound for files still being written, `DISCOVERY_EVERY` when events
/// cannot be relied on. `paused` is whether the last pass found too little disk space.
async fn pass(
    overrides: &Overrides,
    tx: &UnboundedSender<Trigger>,
    watcher: &mut Option<RecommendedWatcher>,
    watched: &mut Option<(Vec<PathBuf>, PathBuf)>,
    ready: &HashSet<PathBuf>,
    paused: &mut bool,
) -> Option<Duration> {
    let Ok(cfg) = config::load(overrides) else {
        return Some(DISCOVERY_EVERY);
    };
    let root_set = roots::resolve(&cfg);
    let root_paths: HashMap<String, PathBuf> = root_set
        .roots
        .iter()
        .map(|r| (r.name.clone(), r.path.clone()))
        .collect();
    let store = Store::for_config(&cfg);

    let key = (
        root_set.roots.iter().map(|r| r.path.clone()).collect(),
        cfg.state_dir.value.clone(),
    );
    if watched.as_ref() != Some(&key) {
        *watcher = None;
        let (paths, ignore, tx) = (
            key.0.clone(),
            vec![store.dir.clone(), cfg.state_dir.value.join("logs")],
            tx.clone(),
        );
        *watcher = tokio::task::spawn_blocking(move || watch(&paths, ignore, tx))
            .await
            .unwrap_or_default();
        *watched = Some(key);
    }

    // ponytail: each pass walks every root; walk only the event's directory if many
    // arrivals in a big root make this slow.
    let walk_cfg = cfg.clone();
    match tokio::task::spawn_blocking(move || store::discover(&walk_cfg, WALK_LIMIT)).await {
        Ok(Err(e)) => tracing::warn!(error = %e, "discovery failed"),
        Err(e) => tracing::warn!(error = %e, "discovery panicked"),
        Ok(Ok(_)) => {}
    }

    let is_ready = |c: &Candidate| {
        root_paths
            .get(&c.root)
            .is_some_and(|r| ready.contains(&r.join(&c.path)))
    };
    let mut tried = HashSet::new();
    loop {
        let now_paused = store.paused();
        if now_paused != std::mem::replace(paused, now_paused) {
            if now_paused {
                tracing::warn!("extraction paused: little free disk space");
            } else {
                tracing::info!("extraction resumed");
            }
        }
        if now_paused {
            return Some(DISCOVERY_EVERY);
        }
        let Some(next) = store
            .pending(SETTLE)
            .into_iter()
            .chain(store.pending(Duration::ZERO).into_iter().filter(is_ready))
            .find(|c| !tried.contains(&store::recording_id(c)))
        else {
            break;
        };
        tried.insert(store::recording_id(&next));
        extract(overrides, &store, &next).await;
    }

    // What is left is still being written: look again once the oldest has settled.
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    let settle_ns = SETTLE.as_nanos() as i64;
    let fresh = store
        .pending(Duration::ZERO)
        .iter()
        .filter(|c| !tried.contains(&store::recording_id(c)))
        .map(|c| (settle_ns - (now_ns - c.mtime_ns)).clamp(0, settle_ns))
        .min()
        .map(|ns| Duration::from_nanos(ns as u64) + Duration::from_millis(50));
    match (fresh, watcher.is_some()) {
        (Some(d), _) => Some(d),
        (None, true) => None,
        (None, false) => Some(DISCOVERY_EVERY),
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

async fn extract(overrides: &Overrides, store: &Store, candidate: &Candidate) {
    let reference = store::recording_id(candidate);
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
            let outcome = report(&String::from_utf8_lossy(&out.stderr), &reference);
            tracing::info!(recording = %reference, %outcome, "extraction finished");
        }
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let error = report(&stderr, &reference);
            tracing::warn!(recording = %reference, %error, "extraction failed");
            let message = stderr.trim().lines().rev().take(20).collect::<Vec<_>>();
            let message = message.into_iter().rev().collect::<Vec<_>>().join("\n");
            // The child records ordinary failures itself; this covers crashes.
            let _ = store.record_failure(
                candidate,
                &format!("extraction stopped ({}): {message}", out.status),
            );
        }
        Ok(Err(e)) => tracing::warn!(error = %e, "extraction process failed"),
        Err(_) => {
            tracing::warn!(recording = %reference, "extraction timed out");
            let _ = store.record_failure(
                candidate,
                &format!(
                    "extraction took longer than {} minutes and was stopped",
                    EXTRACTION_TIMEOUT.as_secs() / 60
                ),
            );
        }
    }
}

/// The child's stderr for the log: without the `✔`/`✖` mark, the recording it names
/// again, or the indented code, fix and docs lines of an error (the store keeps those).
fn report(stderr: &str, reference: &str) -> String {
    stderr
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with(' '))
        .map(|l| {
            let l = l.trim_start_matches(['✔', '✖', ' ']);
            l.strip_prefix(reference)
                .and_then(|r| r.strip_prefix(": "))
                .unwrap_or(l)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn report_keeps_the_message() {
        let r = "demo:a.ulg";
        assert_eq!(
            super::report("✔ demo:a.ulg: 3 stream(s) extracted\n", r),
            "3 stream(s) extracted"
        );
        let failed = "✖ demo:a.ulg: writing the store: bad\n  code: recording_unreadable\n  fix:  x\n  docs: y\n";
        assert_eq!(super::report(failed, r), "writing the store: bad");
        let panic = "thread 'main' panicked at src/x.rs:1:1:\nboom\n";
        assert_eq!(
            super::report(panic, r),
            "thread 'main' panicked at src/x.rs:1:1:\nboom"
        );
    }
}
