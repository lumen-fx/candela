//! The `tower_lsp::LanguageServer` implementation: wires document sync,
//! diagnostics, hover, completion, signature help, document symbols,
//! go-to-definition and the reference features to the analysis in
//! `crate::analysis` (a thin wrapper around candela's own compile), the
//! position-based features in `crate::ide`, and `crate::refs`.
//!
//! Diagnostics are worked out off the request path. Every open buffer is
//! checked as the root of a program, a moment after the last change to it or
//! to a file it imports, against the text of every open buffer rather than
//! what is saved. Each error is published on the file it is in. After a save,
//! a package that names a `check` command in its `candela.toml` has it run
//! for each open program that imports the package, and what it reports is
//! published beside the compiler's errors (see `crate::checks`).

use crate::analysis::{self, ProgramSummary};
use crate::checks::{self, Found};
use crate::hints;
use crate::ide::Ide;
use crate::line_index::{self, LineIndex};
use crate::refs::{self, Decl};
use crate::semantic;
use crate::symbols::{self, Declared};
use candela::Diagnostic as CdlDiagnostic;
use candela::PackageCheck;
use candela::compiler::imports::ImportResolver;
use dashmap::DashMap;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tower_lsp::jsonrpc::{Error as RpcError, ErrorCode, Result as RpcResult};
use tower_lsp::lsp_types::Diagnostic as LspDiagnostic;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

struct Document {
    text: String,
    /// The buffer's file, canonical when it exists on disk: the path an
    /// import of it resolves to.
    path: PathBuf,
    /// The last summary produced by a compile that got past the program's
    /// declarations, kept around so hover/completion/document-symbols/
    /// go-to-definition still work while the buffer is mid-edit and
    /// currently failing to compile, instead of going blank on every
    /// keystroke that introduces a temporary syntax error.
    last_good: Option<ProgramSummary>,
    /// Counts the changes that asked for a compile, so a compile that
    /// finishes after a newer one was asked for is dropped.
    generation: u64,
    /// The generation whose text was last compiled. While it equals
    /// `generation`, nothing the buffer reads has changed since, and a
    /// request answers from `last_good` instead of compiling again.
    compiled_generation: u64,
    /// The same for the package checks.
    check_generation: u64,
}

/// What the last checks of one open buffer, as the root of a program, found.
#[derive(Default)]
struct RootState {
    /// The files the compile read.
    files: Vec<PathBuf>,
    /// The compiler's diagnostics, by the file they are in.
    compiled: HashMap<Url, Vec<LspDiagnostic>>,
    /// The check commands of the packages the program imports.
    checks: Vec<PackageCheck>,
    /// What those commands reported, by file.
    checked: HashMap<Url, Vec<LspDiagnostic>>,
}

/// Where library imports read from, as the editor's settings give it, and
/// how diagnostics are worked out.
///
/// The client sends it as `initializationOptions` and again with every
/// `workspace/didChangeConfiguration`, either bare or under a `candela` key:
///
/// ```json
/// { "libPath": "/opt/candela/libs", "importRoots": { "drift": "/opt/drift/module" },
///   "diagnosticsDelay": 300, "packageChecks": true }
/// ```
///
/// `libPath` is the directory holding `std/`, what `CANDELA_LIB_PATH` names;
/// without it the standard library is looked up the way the `candela` command
/// does. Each `importRoots` entry makes `import "name";` read a package: a
/// directory is a package root, entered through the file its `candela.toml`
/// names or `src/main.cdl`; a `.cdl` file is the module itself.
/// `diagnosticsDelay` is how long after the last change a buffer is checked,
/// in milliseconds. `packageChecks` turns off the `check` commands packages
/// declare.
#[derive(Debug, Clone)]
pub struct ImportSettings {
    lib_path: Option<PathBuf>,
    roots: Vec<(String, PathBuf)>,
    delay: Duration,
    package_checks: bool,
}

/// How long after the last change a buffer is checked when the settings say
/// nothing.
const DEFAULT_DELAY: Duration = Duration::from_millis(250);

/// How long after a save the package checks run: a little longer than a
/// compile, since a save often comes in a burst ("save all").
const CHECK_DELAY: Duration = Duration::from_millis(400);

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            lib_path: None,
            roots: Vec::new(),
            delay: DEFAULT_DELAY,
            package_checks: true,
        }
    }
}

impl ImportSettings {
    /// Reads the settings out of a client's JSON. Missing or malformed keys
    /// are left out.
    #[must_use]
    pub fn from_json(value: &Value) -> Self {
        let value = value.get("candela").unwrap_or(value);
        let lib_path = value
            .get("libPath")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        let roots = value
            .get("importRoots")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(name, dir)| {
                        let dir = dir.as_str()?.trim();
                        (!dir.is_empty()).then(|| (name.clone(), PathBuf::from(dir)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let delay = value
            .get("diagnosticsDelay")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_DELAY, Duration::from_millis);
        let package_checks = value
            .get("packageChecks")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        Self {
            lib_path,
            roots,
            delay,
            package_checks,
        }
    }

    /// The resolver a compile of an open buffer uses.
    #[must_use]
    pub fn resolver(&self) -> ImportResolver {
        let mut resolver = ImportResolver::new();
        if let Some(dir) = &self.lib_path {
            resolver.set_lib_dir(dir.clone());
        }
        for (name, path) in &self.roots {
            if path.extension().is_some_and(|e| e == "cdl") {
                let dir = path.parent().map(PathBuf::from).unwrap_or_default();
                let entry = path.file_name().map(PathBuf::from).unwrap_or_default();
                resolver.add_root(name, dir, entry);
            } else {
                resolver.add_package(name, path.clone());
            }
        }
        resolver
    }
}

pub struct Backend {
    inner: Arc<Inner>,
}

struct Inner {
    client: Client,
    docs: DashMap<Url, Document>,
    /// Where library imports read from. A resolver is built from it for
    /// each compile; the resolver itself does not cross threads.
    settings: RwLock<ImportSettings>,
    roots: Mutex<HashMap<Url, RootState>>,
    /// The files diagnostics were last published for with any in them, so a
    /// file whose last one went away is cleared.
    published: Mutex<HashSet<Url>>,
    /// Held while a package check runs: one runs at a time, and a check
    /// waiting for it that a newer one replaced by then does not run.
    checking: tokio::sync::Mutex<()>,
    /// The editor's workspace folders, canonical.
    folders: RwLock<Vec<PathBuf>>,
    /// What each workspace file declares, as of its modification time, so
    /// a symbol search reads only the files that changed since the last.
    declared: Mutex<HashMap<PathBuf, (std::time::SystemTime, Vec<Declared>)>>,
    /// Whether the client asks again for semantic tokens and inlay hints
    /// when told to, which the server does after a check brings in new
    /// declarations.
    refresh_tokens: std::sync::atomic::AtomicBool,
    refresh_hints: std::sync::atomic::AtomicBool,
}

/// The references to one declaration found in one file.
struct FileRefs {
    path: PathBuf,
    text: String,
    ranges: Vec<(usize, usize)>,
}

/// What a search for references read.
struct RefSearch {
    found: Vec<FileRefs>,
    /// Files that name the declaration but did not compile, so their
    /// references could not be told apart.
    unread: Vec<PathBuf>,
}

/// The most `.cdl` files a search for references reads from a package, or
/// a symbol search from the workspace.
const MAX_PACKAGE_FILES: usize = 2000;

/// The most symbols a workspace symbol search answers with.
const MAX_SYMBOLS: usize = 500;

impl Backend {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                docs: DashMap::new(),
                settings: RwLock::new(ImportSettings::default()),
                roots: Mutex::new(HashMap::new()),
                published: Mutex::new(HashSet::new()),
                checking: tokio::sync::Mutex::new(()),
                folders: RwLock::new(Vec::new()),
                declared: Mutex::new(HashMap::new()),
                refresh_tokens: std::sync::atomic::AtomicBool::new(false),
                refresh_hints: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }
}

/// The buffer's real filesystem path, so `import "..."` resolves the same way
/// it would for the `candela` CLI. Falls back to the raw URI path for
/// non-`file://` URIs (untitled buffers, etc.).
fn uri_to_path(uri: &Url) -> String {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_else(|| uri.path().to_owned())
}

/// The path an import of the file reaches: canonical when the file exists.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

impl Inner {
    fn settings(&self) -> ImportSettings {
        self.settings
            .read()
            .map_or_else(|_| ImportSettings::default(), |s| s.clone())
    }

    /// Every other open buffer's file and text, which a compile of `root`
    /// reads in place of what is saved.
    fn overlays(&self, root: &Url) -> Vec<(PathBuf, String)> {
        self.docs
            .iter()
            .filter(|doc| doc.key() != root)
            .map(|doc| (doc.path.clone(), doc.text.clone()))
            .collect()
    }

    /// The resolver for a compile of `root`.
    fn resolver_for(&self, root: &Url) -> ImportResolver {
        resolver_with(&self.settings(), self.overlays(root))
    }

    fn analyze(&self, uri: &Url, text: &str) -> analysis::AnalysisOutcome {
        analysis::analyze(text, &uri_to_path(uri), &self.resolver_for(uri))
    }

    /// The summary of the document's current text. When nothing it reads
    /// changed since its last compile, that compile's summary; otherwise the
    /// text is compiled again, refreshing the cached `last_good` summary on
    /// success and falling back to it on failure (`None` if the document
    /// has never compiled).
    fn current_or_cached_summary(&self, uri: &Url) -> Option<ProgramSummary> {
        let (text, generation) = {
            let doc = self.docs.get(uri)?;
            if doc.compiled_generation == doc.generation {
                return doc.last_good.clone();
            }
            (doc.text.clone(), doc.generation)
        };
        let outcome = self.analyze(uri, &text);
        let mut doc = self.docs.get_mut(uri)?;
        if doc.generation == generation {
            doc.compiled_generation = generation;
        }
        if let Some(summary) = outcome.summary {
            doc.last_good = Some(summary.clone());
            Some(summary)
        } else {
            doc.last_good.clone()
        }
    }

    /// The summary completion and signature help read: the last good
    /// compile's, whatever has been typed since. Both read what the cursor
    /// is on from the current text, and the debounced check after the change
    /// refreshes the summary, so a keystroke never waits for a compile of
    /// the whole program. A document that has never compiled is compiled now.
    fn typing_summary(&self, uri: &Url) -> Option<ProgramSummary> {
        if let Some(summary) = self.docs.get(uri)?.last_good.clone() {
            return Some(summary);
        }
        self.current_or_cached_summary(uri)
    }

    fn doc_text(&self, uri: &Url) -> Option<String> {
        self.docs.get(uri).map(|d| d.text.clone())
    }

    /// The text, summary and byte offset a position request works on.
    fn request(&self, uri: &Url, position: Position) -> Option<(String, ProgramSummary, usize)> {
        let text = self.doc_text(uri)?;
        let summary = self.current_or_cached_summary(uri)?;
        let offset = line_index::position_to_offset(&text, position);
        Some((text, summary, offset))
    }

    /// The open buffers whose last compile read `path`, besides `except`.
    fn dependents(&self, path: &Path, except: &Url) -> Vec<Url> {
        let Ok(roots) = self.roots.lock() else {
            return Vec::new();
        };
        roots
            .iter()
            .filter(|(uri, state)| *uri != except && state.files.iter().any(|f| f == path))
            .map(|(uri, _)| uri.clone())
            .collect()
    }

    /// Checks `uri` a moment from now, unless it changes again first.
    fn schedule_compile(self: &Arc<Self>, uri: &Url, delay: Duration) {
        let Some(generation) = self.docs.get_mut(uri).map(|mut d| {
            d.generation += 1;
            d.generation
        }) else {
            return;
        };
        let inner = Arc::clone(self);
        let uri = uri.clone();
        tokio::spawn(async move {
            // A check asked for at once (open, save) runs whatever came
            // after it, so the buffer has a summary to fall back on; one
            // asked for after typing waits and gives way to a newer one.
            let debounced = !delay.is_zero();
            if debounced {
                tokio::time::sleep(delay).await;
            }
            inner.compile(&uri, generation, debounced).await;
        });
    }

    /// Runs the package checks for `uri` a moment from now.
    fn schedule_package_checks(self: &Arc<Self>, uri: &Url, delay: Duration) {
        let Some(generation) = self.docs.get_mut(uri).map(|mut d| {
            d.check_generation += 1;
            d.check_generation
        }) else {
            return;
        };
        let inner = Arc::clone(self);
        let uri = uri.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            inner.run_package_checks(&uri, generation).await;
        });
    }

    fn generation(&self, uri: &Url) -> Option<u64> {
        self.docs.get(uri).map(|d| d.generation)
    }

    async fn compile(self: &Arc<Self>, uri: &Url, generation: u64, debounced: bool) {
        if debounced && self.generation(uri) != Some(generation) {
            return;
        }
        let Some(text) = self.doc_text(uri) else {
            return;
        };
        let path = uri_to_path(uri);
        // The resolver is built on the thread that compiles: it does not
        // cross threads. A check asked for at once runs before this task
        // yields, so the requests read after it see its summary.
        let settings = self.settings();
        let overlays = self.overlays(uri);
        let outcome = if debounced {
            let source = text.clone();
            let compile_path = path.clone();
            let Ok(outcome) = tokio::task::spawn_blocking(move || {
                analysis::analyze(&source, &compile_path, &resolver_with(&settings, overlays))
            })
            .await
            else {
                return;
            };
            outcome
        } else {
            analysis::analyze(&text, &path, &resolver_with(&settings, overlays))
        };
        // An older text's summary still serves hover and completion while
        // the newer one is being checked.
        let mut refreshed = false;
        if let Some(mut doc) = self.docs.get_mut(uri) {
            let current = doc.generation == generation;
            if let Some(summary) = &outcome.summary
                && (current || doc.last_good.is_none())
            {
                doc.last_good = Some(summary.clone());
                refreshed = true;
            }
            if current {
                doc.compiled_generation = generation;
            }
        }
        // Tokens and hints were worked out against the summary before; the
        // client asks again with this one.
        let load = |flag: &std::sync::atomic::AtomicBool| {
            refreshed && flag.load(std::sync::atomic::Ordering::Relaxed)
        };
        let (tokens, hints) = (load(&self.refresh_tokens), load(&self.refresh_hints));
        if tokens || hints {
            let client = self.client.clone();
            tokio::spawn(async move {
                if tokens {
                    let _ = client.semantic_tokens_refresh().await;
                }
                if hints {
                    let _ = client.inlay_hint_refresh().await;
                }
            });
        }
        if self.generation(uri) != Some(generation) {
            return;
        }
        let by_file = self.diagnostics_by_file(uri, &path, &text, &outcome);
        let files: Option<Vec<PathBuf>> = outcome.summary.as_ref().map(|s| {
            s.sources
                .iter()
                .map(|f| canonical(Path::new(&f.path)))
                .collect()
        });
        let checks = outcome
            .summary
            .as_ref()
            .map(|s| package_checks(s, Path::new(&path)));
        let mut affected: HashSet<Url> = by_file.keys().cloned().collect();
        affected.insert(uri.clone());
        if let Ok(mut roots) = self.roots.lock() {
            // Checked again under the lock: a buffer closed while it compiled
            // leaves no state behind.
            if self.generation(uri) != Some(generation) {
                return;
            }
            let state = roots.entry(uri.clone()).or_default();
            affected.extend(state.compiled.keys().cloned());
            state.compiled = by_file;
            // A compile that stopped at a declaration read too little to say
            // which files and packages the program has; the last one stands.
            if let Some(files) = files {
                state.files = files;
            }
            if let Some(checks) = checks {
                state.checks = checks;
            }
        }
        self.publish(affected, Some(uri)).await;
    }

    /// Whether another open program reads `uri`'s file while `uri`'s does
    /// not read that program's: its checks then see all `uri`'s does, so
    /// running them for `uri` too would only repeat their findings.
    fn covered(&self, uri: &Url) -> bool {
        let Some(path) = self.docs.get(uri).map(|d| d.path.clone()) else {
            return false;
        };
        let Ok(roots) = self.roots.lock() else {
            return false;
        };
        let own = roots.get(uri).map(|s| s.files.as_slice()).unwrap_or(&[]);
        roots.iter().any(|(other, state)| {
            other != uri
                && state.files.contains(&path)
                && self.docs.get(other).is_some_and(|d| !own.contains(&d.path))
        })
    }

    async fn run_package_checks(self: &Arc<Self>, uri: &Url, generation: u64) {
        let stale = |inner: &Self| {
            inner.docs.get(uri).map(|d| d.check_generation) != Some(generation)
                || !inner.settings().package_checks
        };
        if stale(self) {
            return;
        }
        // One check at a time: a package's check is a program of its own
        // (`drift check` is a second or more of CPU), and a save of a file
        // many open programs read asks each of them for one.
        let _running = self.checking.lock().await;
        if stale(self) {
            return;
        }
        let checks = if self.covered(uri) {
            Vec::new()
        } else {
            self.roots
                .lock()
                .ok()
                .and_then(|roots| roots.get(uri).map(|s| s.checks.clone()))
                .unwrap_or_default()
        };
        let cwd = uri
            .to_file_path()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let mut by_file: HashMap<Url, Vec<LspDiagnostic>> = HashMap::new();
        for check in &checks {
            match checks::run(check, cwd.as_deref()).await {
                Ok(found) => {
                    for (file, diagnostic) in found
                        .into_iter()
                        .filter_map(|f| self.to_lsp_found(f, check))
                    {
                        by_file.entry(file).or_default().push(diagnostic);
                    }
                }
                Err(why) => {
                    self.client
                        .log_message(
                            MessageType::WARNING,
                            format!(
                                "candela-lsp: the {} package's check did not run: {why}",
                                check.package
                            ),
                        )
                        .await;
                }
            }
        }
        if self.docs.get(uri).map(|d| d.check_generation) != Some(generation) {
            return;
        }
        let mut affected: HashSet<Url> = by_file.keys().cloned().collect();
        if let Ok(mut roots) = self.roots.lock() {
            if self.docs.get(uri).map(|d| d.check_generation) != Some(generation) {
                return;
            }
            let state = roots.entry(uri.clone()).or_default();
            affected.extend(state.checked.keys().cloned());
            state.checked = by_file;
        }
        self.publish(affected, None).await;
    }

    /// One diagnostic a package check reported, placed in the editor.
    fn to_lsp_found(&self, found: Found, check: &PackageCheck) -> Option<(Url, LspDiagnostic)> {
        let uri = Url::from_file_path(&found.place.file).ok()?;
        let text = self.file_text(&found.place.file);
        let range = found.range(text.as_deref());
        let related = found
            .related
            .iter()
            .filter_map(|r| {
                let uri = Url::from_file_path(&r.place.file).ok()?;
                let text = self.file_text(&r.place.file);
                Some(DiagnosticRelatedInformation {
                    location: Location::new(uri, r.range(text.as_deref())),
                    message: r.message.clone(),
                })
            })
            .collect::<Vec<_>>();
        Some((
            uri,
            LspDiagnostic {
                range,
                severity: Some(found.severity),
                code: found.code.clone().map(NumberOrString::String),
                source: Some(check.package.clone()),
                message: found.message,
                related_information: (!related.is_empty()).then_some(related),
                ..LspDiagnostic::default()
            },
        ))
    }

    /// The text of the file at `path`: its open buffer's, or what is on disk.
    fn file_text(&self, path: &Path) -> Option<String> {
        let path = canonical(path);
        self.docs
            .iter()
            .find(|d| d.path == path)
            .map(|d| d.text.clone())
            .or_else(|| std::fs::read_to_string(&path).ok())
    }

    /// The compile's errors and warnings, each on the file it is in.
    fn diagnostics_by_file(
        &self,
        root: &Url,
        root_path: &str,
        root_text: &str,
        outcome: &analysis::AnalysisOutcome,
    ) -> HashMap<Url, Vec<LspDiagnostic>> {
        let texts: HashMap<&str, &str> = outcome
            .summary
            .iter()
            .flat_map(|s| s.sources.iter().map(|f| (f.path.as_str(), f.text.as_str())))
            .collect();
        let mut by_file: HashMap<Url, Vec<LspDiagnostic>> = HashMap::new();
        let root_index = LineIndex::new(root_text);
        let mut place = |d: &CdlDiagnostic, severity: DiagnosticSeverity| {
            let (uri, diagnostic) = self.place(d, severity, root, root_path, &root_index, &texts);
            by_file.entry(uri).or_default().push(diagnostic);
        };
        for d in &outcome.errors {
            place(d, DiagnosticSeverity::ERROR);
        }
        for d in &outcome.warnings {
            place(d, DiagnosticSeverity::WARNING);
        }
        if let Some(summary) = &outcome.summary {
            for (start, end, message, code) in crate::ide::attribute_warnings(root_text, summary) {
                by_file
                    .entry(root.clone())
                    .or_default()
                    .push(LspDiagnostic {
                        range: root_index.range(start, end),
                        severity: Some(DiagnosticSeverity::WARNING),
                        code: Some(NumberOrString::String(code.to_owned())),
                        source: Some("candela".to_owned()),
                        message,
                        ..LspDiagnostic::default()
                    });
            }
        }
        by_file
    }

    /// Where a compiler diagnostic shows: on the file it names, at its span.
    /// A file with no path of its own (a module the compiler carries) shows
    /// at the top of the buffer being checked, naming it.
    fn place(
        &self,
        d: &CdlDiagnostic,
        severity: DiagnosticSeverity,
        root: &Url,
        root_path: &str,
        root_index: &LineIndex<'_>,
        texts: &HashMap<&str, &str>,
    ) -> (Url, LspDiagnostic) {
        let locate = |filename: &str, span: &std::ops::Range<usize>| -> Option<(Url, Range)> {
            if filename == root_path || filename.is_empty() {
                return Some((root.clone(), root_index.range(span.start, span.end)));
            }
            let uri = Url::from_file_path(filename).ok()?;
            let text = texts
                .get(filename)
                .map(|t| (*t).to_owned())
                .or_else(|| self.file_text(Path::new(filename)))?;
            Some((uri, range_of(&text, span.start, span.end)))
        };
        let related: Vec<DiagnosticRelatedInformation> = d
            .related
            .iter()
            .filter_map(|r| {
                let (uri, range) = locate(&r.filename, &r.span)?;
                Some(DiagnosticRelatedInformation {
                    location: Location::new(uri, range),
                    message: r.message.clone(),
                })
            })
            .collect();
        let (uri, range, message) = match locate(&d.filename, &d.span) {
            Some((uri, range)) => (uri, range, d.message.clone()),
            None => (
                root.clone(),
                Range::new(Position::new(0, 0), Position::new(0, 0)),
                format!("{} (in {})", d.message, d.filename),
            ),
        };
        (
            uri,
            LspDiagnostic {
                range,
                severity: Some(severity),
                code: Some(NumberOrString::String(d.code.clone())),
                code_description: None,
                source: Some("candela".to_owned()),
                message,
                related_information: (!related.is_empty()).then_some(related),
                tags: None,
                data: None,
            },
        )
    }

    /// Publishes what every open program found in each of `files`, the same
    /// diagnostic once however many programs found it. A file that had none
    /// and still has none is skipped, except `checked`, the buffer whose
    /// check this is, which always hears back.
    async fn publish(&self, files: HashSet<Url>, checked: Option<&Url>) {
        let mut out: Vec<(Url, Vec<LspDiagnostic>)> = Vec::new();
        if let Ok(roots) = self.roots.lock() {
            for file in files {
                let mut list: Vec<LspDiagnostic> = Vec::new();
                for state in roots.values() {
                    for found in [state.compiled.get(&file), state.checked.get(&file)]
                        .into_iter()
                        .flatten()
                    {
                        for d in found {
                            if !list
                                .iter()
                                .any(|seen| seen.range == d.range && seen.message == d.message)
                            {
                                list.push(d.clone());
                            }
                        }
                    }
                }
                list.sort_by_key(|d| (d.range.start.line, d.range.start.character));
                out.push((file, list));
            }
        }
        for (file, list) in out {
            let had = self
                .published
                .lock()
                .map(|mut p| {
                    if list.is_empty() {
                        p.remove(&file)
                    } else {
                        !p.insert(file.clone())
                    }
                })
                .unwrap_or(true);
            if list.is_empty() && !had && checked != Some(&file) {
                continue;
            }
            self.client.publish_diagnostics(file, list, None).await;
        }
    }

    /// Checks `uri` again, and every open program that reads it.
    fn changed(self: &Arc<Self>, uri: &Url, delay: Duration) {
        self.schedule_compile(uri, delay);
        let Some(path) = self.docs.get(uri).map(|d| d.path.clone()) else {
            return;
        };
        for dependent in self.dependents(&path, uri) {
            self.schedule_compile(&dependent, delay);
        }
    }

    /// The canonical path of an open document's file.
    fn doc_path(&self, uri: &Url) -> Option<PathBuf> {
        self.docs.get(uri).map(|d| d.path.clone())
    }

    /// The declaration under the cursor of an open document, and the range
    /// of the name the cursor is on.
    fn declaration_at(
        &self,
        uri: &Url,
        position: Position,
    ) -> Option<(Decl, (usize, usize), String, ProgramSummary)> {
        let (text, summary, offset) = self.request(uri, position)?;
        let path = self.doc_path(uri)?;
        let (decl, under) = refs::declaration(&text, &summary, &path, offset)?;
        Some((decl, under, text, summary))
    }

    /// The files a reference to `decl` can be in: a local's own file, or
    /// for anything else the files of the program `summary` describes, the
    /// open buffers, and every `.cdl` file of the package (or workspace
    /// folder) holding the declaration.
    fn reference_files(&self, decl: &Decl, summary: &ProgramSummary) -> Vec<PathBuf> {
        let mut files = vec![decl.file.clone()];
        if decl.local {
            return files;
        }
        let mut add = |path: PathBuf| {
            if !files.contains(&path) {
                files.push(path);
            }
        };
        for source in summary.sources.iter().skip(1).filter(|s| !s.is_std) {
            add(canonical(Path::new(&source.path)));
        }
        for doc in self.docs.iter() {
            add(doc.path.clone());
        }
        let folders = self.folders.read().map(|f| f.clone()).unwrap_or_default();
        if let Some(root) = package_root(&decl.file, &folders) {
            for path in cdl_files(&root, MAX_PACKAGE_FILES) {
                add(path);
            }
        }
        if let Some(path) = summary
            .sources
            .first()
            .map(|s| canonical(Path::new(&s.path)))
        {
            add(path);
        }
        files
    }

    /// Every reference to `decl` in the files that can hold one. An open
    /// buffer is read as it stands; any other file that spells the name is
    /// compiled as the root of a program, against the open buffers' text.
    async fn find_references(&self, decl: &Decl, summary: &ProgramSummary) -> RefSearch {
        let files = self.reference_files(decl, summary);
        let mut found = Vec::new();
        let mut closed = Vec::new();
        for path in files {
            let open = self
                .docs
                .iter()
                .find(|d| d.path == path)
                .map(|d| d.key().clone());
            match open {
                Some(uri) => {
                    let (Some(text), Some(summary)) =
                        (self.doc_text(&uri), self.current_or_cached_summary(&uri))
                    else {
                        continue;
                    };
                    let ranges = refs::occurrences(&text, &summary, &path, decl);
                    if !ranges.is_empty() {
                        found.push(FileRefs { path, text, ranges });
                    }
                }
                None => closed.push(path),
            }
        }
        let settings = self.settings();
        let overlays: Vec<(PathBuf, String)> = self
            .docs
            .iter()
            .map(|d| (d.path.clone(), d.text.clone()))
            .collect();
        let decl = decl.clone();
        let mut unread = Vec::new();
        // The compiles run off the async threads; the resolver is built on
        // the thread that uses it.
        let read = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            let mut unread = Vec::new();
            for path in closed {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if !text.contains(decl.name.as_str()) {
                    continue;
                }
                let resolver = resolver_with(&settings, overlays.clone());
                let outcome = analysis::analyze(&text, &path.to_string_lossy(), &resolver);
                let Some(summary) = outcome.summary else {
                    unread.push(path);
                    continue;
                };
                let ranges = refs::occurrences(&text, &summary, &path, &decl);
                if !ranges.is_empty() {
                    out.push(FileRefs { path, text, ranges });
                }
            }
            (out, unread)
        })
        .await;
        if let Ok((more, missed)) = read {
            found.extend(more);
            unread = missed;
        }
        RefSearch { found, unread }
    }

    /// Why the declaration under the cursor cannot be renamed, if it cannot.
    fn rename_refused(&self, decl: &Decl, text: &str, summary: &ProgramSummary) -> Option<String> {
        if decl.std {
            return Some(format!(
                "`{}` is declared in the standard library",
                decl.name
            ));
        }
        let folders = self.folders.read().map(|f| f.clone()).unwrap_or_default();
        if !folders.is_empty() && !folders.iter().any(|f| decl.file.starts_with(f)) {
            return Some(format!(
                "`{}` is declared outside the workspace, in {}",
                decl.name,
                decl.file.display()
            ));
        }
        if summary.sources.first().is_none_or(|s| s.text != text) {
            return Some("the file does not parse as it stands; fix that first".to_owned());
        }
        if decl.local && refs::bound_by_pattern(text, &decl.name) {
            return Some(format!(
                "`{}` is also bound by a match arm or a catch clause in this file, which \
                 renaming cannot tell apart; rename it by hand",
                decl.name
            ));
        }
        None
    }

    /// The declarations matching `query` in the workspace's `.cdl` files and
    /// the open buffers.
    fn workspace_symbols(&self, query: &str) -> Vec<SymbolInformation> {
        let folders = self.folders.read().map(|f| f.clone()).unwrap_or_default();
        let open: HashMap<PathBuf, (Url, String)> = self
            .docs
            .iter()
            .map(|d| (d.path.clone(), (d.key().clone(), d.text.clone())))
            .collect();
        let mut files: Vec<PathBuf> = folders
            .iter()
            .flat_map(|f| cdl_files(f, MAX_PACKAGE_FILES))
            .collect();
        files.extend(open.keys().cloned());
        files.sort();
        files.dedup();
        let mut out = Vec::new();
        let Ok(mut cache) = self.declared.lock() else {
            return out;
        };
        cache.retain(|path, _| files.binary_search(path).is_ok());
        for path in files {
            let (uri, text, declared) = if let Some((uri, text)) = open.get(&path) {
                (uri.clone(), text.clone(), symbols::declarations(text))
            } else {
                let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
                    continue;
                };
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let declared = match cache.get(&path) {
                    Some((at, d)) if *at == modified => d.clone(),
                    _ => {
                        let d = symbols::declarations(&text);
                        cache.insert(path.clone(), (modified, d.clone()));
                        d
                    }
                };
                let Ok(uri) = Url::from_file_path(&path) else {
                    continue;
                };
                (uri, text, declared)
            };
            let mut index = None;
            for d in declared
                .into_iter()
                .filter(|d| symbols::matches(&d.name, query))
            {
                let index = index.get_or_insert_with(|| LineIndex::new(&text));
                #[allow(deprecated)]
                out.push(SymbolInformation {
                    name: d.name,
                    kind: d.kind,
                    tags: None,
                    deprecated: None,
                    location: Location::new(uri.clone(), index.range(d.start, d.end)),
                    container_name: d.container,
                });
                if out.len() >= MAX_SYMBOLS {
                    return out;
                }
            }
        }
        out
    }

    /// Says in the editor's log which files a search could not read.
    async fn report_unread(&self, search: &RefSearch) {
        for path in &search.unread {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!(
                        "candela-lsp: {} does not compile, so its references were not searched",
                        path.display()
                    ),
                )
                .await;
        }
    }

    /// The URI an edit or a location for `path` names: the open buffer's
    /// own when the file is open.
    fn uri_for(&self, path: &Path) -> Option<Url> {
        self.docs
            .iter()
            .find(|d| d.path == path)
            .map(|d| d.key().clone())
            .or_else(|| Url::from_file_path(path).ok())
    }

    fn apply_settings(&self, value: &Value) {
        let settings = ImportSettings::from_json(value);
        if let Ok(mut s) = self.settings.write() {
            *s = settings;
        }
    }
}

/// The settings' resolver, reading `overlays` in place of their files.
fn resolver_with(settings: &ImportSettings, overlays: Vec<(PathBuf, String)>) -> ImportResolver {
    let mut resolver = settings.resolver();
    for (path, text) in overlays {
        resolver.set_overlay(path, Arc::from(text));
    }
    resolver
}

/// A request the server turns down, with the reason the editor shows.
fn refused(message: String) -> RpcError {
    RpcError {
        // RequestFailed: the request was valid, and it could not be done.
        code: ErrorCode::ServerError(-32803),
        message: message.into(),
        data: None,
    }
}

/// The directory to search for references to an item declared in `file`:
/// the nearest one above it holding a `candela.toml`, or else the workspace
/// folder it is in.
fn package_root(file: &Path, folders: &[PathBuf]) -> Option<PathBuf> {
    let dir = file.parent()?;
    for ancestor in dir.ancestors() {
        if ancestor.join("candela.toml").is_file() {
            return Some(ancestor.to_path_buf());
        }
        if folders.iter().any(|f| f == ancestor) {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

/// The `.cdl` files under `root`, canonical, at most `max` of them. Hidden
/// directories and build output (`target`, `node_modules`) are skipped.
fn cdl_files(root: &Path, max: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    stack.push(entry.path());
                }
            } else if name.ends_with(".cdl") {
                out.push(canonical(&entry.path()));
                if out.len() >= max {
                    return out;
                }
            }
        }
    }
    out
}

/// The check commands of the packages a program's files belong to, once
/// each, filled in for the buffer being checked.
fn package_checks(summary: &ProgramSummary, checked: &Path) -> Vec<PackageCheck> {
    let mut seen_dirs: HashSet<PathBuf> = HashSet::new();
    let mut found: Vec<PackageCheck> = Vec::new();
    for source in summary.sources.iter().filter(|s| !s.is_std) {
        let path = canonical(Path::new(&source.path));
        let Some(dir) = path.parent() else { continue };
        if !seen_dirs.insert(dir.to_path_buf()) {
            continue;
        }
        if let Some(check) = candela::package_check(&path, checked)
            && !found.iter().any(|c| c.root == check.root)
        {
            found.push(check);
        }
    }
    found
}

fn range_of(text: &str, start: usize, end: usize) -> Range {
    LineIndex::new(text).range(start, end)
}

/// The editor location of a span in one of the program's files. The
/// compile kept every file's text, so the span is placed against the text
/// it indexes; the buffer's own declarations map back to the buffer.
fn location_for(
    summary: &ProgramSummary,
    src_file: u16,
    span: candela::compiler::expr::Span,
    current_uri: &Url,
) -> Option<Location> {
    let source = summary.sources.get(src_file as usize)?;
    let uri = if src_file == 0 {
        current_uri.clone()
    } else {
        Url::from_file_path(&source.path).ok()?
    };
    let range = range_of(&source.text, span.start as usize, span.end as usize);
    Some(Location::new(uri, range))
}

#[allow(deprecated)]
fn document_symbol(
    index: &LineIndex<'_>,
    name: &str,
    detail: String,
    kind: SymbolKind,
    span: candela::compiler::expr::Span,
) -> DocumentSymbol {
    let range = index.range(span.start as usize, span.end as usize);
    DocumentSymbol {
        name: name.to_owned(),
        detail: Some(detail),
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range: range,
        children: None,
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> RpcResult<InitializeResult> {
        if let Some(options) = &params.initialization_options {
            self.inner.apply_settings(options);
        }
        #[allow(deprecated)]
        let mut folders: Vec<Url> = params
            .workspace_folders
            .iter()
            .flatten()
            .map(|f| f.uri.clone())
            .collect();
        #[allow(deprecated)]
        if folders.is_empty()
            && let Some(root) = &params.root_uri
        {
            folders.push(root.clone());
        }
        let workspace = params.capabilities.workspace.as_ref();
        let tokens = workspace
            .and_then(|w| w.semantic_tokens.as_ref())
            .and_then(|t| t.refresh_support)
            .unwrap_or(false);
        let hints = workspace
            .and_then(|w| w.inlay_hint.as_ref())
            .and_then(|t| t.refresh_support)
            .unwrap_or(false);
        self.inner
            .refresh_tokens
            .store(tokens, std::sync::atomic::Ordering::Relaxed);
        self.inner
            .refresh_hints
            .store(hints, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut f) = self.inner.folders.write() {
            *f = folders
                .iter()
                .filter_map(|u| u.to_file_path().ok())
                .map(|p| canonical(&p))
                .collect();
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                        ..TextDocumentSyncOptions::default()
                    },
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(
                        // Not `(` or `{`: a list opened there would take
                        // the Enter that ends the line.
                        [".", ":", "@"].iter().map(|s| (*s).to_owned()).collect(),
                    ),
                    ..Default::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_owned(), ",".to_owned()]),
                    retrigger_characters: None,
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: semantic::TYPES.to_vec(),
                                token_modifiers: semantic::MODIFIERS.to_vec(),
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
                            work_done_progress_options: WorkDoneProgressOptions::default(),
                        },
                    ),
                ),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "candela-lsp".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.inner
            .client
            .log_message(MessageType::INFO, "candela-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.inner.apply_settings(&params.settings);
        // An import that did not resolve before may now.
        let uris: Vec<Url> = self.inner.docs.iter().map(|d| d.key().clone()).collect();
        for uri in uris {
            self.inner.schedule_compile(&uri, Duration::ZERO);
            self.inner.schedule_package_checks(&uri, CHECK_DELAY);
        }
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let path = canonical(Path::new(&uri_to_path(&uri)));
        self.inner.docs.insert(
            uri.clone(),
            Document {
                text: params.text_document.text,
                path,
                last_good: None,
                generation: 0,
                compiled_generation: 0,
                check_generation: 0,
            },
        );
        // Checked before the next message is read, so a request right after
        // the open has a summary to work from.
        let generation = self.inner.docs.get_mut(&uri).map_or(0, |mut d| {
            d.generation += 1;
            d.generation
        });
        self.inner.compile(&uri, generation, false).await;
        // The programs that read the file read the buffer from now on. Their
        // checks wait a moment, so opening several of their files (a
        // restored session) checks each once.
        let delay = self.inner.settings().delay;
        if let Some(path) = self.inner.docs.get(&uri).map(|d| d.path.clone()) {
            for dependent in self.inner.dependents(&path, &uri) {
                self.inner.schedule_compile(&dependent, delay);
            }
        }
        self.inner.schedule_package_checks(&uri, CHECK_DELAY);
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        // Full sync (declared in `initialize`): the last change event carries
        // the entire new document text.
        let Some(change) = params.content_changes.pop() else {
            return;
        };
        if let Some(mut doc) = self.inner.docs.get_mut(&uri) {
            doc.text = change.text;
        } else {
            let path = canonical(Path::new(&uri_to_path(&uri)));
            self.inner.docs.insert(
                uri.clone(),
                Document {
                    text: change.text,
                    path,
                    last_good: None,
                    generation: 0,
                    compiled_generation: 0,
                    check_generation: 0,
                },
            );
        }
        let delay = self.inner.settings().delay;
        self.inner.changed(&uri, delay);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // Check again on save too: an `import`-ed file may have changed on
        // disk even when this buffer's own text did not. A package check
        // reads the files from disk, so it runs after a save.
        let uri = params.text_document.uri;
        self.inner.changed(&uri, Duration::ZERO);
        let mut roots = vec![uri.clone()];
        if let Some(path) = self.inner.docs.get(&uri).map(|d| d.path.clone()) {
            roots.extend(self.inner.dependents(&path, &uri));
        }
        for root in roots {
            self.inner.schedule_package_checks(&root, CHECK_DELAY);
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        // A file changed outside the editor: the programs that read it are
        // checked again, and the package checks of every open program run,
        // since what they read is theirs to know. A file open in the editor
        // is left out: the client watches the files it saves too, and
        // `didSave` has already checked what a save changed.
        let open: HashSet<PathBuf> = self.inner.docs.iter().map(|d| d.path.clone()).collect();
        let mut compile: HashSet<Url> = HashSet::new();
        let mut outside = false;
        for change in &params.changes {
            let Ok(path) = change.uri.to_file_path() else {
                continue;
            };
            let path = canonical(&path);
            if open.contains(&path) {
                continue;
            }
            outside = true;
            let none = Url::parse("file:///").expect("a URL");
            compile.extend(self.inner.dependents(&path, &none));
        }
        for uri in &compile {
            self.inner.schedule_compile(uri, Duration::ZERO);
        }
        if !outside {
            return;
        }
        let open: Vec<Url> = self.inner.docs.iter().map(|d| d.key().clone()).collect();
        for uri in open {
            self.inner.schedule_package_checks(&uri, CHECK_DELAY);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        // The buffer goes before its state does; see `Inner::compile`.
        let path = self.inner.docs.remove(&uri).map(|(_, d)| d.path);
        let mut affected: HashSet<Url> = HashSet::from([uri.clone()]);
        if let Ok(mut roots) = self.inner.roots.lock()
            && let Some(state) = roots.remove(&uri)
        {
            affected.extend(state.compiled.into_keys());
            affected.extend(state.checked.into_keys());
        }
        self.inner.publish(affected, Some(&uri)).await;
        // The programs that read the buffer read the file from now on.
        if let Some(path) = path {
            for dependent in self.inner.dependents(&path, &uri) {
                self.inner.schedule_compile(&dependent, Duration::ZERO);
            }
        }
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        let p = params.text_document_position_params;
        let Some((text, summary, offset)) = self.inner.request(&p.text_document.uri, p.position)
        else {
            return Ok(None);
        };
        Ok(Ide::new(&text, &summary, offset)
            .hover()
            .map(|value| Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                }),
                range: None,
            }))
    }

    async fn completion(&self, params: CompletionParams) -> RpcResult<Option<CompletionResponse>> {
        let p = params.text_document_position;
        let Some(text) = self.inner.doc_text(&p.text_document.uri) else {
            return Ok(None);
        };
        // A buffer that never compiled still gets keywords and builtins.
        let summary = self
            .inner
            .typing_summary(&p.text_document.uri)
            .unwrap_or_default();
        let offset = line_index::position_to_offset(&text, p.position);
        let items = Ide::new(&text, &summary, offset).completions();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> RpcResult<Option<SignatureHelp>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let (Some(text), Some(summary)) =
            (self.inner.doc_text(uri), self.inner.typing_summary(uri))
        else {
            return Ok(None);
        };
        let offset = line_index::position_to_offset(&text, p.position);
        Ok(Ide::new(&text, &summary, offset).signature_help())
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(summary) = self.inner.current_or_cached_summary(&uri) else {
            return Ok(None);
        };
        let text = summary.sources.first().map_or("", |s| s.text.as_str());
        let text = &LineIndex::new(text);
        let mut symbols = Vec::new();
        for f in summary.own_functions() {
            let kind = if f.is_const {
                SymbolKind::CONSTANT
            } else {
                SymbolKind::FUNCTION
            };
            symbols.push(document_symbol(
                text,
                &f.name,
                f.header(),
                kind,
                f.name_span,
            ));
        }
        for s in summary.own_structs() {
            let detail = format!("struct {}", s.name);
            symbols.push(document_symbol(
                text,
                &s.name,
                detail,
                SymbolKind::STRUCT,
                s.name_span,
            ));
        }
        for e in summary.own_enums() {
            let detail = format!("enum {}", e.name);
            symbols.push(document_symbol(
                text,
                &e.name,
                detail,
                SymbolKind::ENUM,
                e.name_span,
            ));
        }
        symbols.sort_by_key(|s| (s.range.start.line, s.range.start.character));
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> RpcResult<Option<GotoDefinitionResponse>> {
        let p = params.text_document_position_params;
        let uri = p.text_document.uri;
        let Some((text, summary, offset)) = self.inner.request(&uri, p.position) else {
            return Ok(None);
        };
        let location = match Ide::new(&text, &summary, offset).definition() {
            Some(Ok(target)) => location_for(&summary, target.src_file, target.span, &uri),
            Some(Err(at)) => Some(Location::new(uri.clone(), range_of(&text, at, at))),
            None => None,
        };
        Ok(location.map(GotoDefinitionResponse::Scalar))
    }

    async fn references(&self, params: ReferenceParams) -> RpcResult<Option<Vec<Location>>> {
        let p = params.text_document_position;
        let Some((decl, _, _, summary)) =
            self.inner.declaration_at(&p.text_document.uri, p.position)
        else {
            return Ok(None);
        };
        let search = self.inner.find_references(&decl, &summary).await;
        self.inner.report_unread(&search).await;
        let mut locations = Vec::new();
        for file in &search.found {
            let Some(uri) = self.inner.uri_for(&file.path) else {
                continue;
            };
            let index = LineIndex::new(&file.text);
            for &(s, e) in &file.ranges {
                if !params.context.include_declaration
                    && file.path == decl.file
                    && decl.name_range == Some((s, e))
                {
                    continue;
                }
                locations.push(Location::new(uri.clone(), index.range(s, e)));
            }
        }
        Ok(Some(locations))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> RpcResult<Option<Vec<DocumentHighlight>>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let Some((decl, _, text, summary)) = self.inner.declaration_at(uri, p.position) else {
            return Ok(None);
        };
        let Some(path) = self.inner.doc_path(uri) else {
            return Ok(None);
        };
        let index = LineIndex::new(&text);
        let highlights = refs::occurrences(&text, &summary, &path, &decl)
            .into_iter()
            .map(|(s, e)| {
                let written = decl.name_range == Some((s, e)) && decl.file == path
                    || refs::assigned_at(&text, e);
                DocumentHighlight {
                    range: index.range(s, e),
                    kind: Some(if written {
                        DocumentHighlightKind::WRITE
                    } else {
                        DocumentHighlightKind::READ
                    }),
                }
            })
            .collect();
        Ok(Some(highlights))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> RpcResult<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let Some(text) = self.inner.doc_text(uri) else {
            return Ok(None);
        };
        // Like completion, from the last check: the names are read from the
        // current text, and a check that brings in new declarations asks
        // the client to fetch them again.
        let summary = self.inner.typing_summary(uri).unwrap_or_default();
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: semantic::tokens(&text, &summary),
        })))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> RpcResult<Option<Vec<InlayHint>>> {
        let uri = &params.text_document.uri;
        let Some(text) = self.inner.doc_text(uri) else {
            return Ok(None);
        };
        let summary = self.inner.typing_summary(uri).unwrap_or_default();
        let index = LineIndex::new(&text);
        let (start, end) = (
            index.offset(params.range.start),
            index.offset(params.range.end),
        );
        Ok(Some(hints::hints(&text, &summary, start, end)))
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> RpcResult<Option<Vec<SymbolInformation>>> {
        Ok(Some(self.inner.workspace_symbols(&params.query)))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> RpcResult<Option<PrepareRenameResponse>> {
        let Some((decl, (s, e), text, summary)) = self
            .inner
            .declaration_at(&params.text_document.uri, params.position)
        else {
            return Err(refused("nothing here can be renamed".to_owned()));
        };
        if let Some(why) = self.inner.rename_refused(&decl, &text, &summary) {
            return Err(refused(why));
        }
        Ok(Some(PrepareRenameResponse::Range(
            LineIndex::new(&text).range(s, e),
        )))
    }

    async fn rename(&self, params: RenameParams) -> RpcResult<Option<WorkspaceEdit>> {
        let p = params.text_document_position;
        let Some((decl, _, text, summary)) =
            self.inner.declaration_at(&p.text_document.uri, p.position)
        else {
            return Err(refused("nothing here can be renamed".to_owned()));
        };
        if let Some(why) = self.inner.rename_refused(&decl, &text, &summary) {
            return Err(refused(why));
        }
        let new_name = params.new_name.trim();
        if !refs::valid_name(new_name) {
            return Err(refused(format!("`{new_name}` is not a name")));
        }
        let search = self.inner.find_references(&decl, &summary).await;
        if let Some(path) = search.unread.first() {
            // A partial rename would leave the program broken without
            // saying where.
            return Err(refused(format!(
                "{} does not compile, so its uses of `{}` cannot be found; fix it first",
                path.display(),
                decl.name
            )));
        }
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for file in &search.found {
            let Some(uri) = self.inner.uri_for(&file.path) else {
                continue;
            };
            let index = LineIndex::new(&file.text);
            changes.entry(uri).or_default().extend(
                file.ranges
                    .iter()
                    .map(|&(s, e)| TextEdit::new(index.range(s, e), new_name.to_owned())),
            );
        }
        Ok(Some(WorkspaceEdit::new(changes)))
    }
}
