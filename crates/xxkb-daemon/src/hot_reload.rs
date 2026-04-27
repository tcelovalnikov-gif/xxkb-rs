//! Watch `~/.config/xxkb/config.toml` and call back when it changes.

use std::{path::PathBuf, time::Duration};

use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode, DebounceEventResult};

/// Returns a guard that, when dropped, stops the watcher.
pub fn start_watch(
    path: PathBuf,
    on_change: impl Fn() + Send + Sync + 'static,
) -> Option<WatcherGuard> {
    let parent = path.parent()?.to_path_buf();
    let target = path.clone();

    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        move |res: DebounceEventResult| match res {
            Ok(events) => {
                let touched = events.iter().any(|e| e.path == target);
                if touched {
                    tracing::debug!("config touched, reloading");
                    on_change();
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "config watcher error");
            }
        },
    )
    .ok()?;

    debouncer
        .watcher()
        .watch(&parent, RecursiveMode::NonRecursive)
        .ok()?;

    Some(WatcherGuard {
        _debouncer: Box::new(debouncer),
    })
}

/// RAII guard for the watcher.
pub struct WatcherGuard {
    _debouncer: Box<dyn std::any::Any + Send + Sync>,
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use super::start_watch;

    /// Writing the watched file should debounce-fire the callback within
    /// a couple of seconds.
    #[test]
    fn fires_on_target_file_write() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# initial\n").unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_cb = counter.clone();
        let _guard = start_watch(path.clone(), move || {
            counter_cb.fetch_add(1, Ordering::SeqCst);
        })
        .expect("watcher started");

        // Give the watcher a moment to subscribe before we mutate.
        thread::sleep(Duration::from_millis(150));
        std::fs::write(&path, "# changed\n").unwrap();

        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if counter.load(Ordering::SeqCst) > 0 {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("hot_reload callback never fired within 5s");
    }

    /// Writing a *sibling* file under the same directory must not fire
    /// the callback — we only care about the target path.
    #[test]
    fn ignores_unrelated_sibling_writes() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let other = dir.path().join("notes.txt");
        std::fs::write(&path, "# initial\n").unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_cb = counter.clone();
        let _guard = start_watch(path, move || {
            counter_cb.fetch_add(1, Ordering::SeqCst);
        })
        .expect("watcher started");

        thread::sleep(Duration::from_millis(150));
        std::fs::write(&other, "hello\n").unwrap();
        thread::sleep(Duration::from_millis(800));

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "callback fired for an unrelated sibling write"
        );
    }
}
