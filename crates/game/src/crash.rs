//! Crash reports: a panic posts one short, anonymous report to the
//! project's public ntfy topic, where `.github/workflows/crash-reports.yml`
//! picks it up and files (or comments on) a GitHub issue labelled `crash`.
//!
//! A report holds the version, OS and architecture, the panicking thread,
//! the panic message and location, and a backtrace. Home folder and user
//! name are replaced before sending; nothing else about the machine, the
//! player or the match is included. The same text is written to
//! `<data dir>/last-crash.txt`.
//!
//! Off with `ROUNDS_PORT_CRASH_REPORTS=0`. Debug builds and CI runs only
//! report with `ROUNDS_PORT_CRASH_REPORTS=1`. At most one report per process,
//! sent from a helper thread that the panicking thread waits on for a few
//! seconds at most.

use std::backtrace::Backtrace;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Public topic; the workflow polls it. Anyone may post there, so the
/// workflow treats what it reads as untrusted text.
const TOPIC_URL: &str = "https://ntfy.sh/rounds-port-crash-t2g6cbvu1b10";
/// ntfy turns longer bodies into attachments.
const MAX_BODY: usize = 3800;
const SEND_TIMEOUT: Duration = Duration::from_secs(4);

static REPORTED: AtomicBool = AtomicBool::new(false);

/// Installs the panic hook (keeps the default one, which prints first).
pub fn install() {
    if !enabled() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if REPORTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(non-string panic payload)".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".into());
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        let body = report(
            &message,
            &location,
            &thread,
            &Backtrace::force_capture().to_string(),
        );
        if let Some(dir) = crate::paths::data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("last-crash.txt"), &body);
        }
        let title = format!("rounds-port {} crash", env!("CARGO_PKG_VERSION"));
        send(title, body);
    }));
}

fn enabled() -> bool {
    match std::env::var("ROUNDS_PORT_CRASH_REPORTS").as_deref() {
        Ok("0") | Ok("off") | Ok("no") => false,
        Ok("1") | Ok("on") | Ok("yes") => true,
        _ => !cfg!(debug_assertions) && std::env::var_os("CI").is_none(),
    }
}

/// The report text. The `fingerprint:` line groups reports of the same
/// fault (version + location) into one issue.
fn report(message: &str, location: &str, thread: &str, backtrace: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut body = format!(
        "fingerprint: {:016x}\nversion: {version}\nos: {} {}\nthread: {}\nlocation: {}\nmessage: {}\n\nbacktrace:\n",
        fnv1a(format!("{version} {location}").as_bytes()),
        std::env::consts::OS,
        std::env::consts::ARCH,
        scrub(thread),
        scrub(location),
        scrub(message),
    );
    body.push_str(&scrub(backtrace));
    truncate(&mut body, MAX_BODY);
    body
}

/// Removes the home folder and the user name from `text`.
fn scrub(text: &str) -> String {
    let mut out = text.to_string();
    for var in ["HOME", "USERPROFILE"] {
        if let Ok(home) = std::env::var(var)
            && home.len() > 1
        {
            out = out.replace(&home, "~");
        }
    }
    for var in ["USER", "USERNAME", "LOGNAME"] {
        if let Ok(user) = std::env::var(var)
            && user.len() >= 3
        {
            out = out.replace(&user, "<user>");
        }
    }
    out
}

fn truncate(s: &mut String, max: usize) {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n…");
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Posts in a helper thread; waits for it at most `SEND_TIMEOUT`.
fn send(title: String, body: String) {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("crash-report".into())
        .spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(SEND_TIMEOUT))
                .build()
                .into();
            let _ = agent
                .post(TOPIC_URL)
                .header("Title", &title)
                .header("Tags", "boom")
                .send(body.as_bytes());
            let _ = done_tx.send(());
        });
    if spawned.is_ok() {
        let _ = done_rx.recv_timeout(SEND_TIMEOUT + Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_fingerprint_and_fits() {
        let long = "x".repeat(10_000);
        let a = report("boom", "crates/game/src/a.rs:1", "main", &long);
        assert!(a.starts_with("fingerprint: "));
        assert!(a.len() <= MAX_BODY + 4);
        let b = report("other text", "crates/game/src/a.rs:1", "main", "");
        assert_eq!(a.lines().next(), b.lines().next());
        let c = report("boom", "crates/game/src/a.rs:2", "main", "");
        assert_ne!(a.lines().next(), c.lines().next());
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let mut s = "ä".repeat(10);
        truncate(&mut s, 5);
        assert!(s.starts_with("ää"));
    }
}
