use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::vault;

pub type Snapshots = Arc<Mutex<HashMap<String, String>>>;

#[derive(Clone, Serialize)]
pub struct DayChanged {
    pub date: String,
    pub content: String,
}

/// Start watching the days directory. Emits `day-changed-externally` when a
/// day file is modified on disk and its content differs from the in-memory
/// snapshot (i.e. it wasn't us). The returned watcher must be kept alive.
pub fn start(app: AppHandle, snapshots: Snapshots) -> Result<RecommendedWatcher, String> {
    let days_dir = vault::days_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&days_dir).map_err(|e| e.to_string())?;

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .map_err(|e| e.to_string())?;
    watcher
        .watch(&days_dir, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;

    thread::spawn(move || {
        // Coalesce bursts: many editors / OneDrive emit several events per save.
        let debounce = Duration::from_millis(120);
        loop {
            let evt = match rx.recv() {
                Ok(Ok(e)) => e,
                Ok(Err(_)) => continue,
                Err(_) => break,
            };
            // Drain any queued events for the debounce window so we only act once.
            let mut events = vec![evt];
            let deadline = std::time::Instant::now() + debounce;
            while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
                match rx.recv_timeout(remaining) {
                    Ok(Ok(e)) => events.push(e),
                    Ok(Err(_)) => {}
                    Err(_) => break,
                }
            }

            let mut handled: std::collections::HashSet<PathBuf> = Default::default();
            for e in events {
                if !matches!(
                    e.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    continue;
                }
                for p in e.paths {
                    if handled.contains(&p) {
                        continue;
                    }
                    if p.extension().and_then(|s| s.to_str()) != Some("md") {
                        continue;
                    }
                    let date = match p.file_stem().and_then(|s| s.to_str()) {
                        Some(s) => s.to_string(),
                        None => continue,
                    };
                    handled.insert(p.clone());

                    let content = std::fs::read_to_string(&p).unwrap_or_default();
                    let mut snaps = snapshots.lock().unwrap();
                    let prev = snaps.get(&date).cloned();
                    if prev.as_deref() == Some(content.as_str()) {
                        continue; // self-write
                    }
                    snaps.insert(date.clone(), content.clone());
                    drop(snaps);

                    let _ = app.emit(
                        "day-changed-externally",
                        DayChanged { date, content },
                    );
                }
            }
        }
    });

    Ok(watcher)
}
