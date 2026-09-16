//! Talking to `lpm`, the registry client.
//!
//! Dependencies are resolved by `lpm`, the Go client of the registry at
//! reg.lumenfx.dev. candela hands it the requirements a manifest lists and
//! reads back where each package landed on disk; it does not resolve versions,
//! write the lock file, or manage the cache. That keeps one implementation of
//! each of those, in the client that owns the registry protocol.
//!
//! `lpm` is found through `LPM_BIN`, then `PATH`, then the one shared path it
//! installs to. When it is missing or older than candela needs, it is
//! downloaded from the registry's own releases, checksum-verified, and unpacked
//! to that shared path.

use crate::update::is_newer;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

/// The oldest `lpm` candela works with. An older one on the machine is
/// replaced.
///
/// 0.2.0 is the first release carrying the `install` verb candela resolves
/// through; 0.1.0 predates it and cannot answer.
pub const LPM_MIN_VERSION: &str = "0.2.0";

/// Where `lpm` releases come from. The newest tag is the last path segment of
/// the URL `<this>/latest` redirects to, the same resolution candela's own
/// update check uses.
const RELEASES_URL: &str = "https://github.com/lumen-fx/registry/releases";

/// Anything that stops a resolution.
#[derive(Debug)]
pub enum LpmError {
    /// `lpm` is not on the machine and could not be downloaded.
    Unavailable(String),
    /// `lpm` ran and reported a problem. The text is its own message.
    Failed(String),
    /// The lock file would change and `--locked` said it must not.
    LockWouldChange,
    /// A package is not in the cache and `--offline` said not to fetch it.
    OfflineMiss(String),
    /// `lpm` printed something candela could not read.
    BadOutput(String),
}

impl fmt::Display for LpmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(e) | Self::Failed(e) | Self::OfflineMiss(e) => write!(f, "{e}"),
            Self::LockWouldChange => write!(
                f,
                "the lock file would change, and --locked says it must not. Run candela update to record the change"
            ),
            Self::BadOutput(e) => write!(f, "cannot read what lpm reported: {e}"),
        }
    }
}

/// One package `lpm` resolved, unpacked and ready to import.
#[derive(Clone, Debug)]
pub struct Package {
    pub name: String,
    pub version: String,
    /// The platform the package is for. candela packages say `candela`.
    pub platform: String,
    pub target: String,
    /// The package root: the directory its `candela.toml` sits in.
    pub dir: PathBuf,
}

/// What a resolution asks for.
pub struct Request<'a> {
    /// Where the lock file goes.
    pub lock: &'a Path,
    /// The requirements, as `(name, requirement)` pairs.
    pub requirements: &'a [(String, String)],
    /// Refuse to change the lock file.
    pub locked: bool,
    /// Resolve from the cache alone.
    pub offline: bool,
}

/// Resolves `request` and returns the packages it settled on.
///
/// # Errors
///
/// Returns an [`LpmError`] when `lpm` cannot be found, refuses the request, or
/// reports something unreadable.
pub fn install(request: &Request<'_>) -> Result<Vec<Package>, LpmError> {
    run_resolution("install", request, &[])
}

/// Re-resolves `request`, letting the lock file move.
///
/// `names` narrows the update to those packages; an empty list updates
/// everything.
///
/// # Errors
///
/// The same as [`install`].
pub fn update(request: &Request<'_>, names: &[String]) -> Result<Vec<Package>, LpmError> {
    run_resolution("update", request, names)
}

fn run_resolution(
    verb: &str,
    request: &Request<'_>,
    names: &[String],
) -> Result<Vec<Package>, LpmError> {
    let binary = locate()?;
    let mut command = Command::new(&binary);
    command
        .arg(verb)
        .arg("--lock")
        .arg(request.lock)
        .arg("--target")
        .arg(host_target())
        .arg("--host")
        .arg(format!("candela@{}", env!("CARGO_PKG_VERSION")));
    for (name, requirement) in request.requirements {
        command.arg("--req").arg(format!("{name}@{requirement}"));
    }
    if request.locked {
        command.arg("--locked");
    }
    if request.offline {
        command.arg("--offline");
    }
    command.arg("--json");
    for name in names {
        command.arg(name);
    }

    let output = command
        .output()
        .map_err(|e| LpmError::Unavailable(format!("cannot run {}: {e}", binary.display())))?;

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    match output.status.code() {
        Some(0) => {}
        Some(3) => return Err(LpmError::LockWouldChange),
        Some(4) => return Err(LpmError::OfflineMiss(lpm_message(&stderr, verb))),
        _ => return Err(LpmError::Failed(lpm_message(&stderr, verb))),
    }

    parse_packages(&String::from_utf8_lossy(&output.stdout))
}

/// Registers `name` on the registry as a candela package.
///
/// # Errors
///
/// Returns an [`LpmError`] when `lpm` cannot be found or refuses.
pub fn publish(name: &str, description: Option<&str>) -> Result<(), LpmError> {
    let binary = locate()?;
    let mut command = Command::new(&binary);
    command
        .arg("publish")
        .arg(name)
        .arg("--platform")
        .arg("candela");
    if let Some(description) = description {
        command.arg("--description").arg(description);
    }
    run_to_completion(command, "publish")
}

/// Publishes one release of `name`.
///
/// `artifacts` are `(target, url)` pairs, `dependencies` are `(name,
/// requirement)` pairs, and `host` is the candela requirement the release
/// carries.
///
/// # Errors
///
/// Returns an [`LpmError`] when `lpm` cannot be found or refuses.
pub fn release(
    name: &str,
    version: &str,
    artifacts: &[(String, String)],
    dependencies: &[(String, String)],
    host: &str,
    description: Option<&str>,
) -> Result<(), LpmError> {
    let binary = locate()?;
    let mut command = Command::new(&binary);
    command.arg("release").arg(name).arg(version);
    for (target, url) in artifacts {
        command.arg("--artifact").arg(format!("{target}={url}"));
    }
    for (dep, requirement) in dependencies {
        command.arg("--dep").arg(format!("{dep}@{requirement}"));
    }
    command.arg("--requires").arg(host);
    if let Some(description) = description {
        command.arg("-d").arg(description);
    }
    run_to_completion(command, "release")
}

fn run_to_completion(mut command: Command, verb: &str) -> Result<(), LpmError> {
    let output = command
        .output()
        .map_err(|e| LpmError::Unavailable(format!("cannot run lpm {verb}: {e}")))?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        let text = text.trim();
        if !text.is_empty() {
            println!("{text}");
        }
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(LpmError::Failed(lpm_message(&stderr, verb)))
}

/// What to show when `lpm` fails. It reports one line on stderr, which is the
/// message; a silent failure gets one naming the verb instead.
fn lpm_message(stderr: &str, verb: &str) -> String {
    if stderr.is_empty() {
        format!("lpm {verb} failed and said nothing")
    } else {
        stderr.to_owned()
    }
}

// ---------------------------------------------------------------------------
// Finding lpm
// ---------------------------------------------------------------------------

/// The path `lpm` installs to, which is the one place candela and every other
/// client look.
#[must_use]
pub fn shared_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(|base| {
                PathBuf::from(base)
                    .join("Programs")
                    .join("lpm")
                    .join("lpm.exe")
            })
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(|home| PathBuf::from(home).join(".local").join("bin").join("lpm"))
    }
}

/// Finds an `lpm` new enough to use, downloading one when the machine has none
/// or only an older one.
///
/// `LPM_BIN` names it outright. Otherwise `PATH` is searched, and then the
/// shared path.
///
/// # Errors
///
/// Returns [`LpmError::Unavailable`] when there is none and one cannot be
/// downloaded.
pub fn locate() -> Result<PathBuf, LpmError> {
    for candidate in candidates() {
        if let Some(version) = installed_version(&candidate)
            && !is_newer(LPM_MIN_VERSION, &version)
        {
            return Ok(candidate);
        }
    }
    download()
}

fn candidates() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Some(named) = std::env::var_os("LPM_BIN").filter(|v| !v.is_empty()) {
        found.push(PathBuf::from(named));
    }
    if let Some(on_path) = on_path() {
        found.push(on_path);
    }
    if let Some(shared) = shared_path() {
        found.push(shared);
    }
    found
}

/// The first `lpm` on `PATH`, if any.
fn on_path() -> Option<PathBuf> {
    let name = if cfg!(windows) { "lpm.exe" } else { "lpm" };
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// Asks a binary its version. `lpm --version` prints
/// `lpm version X.Y.Z (...)`; anything else is treated as no answer.
fn installed_version(binary: &Path) -> Option<String> {
    let output = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&output.stdout))
}

/// Pulls the version out of `lpm --version` output.
fn parse_version(text: &str) -> Option<String> {
    let line = text.lines().next()?;
    let mut words = line.split_whitespace();
    if words.next()? != "lpm" || words.next()? != "version" {
        return None;
    }
    let version = words.next()?.trim_start_matches('v');
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| version.to_owned())
}

// ---------------------------------------------------------------------------
// Downloading lpm
// ---------------------------------------------------------------------------

/// Downloads `lpm` to the shared path and returns where it landed.
///
/// The release is resolved the same way the toolchain's own update check
/// resolves candela's: the newest tag is the end of the `/latest` redirect, and
/// the archive is checked against the `checksums.txt` published beside it
/// before anything is unpacked.
fn download() -> Result<PathBuf, LpmError> {
    let dest = shared_path().ok_or_else(|| {
        LpmError::Unavailable(String::from(
            "lpm is not installed and there is no home directory to install it into. Install it yourself and set LPM_BIN",
        ))
    })?;

    let base = releases_url();
    let tag = latest_tag(&base).ok_or_else(|| {
        LpmError::Unavailable(format!(
            "lpm is not installed and the newest release could not be resolved. Install it from {base} and set LPM_BIN"
        ))
    })?;
    let version = tag.trim_start_matches('v');
    let asset = asset_name(version);
    let asset_url = format!("{base}/download/{tag}/{asset}");
    let sums_url = format!("{base}/download/{tag}/checksums.txt");

    let staging = std::env::temp_dir().join(format!("candela-lpm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|e| LpmError::Unavailable(format!("cannot create {}: {e}", staging.display())))?;
    let cleanup = Cleanup(staging.clone());

    let sums = staging.join("checksums.txt");
    fetch(&sums_url, &sums)?;
    let archive = staging.join(&asset);
    fetch(&asset_url, &archive)?;

    let expected = recorded_sha(&std::fs::read_to_string(&sums).unwrap_or_default(), &asset)
        .ok_or_else(|| {
            LpmError::Unavailable(format!(
                "release {tag} of lpm publishes no checksum for {asset}. See {base}"
            ))
        })?;
    let got = sha256_of(&archive).ok_or_else(|| {
        LpmError::Unavailable(String::from(
            "cannot verify the lpm download: none of sha256sum, shasum or certutil is here to hash it",
        ))
    })?;
    if !got.eq_ignore_ascii_case(&expected) {
        return Err(LpmError::Unavailable(format!(
            "the lpm download does not match the checksum published with {tag}. Nothing was installed"
        )));
    }

    unpack(&archive, &staging)?;
    let binary_name = if cfg!(windows) { "lpm.exe" } else { "lpm" };
    let unpacked = find_file(&staging, binary_name)
        .ok_or_else(|| LpmError::Unavailable(format!("{asset} holds no {binary_name}")))?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            LpmError::Unavailable(format!("cannot create {}: {e}", parent.display()))
        })?;
    }
    let _ = std::fs::remove_file(&dest);
    std::fs::copy(&unpacked, &dest)
        .map_err(|e| LpmError::Unavailable(format!("cannot write {}: {e}", dest.display())))?;
    make_executable(&dest);
    drop(cleanup);

    eprintln!("Installed lpm {version} to {}", dest.display());
    Ok(dest)
}

/// Removes a staging directory when the download leaves, however it leaves.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Where releases are read from. The environment override exists so the
/// download path can be tested against local files.
fn releases_url() -> String {
    std::env::var("CANDELA_LPM_RELEASES_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| String::from(RELEASES_URL))
}

/// The newest release tag. A test that points `CANDELA_LPM_RELEASES_URL` at
/// local files names the tag outright, since local files do not redirect.
fn latest_tag(base: &str) -> Option<String> {
    if let Some(tag) = std::env::var("CANDELA_LPM_TAG")
        .ok()
        .filter(|v| !v.is_empty())
    {
        return Some(tag);
    }
    let url = format!("{base}/latest");
    let out = Command::new("curl")
        .args(["-fsSI", "--max-time", "10", &url])
        .output()
        .ok()
        .or_else(|| {
            Command::new("wget")
                .args([
                    "--spider",
                    "--server-response",
                    "--max-redirect=0",
                    "--timeout=10",
                    "--tries=1",
                    &url,
                ])
                .output()
                .ok()
        })?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    last_location_tag(&text)
}

/// The tag at the end of a chain of redirects: the last `location` header's
/// final path segment.
fn last_location_tag(headers: &str) -> Option<String> {
    let mut found = None;
    for line in headers.lines() {
        let Some((name, target)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("location") {
            continue;
        }
        let segment = target.trim().trim_end_matches('/').rsplit('/').next()?;
        if segment
            .trim_start_matches('v')
            .starts_with(|c: char| c.is_ascii_digit())
        {
            found = Some(String::from(segment));
        }
    }
    found
}

/// The asset `lpm` publishes for this machine.
///
/// The names are the Go release convention the registry publishes under:
/// `lpm_<version>_<os>_<arch>` with `darwin` for macOS and `amd64`/`arm64` for
/// the processors, zipped on Windows and tarred everywhere else.
fn asset_name(version: &str) -> String {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    format!("lpm_{version}_{os}_{arch}.{extension}")
}

/// The checksum recorded for `name` in a `checksums.txt`, which is
/// `sha256sum`'s own format: one `<hex>  <filename>` line per asset.
fn recorded_sha(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(hash), Some(file)) = (fields.next(), fields.next()) else {
            continue;
        };
        if file.trim_start_matches('*') == name {
            return Some(hash.to_owned());
        }
    }
    None
}

fn fetch(url: &str, dest: &Path) -> Result<(), LpmError> {
    let failed = || LpmError::Unavailable(format!("cannot download {url}"));
    let curl = Command::new("curl")
        .args(["-fsSL", "--max-time", "120", "-o"])
        .arg(dest)
        .arg(url)
        .status();
    match curl {
        Ok(status) if status.success() => return Ok(()),
        Ok(_) => return Err(failed()),
        Err(_) => {}
    }
    let wget = Command::new("wget")
        .args(["-q", "--timeout=120", "-O"])
        .arg(dest)
        .arg(url)
        .status()
        .map_err(|_| {
            LpmError::Unavailable(String::from(
                "lpm is not installed and neither curl nor wget is here to download it. Install it yourself and set LPM_BIN",
            ))
        })?;
    if wget.success() {
        Ok(())
    } else {
        Err(failed())
    }
}

/// A hashing tool: the program name, the arguments that go before the path,
/// and the arguments that go after it.
type HashingTool = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

/// The hashing tools, in the order they are tried.
///
/// Windows is asked `certutil` first, because it is the tool the system itself
/// ships. The GNU `sha256sum` that arrives on Windows with Git prints a path
/// holding a backslash in escaped form, which is legal output and easy to
/// misread; `certutil` has no such shape to read around.
fn hashing_tools() -> [HashingTool; 3] {
    let certutil = ("certutil", &["-hashfile"][..], &["SHA256"][..]);
    let sha256sum = ("sha256sum", &[][..], &[][..]);
    let shasum = ("shasum", &["-a", "256"][..], &[][..]);
    if cfg!(windows) {
        [certutil, sha256sum, shasum]
    } else {
        [sha256sum, shasum, certutil]
    }
}

/// Hashes a file with whichever hashing tool the machine has.
fn sha256_of(path: &Path) -> Option<String> {
    for (program, before, after) in hashing_tools() {
        if let Ok(out) = Command::new(program)
            .args(before)
            .arg(path)
            .args(after)
            .output()
            && out.status.success()
            && let Some(hash) = sha256_in(&String::from_utf8_lossy(&out.stdout))
        {
            return Some(hash);
        }
    }
    None
}

/// The digest in what a hashing tool printed, whichever tool printed it.
///
/// Every shape any of them uses puts the digest where this finds it:
///
/// - `sha256sum` and `shasum` print `<hex>  <filename>`. GNU coreutils escapes
///   a filename holding a backslash or a newline by writing the name escaped
///   and marking the line with a leading `\`, so the digest is the first field
///   once that marker is off. Windows paths hold backslashes, which is why the
///   marker shows up at all.
/// - `certutil -hashfile` prints the digest on a line of its own, spaced into
///   byte pairs on older builds and contiguous on newer ones.
fn sha256_in(text: &str) -> Option<String> {
    for line in text.lines() {
        for field in line.split_whitespace() {
            if let Some(hash) = sha256_digest(field.trim_start_matches('\\')) {
                return Some(hash);
            }
        }
        let spaced: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(hash) = sha256_digest(&spaced) {
            return Some(hash);
        }
    }
    None
}

/// `text` as a digest, if it is one: 64 hex digits and nothing else.
fn sha256_digest(text: &str) -> Option<String> {
    (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}

/// Unpacks the archive. `tar` reads both shapes: the gzipped tarball every
/// other platform gets, and the zip on Windows, where `tar` is bsdtar.
fn unpack(archive: &Path, into: &Path) -> Result<(), LpmError> {
    let status = Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()
        .map_err(|e| LpmError::Unavailable(format!("cannot run tar: {e}")))?;
    if status.success() {
        return Ok(());
    }
    Err(LpmError::Unavailable(format!(
        "cannot unpack {}",
        archive.display()
    )))
}

/// Looks for `name` in `dir` and one level under it, which covers an archive
/// that wraps its contents in a directory.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if entry.path().is_dir() {
            let nested = entry.path().join(name);
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    None
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
const fn make_executable(_path: &Path) {}

// ---------------------------------------------------------------------------
// The host target
// ---------------------------------------------------------------------------

/// The target triple the registry names this machine by.
#[must_use]
pub fn host_target() -> &'static str {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    match (os, arch) {
        ("windows", "aarch64") => "windows-aarch64",
        ("windows", _) => "windows-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", _) => "macos-x86_64",
        (_, "aarch64") => "linux-aarch64",
        _ => "linux-x86_64",
    }
}

// ---------------------------------------------------------------------------
// Reading what lpm reports
// ---------------------------------------------------------------------------

/// Reads the `packages` array out of an `lpm --json` report.
fn parse_packages(text: &str) -> Result<Vec<Package>, LpmError> {
    let report = json::parse(text).map_err(LpmError::BadOutput)?;
    let schema = report
        .get("schema")
        .and_then(json::Value::as_number)
        .ok_or_else(|| LpmError::BadOutput(String::from("no schema number")))?;
    if schema as i64 != 1 {
        return Err(LpmError::BadOutput(format!(
            "schema {schema} is newer than this candela reads. Update candela"
        )));
    }
    let entries = report
        .get("packages")
        .and_then(json::Value::as_array)
        .ok_or_else(|| LpmError::BadOutput(String::from("no packages array")))?;

    let mut packages = Vec::with_capacity(entries.len());
    for entry in entries {
        let field = |key: &str| {
            entry
                .get(key)
                .and_then(json::Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| LpmError::BadOutput(format!("a package has no {key}")))
        };
        packages.push(Package {
            name: field("name")?,
            version: field("version")?,
            platform: field("platform")?,
            target: field("target")?,
            dir: PathBuf::from(field("dir")?),
        });
    }
    Ok(packages)
}

/// Just enough JSON to read what `lpm` reports.
///
/// The shape is fixed and small, so this reads it directly rather than pulling
/// a parser and its derive machinery into a toolchain whose own budget is the
/// point.
mod json {
    use std::collections::BTreeMap;

    #[derive(Debug)]
    pub enum Value {
        Null,
        /// A boolean. Which one it was is not recorded: nothing candela reads
        /// out of a report is a boolean, and the variant is here so a report
        /// that carries one elsewhere still parses.
        Bool,
        Number(f64),
        String(String),
        Array(Vec<Self>),
        Object(BTreeMap<String, Self>),
    }

    impl Value {
        pub fn get(&self, key: &str) -> Option<&Self> {
            match self {
                Self::Object(map) => map.get(key),
                _ => None,
            }
        }

        pub fn as_str(&self) -> Option<&str> {
            match self {
                Self::String(s) => Some(s),
                _ => None,
            }
        }

        pub const fn as_number(&self) -> Option<f64> {
            match self {
                Self::Number(n) => Some(*n),
                _ => None,
            }
        }

        pub fn as_array(&self) -> Option<&[Self]> {
            match self {
                Self::Array(items) => Some(items),
                _ => None,
            }
        }
    }

    /// Parses one JSON document.
    ///
    /// # Errors
    ///
    /// Returns a one-line message naming what was wrong.
    pub fn parse(text: &str) -> Result<Value, String> {
        let bytes = text.as_bytes();
        let mut at = 0;
        let value = parse_value(bytes, &mut at)?;
        skip_space(bytes, &mut at);
        if at != bytes.len() {
            return Err(format!("trailing text at byte {at}"));
        }
        Ok(value)
    }

    const fn skip_space(bytes: &[u8], at: &mut usize) {
        while *at < bytes.len() && bytes[*at].is_ascii_whitespace() {
            *at += 1;
        }
    }

    fn parse_value(bytes: &[u8], at: &mut usize) -> Result<Value, String> {
        skip_space(bytes, at);
        match bytes.get(*at) {
            None => Err(String::from("the report is empty")),
            Some(b'{') => parse_object(bytes, at),
            Some(b'[') => parse_array(bytes, at),
            Some(b'"') => parse_string(bytes, at).map(Value::String),
            Some(b't') => literal(bytes, at, "true", Value::Bool),
            Some(b'f') => literal(bytes, at, "false", Value::Bool),
            Some(b'n') => literal(bytes, at, "null", Value::Null),
            Some(_) => parse_number(bytes, at),
        }
    }

    fn literal(bytes: &[u8], at: &mut usize, word: &str, value: Value) -> Result<Value, String> {
        if bytes[*at..].starts_with(word.as_bytes()) {
            *at += word.len();
            return Ok(value);
        }
        Err(format!("unexpected text at byte {at}"))
    }

    fn parse_object(bytes: &[u8], at: &mut usize) -> Result<Value, String> {
        *at += 1;
        let mut map = BTreeMap::new();
        skip_space(bytes, at);
        if bytes.get(*at) == Some(&b'}') {
            *at += 1;
            return Ok(Value::Object(map));
        }
        loop {
            skip_space(bytes, at);
            let key = parse_string(bytes, at)?;
            skip_space(bytes, at);
            if bytes.get(*at) != Some(&b':') {
                return Err(format!("expected a colon at byte {at}"));
            }
            *at += 1;
            let value = parse_value(bytes, at)?;
            map.insert(key, value);
            skip_space(bytes, at);
            match bytes.get(*at) {
                Some(b',') => *at += 1,
                Some(b'}') => {
                    *at += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err(format!("expected a comma or a closing brace at byte {at}")),
            }
        }
    }

    fn parse_array(bytes: &[u8], at: &mut usize) -> Result<Value, String> {
        *at += 1;
        let mut items = Vec::new();
        skip_space(bytes, at);
        if bytes.get(*at) == Some(&b']') {
            *at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(parse_value(bytes, at)?);
            skip_space(bytes, at);
            match bytes.get(*at) {
                Some(b',') => *at += 1,
                Some(b']') => {
                    *at += 1;
                    return Ok(Value::Array(items));
                }
                _ => {
                    return Err(format!(
                        "expected a comma or a closing bracket at byte {at}"
                    ));
                }
            }
        }
    }

    fn parse_string(bytes: &[u8], at: &mut usize) -> Result<String, String> {
        if bytes.get(*at) != Some(&b'"') {
            return Err(format!("expected a string at byte {at}"));
        }
        *at += 1;
        let mut out = String::new();
        loop {
            let byte = *bytes
                .get(*at)
                .ok_or_else(|| String::from("a string never ends"))?;
            *at += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = *bytes
                        .get(*at)
                        .ok_or_else(|| String::from("an escape never ends"))?;
                    *at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(parse_escape(bytes, at)?),
                        other => {
                            return Err(format!("unknown escape \\{}", other as char));
                        }
                    }
                }
                _ => {
                    // Everything else goes through as written, including
                    // multi-byte UTF-8, whose continuation bytes take this
                    // branch one at a time.
                    let start = *at - 1;
                    let width = utf8_width(byte);
                    let end = (start + width).min(bytes.len());
                    out.push_str(
                        std::str::from_utf8(&bytes[start..end])
                            .map_err(|_| String::from("a string is not valid UTF-8"))?,
                    );
                    *at = end;
                }
            }
        }
    }

    /// Reads a `\uXXXX` escape, joining a surrogate pair when one follows.
    fn parse_escape(bytes: &[u8], at: &mut usize) -> Result<char, String> {
        let first = hex4(bytes, at)?;
        if (0xD800..0xDC00).contains(&first) {
            if bytes.get(*at) == Some(&b'\\') && bytes.get(*at + 1) == Some(&b'u') {
                *at += 2;
                let second = hex4(bytes, at)?;
                let combined =
                    0x1_0000 + ((first - 0xD800) << 10) + (second.wrapping_sub(0xDC00) & 0x3FF);
                return char::from_u32(combined).ok_or_else(|| String::from("bad escape"));
            }
            return Err(String::from("a lone surrogate escape"));
        }
        char::from_u32(first).ok_or_else(|| String::from("bad escape"))
    }

    fn hex4(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
        let digits = bytes
            .get(*at..*at + 4)
            .ok_or_else(|| String::from("a short escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| String::from("a bad escape"))?;
        let value = u32::from_str_radix(text, 16).map_err(|_| String::from("a bad escape"))?;
        *at += 4;
        Ok(value)
    }

    const fn utf8_width(byte: u8) -> usize {
        match byte {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            _ => 4,
        }
    }

    fn parse_number(bytes: &[u8], at: &mut usize) -> Result<Value, String> {
        let start = *at;
        while let Some(byte) = bytes.get(*at) {
            if byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E') {
                *at += 1;
            } else {
                break;
            }
        }
        std::str::from_utf8(&bytes[start..*at])
            .ok()
            .and_then(|text| text.parse().ok())
            .map(Value::Number)
            .ok_or_else(|| format!("expected a value at byte {start}"))
    }
}

#[cfg(test)]
mod tests {
    use super::hashing_tools;
    use super::json;
    use super::last_location_tag;
    use super::parse_packages;
    use super::parse_version;
    use super::recorded_sha;
    use super::sha256_in;
    use super::sha256_of;
    use std::path::Path;

    const REPORT: &str = r#"{"schema":1,"lock":"/p/candela.lock","packages":[
        {"name":"shapes","version":"1.2.3","platform":"candela","target":"any",
         "dir":"/cache/shapes/1.2.3/any","files":["candela.toml","shapes.cdl"],
         "dependencies":{"geom":"0.3.1"}}]}"#;

    #[test]
    fn a_report_reads_into_packages() {
        let packages = parse_packages(REPORT).unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "shapes");
        assert_eq!(packages[0].version, "1.2.3");
        assert_eq!(packages[0].platform, "candela");
        assert_eq!(packages[0].target, "any");
        assert_eq!(packages[0].dir, Path::new("/cache/shapes/1.2.3/any"));
    }

    #[test]
    fn an_empty_package_list_is_fine() {
        let packages = parse_packages(r#"{"schema":1,"packages":[]}"#).unwrap();
        assert!(packages.is_empty());
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let err = parse_packages(r#"{"schema":2,"packages":[]}"#).unwrap_err();
        assert!(err.to_string().contains("schema"), "{err}");
    }

    #[test]
    fn a_package_missing_a_field_is_refused() {
        let err = parse_packages(r#"{"schema":1,"packages":[{"name":"a"}]}"#).unwrap_err();
        assert!(err.to_string().contains("version"), "{err}");
    }

    #[test]
    fn output_that_is_not_json_is_refused() {
        assert!(parse_packages("lpm: something went wrong").is_err());
        assert!(parse_packages("").is_err());
    }

    /// Escapes and multi-byte text both survive a round trip. A package
    /// directory can sit under a path in any script, and JSON spells anything
    /// past the basic plane as a surrogate pair.
    #[test]
    fn strings_carry_escapes_and_wide_characters() {
        let accented = "caf\u{e9}";
        let wide = "\u{1f600}";

        let escaped = json::parse(r#"{"a":"line\nbreak caf\u00e9 \ud83d\ude00"}"#).unwrap();
        assert_eq!(
            escaped.get("a").and_then(json::Value::as_str),
            Some(format!("line\nbreak {accented} {wide}").as_str())
        );

        // The same text as bytes rather than escapes, which is the shape a
        // report from the client arrives in.
        let literal = json::parse(&format!("{{\"a\":\"{accented} {wide}\"}}")).unwrap();
        assert_eq!(
            literal.get("a").and_then(json::Value::as_str),
            Some(format!("{accented} {wide}").as_str())
        );
    }

    #[test]
    fn nesting_and_the_other_value_shapes_parse() {
        let value = json::parse(r#"{"a":[1,-2.5,1e3,true,false,null,{"b":[]}]," c ":{}}"#).unwrap();
        let items = value.get("a").and_then(json::Value::as_array).unwrap();
        assert_eq!(items.len(), 7);
        assert_eq!(items[0].as_number(), Some(1.0));
        assert_eq!(items[1].as_number(), Some(-2.5));
        assert_eq!(items[2].as_number(), Some(1000.0));
    }

    #[test]
    fn malformed_json_says_so_instead_of_panicking() {
        for text in [
            "{",
            "[",
            "{\"a\"}",
            "{\"a\":}",
            "{\"a\":1,}",
            "\"unterminated",
            "{\"a\":1} trailing",
            "tru",
            "{\"a\":\\}",
        ] {
            assert!(json::parse(text).is_err(), "{text} parsed");
        }
    }

    #[test]
    fn a_version_line_reads_the_number() {
        assert_eq!(
            parse_version("lpm version 0.2.1 (built 2026-09-01)\n").as_deref(),
            Some("0.2.1")
        );
        assert_eq!(
            parse_version("lpm version v1.0.0\n").as_deref(),
            Some("1.0.0")
        );
    }

    #[test]
    fn anything_else_is_not_a_version() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("lpm 0.2.1"), None);
        assert_eq!(parse_version("cargo version 1.0.0"), None);
        assert_eq!(parse_version("lpm version unknown"), None);
    }

    #[test]
    fn the_tag_comes_from_the_last_location_header() {
        let headers = "HTTP/2 302\r\n\
             location: https://github.com/lumen-fx/registry/releases/tag/v0.3.0\r\n";
        assert_eq!(last_location_tag(headers).as_deref(), Some("v0.3.0"));
        assert_eq!(last_location_tag("HTTP/2 200\r\n"), None);
    }

    #[test]
    fn a_checksum_line_is_found_by_its_filename() {
        let sums = "abc123  lpm_0.1.0_linux_amd64.tar.gz\ndef456 *lpm_0.1.0_darwin_arm64.tar.gz\n";
        assert_eq!(
            recorded_sha(sums, "lpm_0.1.0_linux_amd64.tar.gz").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            recorded_sha(sums, "lpm_0.1.0_darwin_arm64.tar.gz").as_deref(),
            Some("def456")
        );
        assert_eq!(recorded_sha(sums, "lpm_0.1.0_windows_amd64.zip"), None);
    }

    /// The digest of the five bytes `hello`, which every fixture below is a
    /// tool's report of.
    const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn a_digest_is_read_out_of_every_tools_own_shape() {
        // sha256sum, given a path with nothing to escape.
        assert_eq!(
            sha256_in(&format!("{HELLO}  /tmp/lpm_0.2.0_linux_amd64.tar.gz\n")).as_deref(),
            Some(HELLO)
        );

        // sha256sum, given a Windows path. GNU coreutils marks the line with a
        // leading backslash and doubles the ones in the name.
        assert_eq!(
            sha256_in(&format!(
                "\\{HELLO}  C:\\\\Temp\\\\candela-lpm-1\\\\lpm_0.2.0_windows_amd64.zip\n"
            ))
            .as_deref(),
            Some(HELLO)
        );

        // shasum, which marks a binary read with an asterisk.
        assert_eq!(
            sha256_in(&format!("{HELLO} *lpm_0.2.0_darwin_arm64.tar.gz\n")).as_deref(),
            Some(HELLO)
        );

        // certutil on an older build, which spaces the digest into byte pairs.
        let spaced = HELLO
            .as_bytes()
            .chunks(2)
            .map(|pair| String::from_utf8_lossy(pair).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            sha256_in(&format!(
                "SHA256 hash of C:\\Temp\\lpm.zip:\r\n{spaced}\r\nCertUtil: -hashfile command completed successfully.\r\n"
            ))
            .as_deref(),
            Some(HELLO)
        );

        // certutil on a newer build, which does not.
        assert_eq!(
            sha256_in(&format!(
                "SHA256 hash of C:\\Temp\\lpm.zip:\r\n{HELLO}\r\nCertUtil: -hashfile command completed successfully.\r\n"
            ))
            .as_deref(),
            Some(HELLO)
        );
    }

    #[test]
    fn an_uppercase_digest_reads_as_the_same_digest() {
        assert_eq!(
            sha256_in(&format!("{}  lpm.zip\n", HELLO.to_ascii_uppercase())).as_deref(),
            Some(HELLO)
        );
    }

    #[test]
    fn output_with_no_digest_in_it_is_no_answer() {
        assert_eq!(sha256_in(""), None);
        assert_eq!(
            sha256_in("sha256sum: lpm.zip: No such file or directory"),
            None
        );
        // 63 digits and 65 digits are both not a digest.
        assert_eq!(sha256_in(&HELLO[..63]), None);
        assert_eq!(sha256_in(&format!("{HELLO}0")), None);
        // Hex-looking but not hex.
        assert_eq!(sha256_in(&format!("{}g  lpm.zip", &HELLO[..63])), None);
    }

    /// Every platform asks the tool it is sure to have first, and keeps the
    /// other two to fall back on.
    #[test]
    fn the_hashing_tools_are_tried_in_the_platforms_own_order() {
        let tools = hashing_tools();
        let first = tools[0].0;
        if cfg!(windows) {
            assert_eq!(first, "certutil");
        } else {
            assert_eq!(first, "sha256sum");
        }
        let mut names: Vec<&str> = tools.iter().map(|(name, _, _)| *name).collect();
        names.sort_unstable();
        assert_eq!(names, ["certutil", "sha256sum", "shasum"]);
    }

    /// The whole walk, against the tool this machine actually has. A machine
    /// with none of the three cannot install the client at all, which is what
    /// a failure here would mean.
    #[test]
    fn a_file_is_hashed_with_whichever_tool_the_machine_has() {
        let dir = std::env::temp_dir().join(format!("candela_lpm_hash_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the scratch dir");
        let file = dir.join("empty");
        std::fs::write(&file, b"").expect("write the file to hash");
        assert_eq!(
            sha256_of(&file).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
