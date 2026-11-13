//! The `tower_lsp::LanguageServer` implementation: wires document sync,
//! diagnostics, hover, completion, signature help, document symbols, and
//! go-to-definition to the analysis in `crate::analysis` (a thin wrapper
//! around candela's own compile) and the position-based features in
//! `crate::ide`.

use crate::analysis::{self, ProgramSummary};
use crate::ide::Ide;
use crate::line_index;
use candela::Diagnostic as CdlDiagnostic;
use candela::compiler::imports::ImportResolver;
use dashmap::DashMap;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::RwLock;
use tower_lsp::jsonrpc::Result as RpcResult;
use tower_lsp::lsp_types::Diagnostic as LspDiagnostic;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

struct Document {
    text: String,
    /// The last summary produced by a *successful* compile of this
    /// document, kept around so hover/completion/document-symbols/
    /// go-to-definition still work while the buffer is mid-edit and
    /// currently failing to compile, instead of going blank on every
    /// keystroke that introduces a temporary syntax error.
    last_good: Option<ProgramSummary>,
}

/// Where library imports read from, as the editor's settings give it.
///
/// The client sends it as `initializationOptions` and again with every
/// `workspace/didChangeConfiguration`, either bare or under a `candela` key:
///
/// ```json
/// { "libPath": "/opt/candela/libs", "importRoots": { "drift": "/opt/drift/module" } }
/// ```
///
/// `libPath` is the directory holding `std/`, what `CANDELA_LIB_PATH` names;
/// without it the standard library is looked up the way the `candela` command
/// does. Each `importRoots` entry makes `import "name";` read a package: a
/// directory is a package root, entered through the file its `candela.toml`
/// names or `src/main.cdl`; a `.cdl` file is the module itself.
#[derive(Debug, Clone, Default)]
pub struct ImportSettings {
    lib_path: Option<PathBuf>,
    roots: Vec<(String, PathBuf)>,
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
        Self { lib_path, roots }
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
    client: Client,
    docs: DashMap<Url, Document>,
    /// Where library imports read from. A resolver is built from it for
    /// each compile; the resolver itself does not cross threads.
    settings: RwLock<ImportSettings>,
}

impl Backend {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            docs: DashMap::new(),
            settings: RwLock::new(ImportSettings::default()),
        }
    }

    /// The buffer's real filesystem path, so `import "..."` resolves the
    /// same way it would for the `candela` CLI. Falls back to the raw URI
    /// path for non-`file://` URIs (untitled buffers, etc.).
    fn uri_to_path(uri: &Url) -> String {
        uri.to_file_path()
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned))
            .unwrap_or_else(|| uri.path().to_owned())
    }

    fn analyze(&self, text: &str, path: &str) -> analysis::AnalysisOutcome {
        let resolver = self
            .settings
            .read()
            .map_or_else(|_| ImportResolver::new(), |s| s.resolver());
        analysis::analyze(text, path, &resolver)
    }

    /// Re-analyzes the document's current text. On success, refreshes the
    /// cached `last_good` summary and returns it; on failure, falls back to
    /// whatever was last cached (which may be `None` if the document has
    /// never compiled successfully).
    fn current_or_cached_summary(&self, uri: &Url) -> Option<ProgramSummary> {
        let text = self.doc_text(uri)?;
        let outcome = self.analyze(&text, &Self::uri_to_path(uri));
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

    async fn publish_diagnostics_for(&self, uri: &Url) {
        let Some(text) = self.doc_text(uri) else {
            return;
        };
        let path = Self::uri_to_path(uri);
        let outcome = self.analyze(&text, &path);
        if let Some(summary) = &outcome.summary
            && let Some(mut doc) = self.docs.get_mut(uri)
        {
            doc.last_good = Some(summary.clone());
        }
        let mut diagnostics: Vec<LspDiagnostic> = outcome
            .diagnostic
            .iter()
            .map(|d| to_lsp_diagnostic(d, &path, &text, DiagnosticSeverity::ERROR))
            .chain(
                outcome
                    .warnings
                    .iter()
                    .map(|w| to_lsp_diagnostic(w, &path, &text, DiagnosticSeverity::WARNING)),
            )
            .collect();
        if let Some(summary) = &outcome.summary {
            for (start, end, message, code) in crate::ide::attribute_warnings(&text, summary) {
                diagnostics.push(LspDiagnostic {
                    range: range_of(&text, start, end),
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String(code.to_owned())),
                    source: Some("candela".to_owned()),
                    message,
                    ..LspDiagnostic::default()
                });
            }
        }
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }

    /// The text, summary and byte offset a position request works on.
    fn request(&self, uri: &Url, position: Position) -> Option<(String, ProgramSummary, usize)> {
        let text = self.doc_text(uri)?;
        let summary = self.current_or_cached_summary(uri)?;
        let offset = line_index::position_to_offset(&text, position) as usize;
        Some((text, summary, offset))
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

    fn apply_settings(&self, value: &Value) {
        let settings = ImportSettings::from_json(value);
        if let Ok(mut s) = self.settings.write() {
            *s = settings;
        }
    }
}

fn range_of(text: &str, start: usize, end: usize) -> Range {
    Range::new(
        line_index::offset_to_position(text, u32::try_from(start).unwrap_or(u32::MAX)),
        line_index::offset_to_position(text, u32::try_from(end).unwrap_or(u32::MAX)),
    )
}

fn to_lsp_diagnostic(
    d: &CdlDiagnostic,
    doc_path: &str,
    doc_text: &str,
    severity: DiagnosticSeverity,
) -> LspDiagnostic {
    // The first error in a compile can originate from an imported file
    // rather than the buffer itself (e.g. a type error inside a module this
    // document imports). We can only place a precise range against text we
    // have in hand (this buffer's), so an error from elsewhere is reported
    // at the top of the document with the real file named in the message,
    // rather than a fabricated location.
    let (range, message) = if d.filename == doc_path {
        (
            range_of(doc_text, d.span.start, d.span.end),
            d.message.clone(),
        )
    } else {
        (
            Range::new(Position::new(0, 0), Position::new(0, 0)),
            format!("{} (in {})", d.message, d.filename),
        )
    };
    LspDiagnostic {
        range,
        severity: Some(severity),
        code: Some(NumberOrString::String(d.code.clone())),
        code_description: None,
        source: Some("candela".to_owned()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
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
            self.apply_settings(options);
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
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
        self.client
            .log_message(MessageType::INFO, "candela-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.apply_settings(&params.settings);
        // An import that did not resolve before may now.
        let uris: Vec<Url> = self.docs.iter().map(|d| d.key().clone()).collect();
        for uri in uris {
            self.publish_diagnostics_for(&uri).await;
        }
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.docs.insert(
            uri.clone(),
            Document {
                text: params.text_document.text,
                last_good: None,
            },
        );
        self.publish_diagnostics_for(&uri).await;
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        // Full sync (declared in `initialize`): the last change event carries
        // the entire new document text.
        let Some(change) = params.content_changes.pop() else {
            return;
        };
        if let Some(mut doc) = self.docs.get_mut(&uri) {
            doc.text = change.text;
        } else {
            self.docs.insert(
                uri.clone(),
                Document {
                    text: change.text,
                    last_good: None,
                },
            );
        }
        self.publish_diagnostics_for(&uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // Re-analyze on save too: an `import`-ed file may have changed on
        // disk even when this buffer's own text did not.
        self.publish_diagnostics_for(&params.text_document.uri)
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.docs.remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        let p = params.text_document_position_params;
        let Some((text, summary, offset)) = self.request(&p.text_document.uri, p.position) else {
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
        let Some(text) = self.doc_text(&p.text_document.uri) else {
            return Ok(None);
        };
        // A buffer that never compiled still gets keywords and builtins.
        let summary = self
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
        let Some((text, summary, offset)) = self.request(&p.text_document.uri, p.position) else {
            return Ok(None);
        };
        Ok(Ide::new(&text, &summary, offset).signature_help())
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(summary) = self.current_or_cached_summary(&uri) else {
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
        let Some((text, summary, offset)) = self.request(&uri, p.position) else {
            return Ok(None);
        };
        let location = match Ide::new(&text, &summary, offset).definition() {
            Some(Ok(target)) => Self::location_for(&summary, target.src_file, target.span, &uri),
            Some(Err(at)) => Some(Location::new(uri.clone(), range_of(&text, at, at))),
            None => None,
        };
        Ok(location.map(GotoDefinitionResponse::Scalar))
    }
}
