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
//!
//! On Windows, `candela --help` goes one step further and offers to install the
//! new release, since there is an installer to hand it to. The REPL never
//! offers: it is holding its own line reader, and a second one competing for
//! stdin would eat the keystrokes meant for the prompt.

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
#[cfg(not(windows))]
const INSTALL_CMD: &str = "curl -fsSL https://candela.lumenfx.dev/install.sh | sh";

/// Windows installs from a package rather than a shell script, so the notice
/// names the download instead of a command. The same URL is what the offer
/// below fetches.
#[cfg(windows)]
const INSTALL_CMD: &str =
    "https://github.com/lumen-fx/candela/releases/latest/download/candela-x86_64-windows.msi";

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
///
/// This is also the only place that offers to install the update, because the
/// process is on its way out: the installer runs against files nothing is
/// holding open.
pub fn finish(check: Option<Check>) {
    let latest = match check {
        Some(Check::Known(latest)) => Some(latest),
        Some(Check::Running(rx)) => rx.recv_timeout(WAIT_FOR_RESULT).ok(),
        None => None,
    };
    if let Some(latest) = latest {
        notice(&latest);
        offer_update(&latest);
    }
}

fn notice(latest: &str) {
    let current = current();
    eprintln!("candela {latest} is available (you have {current}). Update: {INSTALL_CMD}");
}

/// Reads one line and decides whether it is a yes. Anything else, an empty line
/// included, is a no.
#[cfg(any(windows, test))]
fn said_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Unix updates through the install script the notice already named.
#[cfg(not(windows))]
const fn offer_update(_latest: &str) {}

/// Asks whether to install the new release, and sets it up if the answer is
/// yes. The install itself happens after this process exits: `msiexec` cannot
/// replace `candela.exe` while it is the running image, and a package that
/// finds its own files in use fails instead of asking.
#[cfg(windows)]
fn offer_update(latest: &str) {
    use std::io::Write as _;

    // Both ends have to be a terminal: stderr because that is where the
    // question goes, stdin because that is where the answer comes from. A
    // piped stdin would answer for a person who never saw the question.
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return;
    }

    eprint!("Update now? [y/N] ");
    let _ = std::io::stderr().flush();

    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() || !said_yes(&answer) {
        return;
    }

    let Some(installer) = download_installer() else {
        eprintln!("Could not download the installer. Get it from {INSTALL_CMD}");
        return;
    };

    if spawn_installer(&installer).is_err() {
        eprintln!(
            "Could not start the installer. Run it yourself: {}",
            installer.display()
        );
        return;
    }

    eprintln!(
        "Candela {latest} installs once this command exits. Open a new terminal when it is done."
    );
}

/// Downloads the package to the temp directory and returns where it landed.
///
/// `curl.exe` ships with Windows and what it writes carries no
/// Mark-of-the-Web, so this path never meets SmartScreen. PowerShell covers the
/// machines that have no `curl.exe`.
#[cfg(windows)]
fn download_installer() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("TEMP")?).join("candela-update.msi");
    let dest = path.to_str()?;

    let downloaded = match Command::new("curl.exe")
        .args(["-fL", "--max-time", "300", "-o", dest, INSTALL_CMD])
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Invoke-WebRequest -Uri '{INSTALL_CMD}' -OutFile '{dest}'"),
            ])
            .status()
            .is_ok_and(|status| status.success()),
    };

    downloaded.then_some(path)
}

/// Starts a detached helper that waits for this process to go away, runs the
/// installer, and removes the download. Detaching is the point: it outlives the
/// command that spawned it, and it is not waited on here.
#[cfg(windows)]
fn spawn_installer(installer: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt as _;

    const DETACHED_PROCESS: u32 = 0x8;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;

    let package = installer.display();
    let pid = std::process::id();
    let script = format!(
        "Wait-Process -Id {pid} -Timeout 120 -ErrorAction SilentlyContinue; \
         Start-Process msiexec -ArgumentList '/i','{package}','/passive','/norestart' -Wait; \
         Remove-Item '{package}' -ErrorAction SilentlyContinue"
    );

    Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
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
    use super::{is_newer, parse_latest_tag, said_yes};

    #[test]
    fn only_a_plain_yes_is_a_yes() {
        assert!(said_yes("y\n"));
        assert!(said_yes("Y\r\n"));
        assert!(said_yes("  yes  "));
        assert!(said_yes("YES"));
    }

    #[test]
    fn everything_else_is_a_no() {
        assert!(!said_yes(""));
        assert!(!said_yes("\n"));
        assert!(!said_yes("n"));
        assert!(!said_yes("no"));
        assert!(!said_yes("yeah"));
        assert!(!said_yes("y e s"));
    }

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
