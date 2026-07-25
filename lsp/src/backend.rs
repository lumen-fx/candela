//! The `tower_lsp::LanguageServer` implementation: wires document sync,
//! diagnostics, hover, completion, document symbols, and go-to-definition to
//! the analysis in `crate::analysis`, which in turn is a thin wrapper around
//! candela's own `compile()`.

use crate::analysis::{self, ProgramSummary, RefKind};
use crate::builtins;
use crate::line_index;
use candela::Diagnostic as CdlDiagnostic;
use dashmap::DashMap;
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

pub struct Backend {
    client: Client,
    docs: DashMap<Url, Document>,
}

impl Backend {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            docs: DashMap::new(),
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

    /// Re-analyzes the document's current text. On success, refreshes the
    /// cached `last_good` summary and returns it; on failure, falls back to
    /// whatever was last cached (which may be `None` if the document has
    /// never compiled successfully).
    fn current_or_cached_summary(&self, uri: &Url) -> Option<ProgramSummary> {
        let mut doc = self.docs.get_mut(uri)?;
        let path = Self::uri_to_path(uri);
        let outcome = analysis::analyze(&doc.text, &path);
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
        let outcome = analysis::analyze(&text, &path);
        if let Some(summary) = &outcome.summary
            && let Some(mut doc) = self.docs.get_mut(uri)
        {
            doc.last_good = Some(summary.clone());
        }
        let diagnostics = outcome
            .diagnostic
            .map(|d| vec![to_lsp_diagnostic(&d, &path, &text)])
            .unwrap_or_default();
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }

    /// Resolves a `RefSite`'s target to editor `Location`s. For a
    /// declaration in the buffer itself (`src_file == 0`) this reuses the
    /// buffer's own in-memory text. For a declaration pulled in via
    /// `import` (`src_file != 0`), candela records that file's absolute,
    /// canonicalized path (see `ImportFile` handling in
    /// `compiler.rs`) but not its text, so this reads it from disk -- a
    /// synchronous read on the async task, acceptable for the small scripts
    /// candela targets, but a known limitation (see README) for very large
    /// imported files or files that only exist unsaved in another editor
    /// buffer.
    fn location_for(
        &self,
        summary: &ProgramSummary,
        src_file: u16,
        span: candela::compiler::expr::Span,
        current_uri: &Url,
    ) -> Option<Location> {
        let (target_uri, text) = if src_file == 0 {
            (current_uri.clone(), self.doc_text(current_uri)?)
        } else {
            let path = summary.source_files.get(src_file as usize)?;
            let uri = Url::from_file_path(path).ok()?;
            let text = std::fs::read_to_string(path).ok()?;
            (uri, text)
        };
        let range = Range::new(
            line_index::offset_to_position(&text, span.start),
            line_index::offset_to_position(&text, span.end),
        );
        Some(Location::new(target_uri, range))
    }
}

fn to_lsp_diagnostic(d: &CdlDiagnostic, doc_path: &str, doc_text: &str) -> LspDiagnostic {
    // The first error in a compile can originate from an imported file
    // rather than the buffer itself (e.g. a type error inside a module this
    // document imports). We can only place a precise range against text we
    // have in hand (this buffer's), so an error from elsewhere is reported
    // at the top of the document with the real file named in the message,
    // rather than a fabricated location.
    let (range, message) = if d.filename == doc_path {
        (
            Range::new(
                line_index::offset_to_position(doc_text, d.span.start as u32),
                line_index::offset_to_position(doc_text, d.span.end as u32),
            ),
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
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(d.code.clone())),
        code_description: None,
        source: Some("candela".to_owned()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

fn render_function_hover(f: &analysis::FunctionSymbol) -> String {
    let signature = format!("fn {}({})", f.name, f.params.join(", "));
    let detail = if f.signatures.is_empty() {
        "Return type not yet inferred: candela specializes a function's return type per call \
         site, and this function has not been called anywhere in the compiled program yet."
            .to_owned()
    } else {
        let lines: Vec<String> = f.signatures.iter().map(|s| format!("- `{s}`")).collect();
        format!("Inferred signature(s):\n{}", lines.join("\n"))
    };
    format!("```candela\n{signature}\n```\n\n{detail}")
}

fn render_struct_hover(s: &analysis::StructSymbol) -> String {
    let fields: Vec<String> = s
        .fields
        .iter()
        .map(|(name, ty)| format!("    {name}: {ty},"))
        .collect();
    format!(
        "```candela\nstruct {} {{\n{}\n}}\n```",
        s.name,
        fields.join("\n")
    )
}

fn hover_markdown(summary: &ProgramSummary, offset: u32) -> Option<String> {
    if let Some(f) = summary.own_function_decl_at(offset) {
        return Some(render_function_hover(f));
    }
    if let Some(s) = summary.own_struct_decl_at(offset) {
        return Some(render_struct_hover(s));
    }
    let r = summary.reference_at(offset)?;
    match r.kind {
        RefKind::Call => {
            if let Some(f) = summary.functions_named(&r.target_name).next() {
                return Some(render_function_hover(f));
            }
            builtins::BUILTIN_FUNCTIONS
                .iter()
                .chain(builtins::BUILTIN_METHODS)
                .find(|(name, _)| *name == r.target_name)
                .map(|(name, doc)| format!("**{name}** (builtin)\n\n{doc}"))
        }
        RefKind::StructLiteral => summary
            .structs_named(&r.target_name)
            .next()
            .map(render_struct_hover),
    }
}

fn completion_item(label: &str, kind: CompletionItemKind, doc: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind: Some(kind),
        detail: Some(doc.lines().next().unwrap_or_default().to_owned()),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.to_owned(),
        })),
        ..CompletionItem::default()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> RpcResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_owned()]),
                    ..Default::default()
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
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.doc_text(&uri) else {
            return Ok(None);
        };
        let offset = line_index::position_to_offset(&text, position);
        let Some(summary) = self.current_or_cached_summary(&uri) else {
            return Ok(None);
        };
        Ok(hover_markdown(&summary, offset).map(|value| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }

    async fn completion(&self, params: CompletionParams) -> RpcResult<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let is_method_position = params
            .context
            .as_ref()
            .and_then(|c| c.trigger_character.as_deref())
            == Some(".");

        let mut items = Vec::new();
        if is_method_position {
            for (name, doc) in builtins::BUILTIN_METHODS {
                items.push(completion_item(name, CompletionItemKind::METHOD, doc));
            }
        } else {
            for kw in builtins::KEYWORDS {
                items.push(CompletionItem {
                    label: (*kw).to_owned(),
                    kind: Some(CompletionItemKind::KEYWORD),
                    ..CompletionItem::default()
                });
            }
            for (name, doc) in builtins::BUILTIN_FUNCTIONS {
                items.push(completion_item(name, CompletionItemKind::FUNCTION, doc));
            }
            if let Some(summary) = self.current_or_cached_summary(&uri) {
                for f in &summary.functions {
                    let doc = format!("fn {}({})", f.name, f.params.join(", "));
                    items.push(completion_item(&f.name, CompletionItemKind::FUNCTION, &doc));
                }
                for s in &summary.structs {
                    let doc = format!("struct {}", s.name);
                    items.push(completion_item(&s.name, CompletionItemKind::STRUCT, &doc));
                }
            }
        }
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(text) = self.doc_text(&uri) else {
            return Ok(None);
        };
        let Some(summary) = self.current_or_cached_summary(&uri) else {
            return Ok(None);
        };

        let mut symbols = Vec::new();
        for f in summary.own_functions() {
            let range = Range::new(
                line_index::offset_to_position(&text, f.name_span.start),
                line_index::offset_to_position(&text, f.name_span.end),
            );
            #[allow(deprecated)]
            symbols.push(DocumentSymbol {
                name: f.name.clone(),
                detail: Some(format!("fn {}({})", f.name, f.params.join(", "))),
                kind: SymbolKind::FUNCTION,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            });
        }
        for s in summary.own_structs() {
            let range = Range::new(
                line_index::offset_to_position(&text, s.name_span.start),
                line_index::offset_to_position(&text, s.name_span.end),
            );
            #[allow(deprecated)]
            symbols.push(DocumentSymbol {
                name: s.name.clone(),
                detail: Some(format!("struct {}", s.name)),
                kind: SymbolKind::STRUCT,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            });
        }
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> RpcResult<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.doc_text(&uri) else {
            return Ok(None);
        };
        let offset = line_index::position_to_offset(&text, position);
        let Some(summary) = self.current_or_cached_summary(&uri) else {
            return Ok(None);
        };
        let Some(r) = summary.reference_at(offset) else {
            return Ok(None);
        };

        // Bare-name matching only: `namespace::name` qualification in the
        // call is not resolved against the qualified path, just the final
        // segment. See the crate README's "known simplifications".
        let locations: Vec<Location> = match r.kind {
            RefKind::Call => summary
                .functions_named(&r.target_name)
                .filter_map(|f| self.location_for(&summary, f.src_file, f.name_span, &uri))
                .collect(),
            RefKind::StructLiteral => summary
                .structs_named(&r.target_name)
                .filter_map(|s| {
                    let src_file = s.src_file?;
                    self.location_for(&summary, src_file, s.name_span, &uri)
                })
                .collect(),
        };
        if locations.is_empty() {
            return Ok(None);
        }
        Ok(Some(GotoDefinitionResponse::Array(locations)))
    }
}
