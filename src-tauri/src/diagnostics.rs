//! Local-only error reporting (P6 dev spec §6): a crash file written by the panic hook, and a
//! diagnostics text the user can copy. Nothing is sent anywhere, and neither contains user
//! content (the logs hold ids, counts and timings only).

use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};

pub const CRASH_SEEN_KEY: &str = "crash_seen";
const LOG_LINES: usize = 200;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Set by build.rs.
pub const COMMIT: &str = env!("TE_COMMIT");
pub const BUILD_DATE: &str = env!("TE_BUILD_DATE");

/// Write `crash-<unix seconds>.txt` into the log folder; returns its path.
pub fn write_crash(dir: &Path, now: DateTime<Utc>, message: &str, location: &str, backtrace: &str) -> Option<PathBuf> {
    let path = dir.join(format!("crash-{}.txt", now.timestamp()));
    let text = format!(
        "Tech English {VERSION} ({COMMIT}, built {BUILD_DATE})\ntime: {}\npanic: {message}\nat: {location}\n\n{backtrace}\n",
        now.to_rfc3339_opts(SecondsFormat::Secs, true)
    );
    std::fs::write(&path, text).ok().map(|_| path)
}

/// Every panic leaves a crash file, also with `panic = "abort"` (the hook runs first).
pub fn install_panic_hook(log_dir: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let now = DateTime::<Utc>::from(std::time::SystemTime::now());
        write_crash(&log_dir, now, &message, &location, &backtrace);
        default(info);
    }));
}

/// The newest crash file: (file name, text).
pub fn latest_crash(dir: &Path) -> Option<(String, String)> {
    let mut files: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let ts = name.strip_prefix("crash-")?.strip_suffix(".txt")?.parse::<u64>().ok()?;
            Some((ts, e.path()))
        })
        .collect();
    files.sort();
    let (_, path) = files.pop()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    Some((name, std::fs::read_to_string(&path).ok()?))
}

/// Keep the 5 newest crash files.
pub fn prune_crashes(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("crash-")))
        .collect();
    files.sort();
    let extra = files.len().saturating_sub(5);
    for p in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
}

/// The last lines of the newest `app.*.log`.
pub fn last_log_lines(dir: &Path) -> Vec<String> {
    let newest = std::fs::read_dir(dir).ok().and_then(|entries| {
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("app") && n.to_string_lossy().ends_with("log"))
            })
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
    });
    let Some(text) = newest.and_then(|p| std::fs::read(p).ok()) else {
        return vec![];
    };
    let text = String::from_utf8_lossy(&text);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(LOG_LINES)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

pub fn system_line() -> String {
    let os = sysinfo::System::long_os_version().unwrap_or_else(|| "macOS".into());
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    let chip = sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default();
    format!("{os} · {chip} · {:.0} GB RAM", sys.total_memory() as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::parse_ts;

    #[test]
    fn crash_files_are_written_found_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        assert!(latest_crash(dir.path()).is_none());
        let t = parse_ts("2026-09-30T10:00:00Z").unwrap();
        for i in 0..7 {
            write_crash(
                dir.path(),
                t + chrono::Duration::seconds(i),
                &format!("boom {i}"),
                "src/x.rs:1",
                "bt",
            )
            .unwrap();
        }
        std::fs::write(dir.path().join("app.2026-09-30.log"), "a\nb\n").unwrap();
        let (name, text) = latest_crash(dir.path()).unwrap();
        assert_eq!(name, format!("crash-{}.txt", t.timestamp() + 6));
        assert!(text.contains("panic: boom 6") && text.contains("at: src/x.rs:1") && text.contains(VERSION));
        prune_crashes(dir.path());
        let left = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("crash-"))
            .count();
        assert_eq!(left, 5);
        assert!(
            latest_crash(dir.path()).unwrap().1.contains("boom 6"),
            "the newest is kept"
        );
    }

    #[test]
    fn only_the_last_log_lines_are_returned() {
        let dir = tempfile::tempdir().unwrap();
        assert!(last_log_lines(dir.path()).is_empty());
        let lines: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.path().join("app.2026-09-30.log"), lines.join("\n")).unwrap();
        let got = last_log_lines(dir.path());
        assert_eq!(got.len(), 200);
        assert_eq!((got[0].as_str(), got[199].as_str()), ("line 300", "line 499"));
    }

    #[test]
    fn build_info_is_set() {
        assert!(!COMMIT.is_empty() && BUILD_DATE.len() == 10, "{COMMIT} {BUILD_DATE}");
        assert!(system_line().contains("RAM"));
    }
}
