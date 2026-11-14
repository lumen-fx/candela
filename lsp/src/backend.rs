//! The `tower_lsp::LanguageServer` implementation: wires document sync,
//! diagnostics, hover, completion, signature help, document symbols, and
//! go-to-definition to the analysis in `crate::analysis` (a thin wrapper
//! around candela's own compile) and the position-based features in
//! `crate::ide`.
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
use crate::ide::Ide;
use crate::line_index;
use candela::Diagnostic as CdlDiagnostic;
use candela::PackageCheck;
use candela::compiler::imports::ImportResolver;
use dashmap::DashMap;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tower_lsp::jsonrpc::Result as RpcResult;
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
}

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

    /// Re-analyzes the document's current text. On success, refreshes the
    /// cached `last_good` summary and returns it; on failure, falls back to
    /// whatever was last cached (which may be `None` if the document has
    /// never compiled successfully).
    fn current_or_cached_summary(&self, uri: &Url) -> Option<ProgramSummary> {
        let text = self.doc_text(uri)?;
        let outcome = self.analyze(uri, &text);
        let mut doc = self.docs.get_mut(uri)?;
        if let Some(summary) = outcome.summary {
            doc.last_good = Some(summary.clone());
            Some(summary)
        } else {
            doc.last_good.clone()
        }
    }

    fn doc_text(&self, uri: &Url) -> Option<String> {
        self.docs.get(uri).map(|d| d.text.clone())
    }

    /// The text, summary and byte offset a position request works on.
    fn request(&self, uri: &Url, position: Position) -> Option<(String, ProgramSummary, usize)> {
        let text = self.doc_text(uri)?;
        let summary = self.current_or_cached_summary(uri)?;
        let offset = line_index::position_to_offset(&text, position) as usize;
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
        if let Some(summary) = &outcome.summary
            && let Some(mut doc) = self.docs.get_mut(uri)
            && (doc.generation == generation || doc.last_good.is_none())
        {
            doc.last_good = Some(summary.clone());
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

    async fn run_package_checks(self: &Arc<Self>, uri: &Url, generation: u64) {
        if self.docs.get(uri).map(|d| d.check_generation) != Some(generation)
            || !self.settings().package_checks
        {
            return;
        }
        let checks = self
            .roots
            .lock()
            .ok()
            .and_then(|roots| roots.get(uri).map(|s| s.checks.clone()))
            .unwrap_or_default();
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
        let mut place = |d: &CdlDiagnostic, severity: DiagnosticSeverity| {
            let (uri, diagnostic) = self.place(d, severity, root, root_path, root_text, &texts);
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
                        range: range_of(root_text, start, end),
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
        root_text: &str,
        texts: &HashMap<&str, &str>,
    ) -> (Url, LspDiagnostic) {
        let locate = |filename: &str, span: &std::ops::Range<usize>| -> Option<(Url, Range)> {
            if filename == root_path || filename.is_empty() {
                return Some((root.clone(), range_of(root_text, span.start, span.end)));
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
    Range::new(
        line_index::offset_to_position(text, u32::try_from(start).unwrap_or(u32::MAX)),
        line_index::offset_to_position(text, u32::try_from(end).unwrap_or(u32::MAX)),
    )
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
    text: &str,
    name: &str,
    detail: String,
    kind: SymbolKind,
    span: candela::compiler::expr::Span,
) -> DocumentSymbol {
    let range = range_of(text, span.start as usize, span.end as usize);
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
        if let Some(path) = self.inner.docs.get(&uri).map(|d| d.path.clone()) {
            for dependent in self.inner.dependents(&path, &uri) {
                self.inner.schedule_compile(&dependent, Duration::ZERO);
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
        // since what they read is theirs to know.
        let mut compile: HashSet<Url> = HashSet::new();
        for change in &params.changes {
            let Ok(path) = change.uri.to_file_path() else {
                continue;
            };
            let path = canonical(&path);
            let none = Url::parse("file:///").expect("a URL");
            compile.extend(self.inner.dependents(&path, &none));
        }
        for uri in &compile {
            self.inner.schedule_compile(uri, Duration::ZERO);
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
            .current_or_cached_summary(&p.text_document.uri)
            .unwrap_or_default();
        let offset = line_index::position_to_offset(&text, p.position) as usize;
        let items = Ide::new(&text, &summary, offset).completions();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> RpcResult<Option<SignatureHelp>> {
        let p = params.text_document_position_params;
        let Some((text, summary, offset)) = self.inner.request(&p.text_document.uri, p.position)
        else {
            return Ok(None);
        };
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
}
