//! Tells you when a newer Candela release is out.
//!
//! The check only runs where a person is reading the output: the REPL and
//! `candela --help`. Running a program never triggers it, so a script's own
//! output and exit status stay untouched. The notice goes to stderr.
//!
//! The check is opt-out (`CANDELA_NO_UPDATE_CHECK`), skipped in CI, limited to
//! one network request a day, and skipped entirely for binaries the installer
//! did not place (a `cargo build` never reaches the network) and for installs
//! pinned to a release with `install.sh --version`. Every step here fails
//! silently: a broken update check must never get in the way of the session.

use std::fmt::Write as _;
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Redirects to the tag page of the newest release, which is where the version
/// number comes from. The JSON API would give a cleaner answer, but it rate
/// limits anonymous callers.
const LATEST_URL: &str = "https://github.com/lumen-fx/candela/releases/latest";

/// The command the docs tell you to run to install Candela.
const INSTALL_CMD: &str =
    "curl -fsSL https://raw.githubusercontent.com/lumen-fx/candela/main/install.sh | sh";

/// One network check a day.
const CHECK_INTERVAL: u64 = 24 * 60 * 60;

/// How long `candela --help` waits for a check it started. `curl` gives up
/// after four seconds on its own; this is the margin around that.
const WAIT_FOR_RESULT: Duration = Duration::from_secs(5);

/// A check in progress, or a result already known from the cache.
pub enum Check {
    /// The last check saw a newer release and no new request is due yet.
    Known(String),
    /// A request is running on a background thread.
    Running(Receiver<String>),
}

/// Starts a check, unless anything about the environment says not to.
pub fn start() -> Option<Check> {
    if !std::io::stderr().is_terminal() {
        return None;
    }
    if env_set("CANDELA_NO_UPDATE_CHECK") || env_set("CI") {
        return None;
    }
    // No receipt means no installer put this binary here: it was built from
    // source, and a build from source never phones home. A pinned install
    // chose its release, so leave it be.
    if !installed_unpinned() {
        return None;
    }

    let now = now_secs()?;
    let cached = read_cache();
    let last_seen = cached.as_ref().and_then(|c| c.latest.clone());

    if let Some(cache) = &cached
        && now.saturating_sub(cache.checked) < CHECK_INTERVAL
    {
        return last_seen
            .filter(|v| is_newer(v, current()))
            .map(Check::Known);
    }

    Some(Check::Running(spawn(now, last_seen)))
}

/// Prints the notice if the answer has arrived. Returns the check again while
/// it is still running, so the REPL can ask once per prompt without ever
/// waiting on it.
pub fn poll(check: Check) -> Option<Check> {
    match check {
        Check::Known(latest) => {
            notice(&latest);
            None
        }
        Check::Running(rx) => match rx.try_recv() {
            Ok(latest) => {
                notice(&latest);
                None
            }
            Err(TryRecvError::Empty) => Some(Check::Running(rx)),
            Err(TryRecvError::Disconnected) => None,
        },
    }
}

/// Waits for the answer and prints the notice. For one-shot commands that are
/// about to exit; the REPL uses [`poll`] instead.
pub fn finish(check: Option<Check>) {
    match check {
        Some(Check::Known(latest)) => notice(&latest),
        Some(Check::Running(rx)) => {
            if let Ok(latest) = rx.recv_timeout(WAIT_FOR_RESULT) {
                notice(&latest);
            }
        }
        None => {}
    }
}

fn notice(latest: &str) {
    let current = current();
    eprintln!("candela {latest} is available (you have {current}). Update: {INSTALL_CMD}");
}

const fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn env_set(key: &str) -> bool {
    std::env::var_os(key).is_some_and(|v| !v.is_empty())
}

fn now_secs() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Asks the network for the newest release on a background thread, so nothing
/// here delays the prompt. The answer goes back over the channel only when it
/// is newer than what is running.
fn spawn(now: u64, last_seen: Option<String>) -> Receiver<String> {
    let (tx, rx) = channel();
    let _ = std::thread::Builder::new()
        .name(String::from("candela-update-check"))
        .spawn(move || {
            // A failed request still counts as an attempt, so an offline
            // machine backs off instead of retrying on every start. The last
            // known release is kept so the notice survives the failure.
            let latest = fetch_latest().or(last_seen);
            write_cache(now, latest.as_deref());
            if let Some(latest) = latest
                && is_newer(&latest, current())
            {
                let _ = tx.send(latest);
            }
        });
    rx
}

/// Reads the redirect target of the "latest release" URL. Only the headers are
/// fetched, and the response body is never touched.
fn fetch_latest() -> Option<String> {
    probe("curl", &["-fsSI", "--max-time", "4", LATEST_URL]).or_else(|| {
        probe(
            "wget",
            &[
                "--spider",
                "--server-response",
                "--max-redirect=0",
                "--timeout=4",
                "--tries=1",
                LATEST_URL,
            ],
        )
    })
}

/// Runs one downloader and looks for the version in whatever it printed. The
/// exit status is ignored: `wget` reports a redirect it was told not to follow
/// as a failure, and the headers are still there.
fn probe(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    parse_latest_tag(&text)
}

/// Pulls the release version out of HTTP response headers. The last `location`
/// header wins, since a chain of redirects ends at the tag page:
/// `.../releases/tag/v0.3.0`.
fn parse_latest_tag(headers: &str) -> Option<String> {
    let mut found = None;
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("location") {
            continue;
        }
        let target = value.trim().trim_end_matches('/');
        let Some(segment) = target.rsplit('/').next() else {
            continue;
        };
        let tag = segment.strip_prefix('v').unwrap_or(segment);
        // Anything that does not open with a digit is not a release version.
        if tag.starts_with(|c: char| c.is_ascii_digit()) {
            found = Some(String::from(tag));
        }
    }
    found
}

/// Compares two versions by their numeric parts. A trailing pre-release label
/// is ignored, so `0.3.0-rc1` never counts as newer than `0.3.0`.
fn is_newer(latest: &str, current: &str) -> bool {
    version_parts(latest) > version_parts(current)
}

fn version_parts(version: &str) -> [u64; 3] {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let mut parts = [0_u64; 3];
    for (slot, part) in parts.iter_mut().zip(version.split('.')) {
        let end = part
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(part.len());
        *slot = part[..end].parse().unwrap_or(0);
    }
    parts
}

/// True when the installer put this binary here and left the release open.
///
/// The receipt sits next to the binary itself, so resolve the `/usr/local/bin`
/// symlink first.
fn installed_unpinned() -> bool {
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return false;
    };
    let Some(dir) = exe.parent() else {
        return false;
    };
    let Ok(receipt) = std::fs::read_to_string(dir.join("receipt")) else {
        return false;
    };
    !receipt
        .lines()
        .any(|line| line.split_whitespace().next() == Some("pinned"))
}

struct Cache {
    checked: u64,
    latest: Option<String>,
}

#[cfg(windows)]
fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(|d| PathBuf::from(d).join("candela"))
}

#[cfg(not(windows))]
fn cache_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg).join("candela"));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".cache").join("candela"))
}

fn read_cache() -> Option<Cache> {
    let text = std::fs::read_to_string(cache_dir()?.join("update-check")).ok()?;
    let mut cache = Cache {
        checked: 0,
        latest: None,
    };
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        match (fields.next(), fields.next()) {
            (Some("checked"), Some(value)) => cache.checked = value.parse().unwrap_or(0),
            (Some("latest"), Some(value)) => cache.latest = Some(String::from(value)),
            _ => {}
        }
    }
    Some(cache)
}

fn write_cache(now: u64, latest: Option<&str>) {
    let Some(dir) = cache_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let mut text = format!("checked {now}\n");
    if let Some(latest) = latest {
        let _ = writeln!(text, "latest {latest}");
    }
    let _ = std::fs::write(dir.join("update-check"), text);
}

#[cfg(test)]
mod tests {
    use super::{is_newer, parse_latest_tag};

    #[test]
    fn newer_versions_win() {
        assert!(is_newer("0.4.0", "0.3.9"));
        assert!(is_newer("0.3.10", "0.3.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("v0.4.0", "0.3.0"));
    }

    #[test]
    fn same_or_older_versions_do_not() {
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.3.0", "0.4.0"));
        assert!(!is_newer("0.3.0-rc1", "0.3.0"));
        assert!(!is_newer("0.3", "0.3.1"));
        assert!(!is_newer("latest", "0.0.1"));
        assert!(!is_newer("", "0.0.1"));
    }

    #[test]
    fn short_versions_pad_with_zeros() {
        assert!(is_newer("0.4", "0.3.9"));
        assert!(!is_newer("1", "1.0.0"));
    }

    #[test]
    fn tag_comes_from_the_last_location_header() {
        let headers = "HTTP/2 302\r\n\
             server: github.com\r\n\
             location: https://github.com/lumen-fx/candela/releases/tag/v0.3.0\r\n\r\n";
        assert_eq!(parse_latest_tag(headers).as_deref(), Some("0.3.0"));

        let chained = "HTTP/1.1 301\r\n\
             Location: http://github.com/lumen-fx/candela/releases/latest\r\n\
             HTTP/2 302\r\n\
             Location: https://github.com/lumen-fx/candela/releases/tag/1.2.3\r\n";
        assert_eq!(parse_latest_tag(chained).as_deref(), Some("1.2.3"));
    }

    #[test]
    fn trailing_slashes_and_indented_headers_are_fine() {
        let headers = "  Location: https://github.com/lumen-fx/candela/releases/tag/v10.0.2/\n";
        assert_eq!(parse_latest_tag(headers).as_deref(), Some("10.0.2"));
    }

    #[test]
    fn headers_without_a_version_give_nothing() {
        assert_eq!(parse_latest_tag(""), None);
        assert_eq!(
            parse_latest_tag("HTTP/2 200\r\ncontent-length: 12\r\n"),
            None
        );
        assert_eq!(
            parse_latest_tag("location: https://github.com/lumen-fx/candela/releases/latest\r\n"),
            None
        );
        assert_eq!(
            parse_latest_tag("x-location: /releases/tag/v9.9.9\r\n"),
            None
        );
    }
}
