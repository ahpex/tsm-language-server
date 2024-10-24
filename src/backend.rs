use crate::CliArgs;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::sync::{Arc, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tree_sitter::{Parser, Tree};

pub struct Doc {
    text: String,
    ast: Option<Tree>,
}

pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, Doc>>>, // To store opened documents
    args: CliArgs,
}

impl Backend {
    pub fn new(client: Client, args: CliArgs) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            args,
        }
    }

    fn get_files(root: &str) -> Vec<OsString> {
        match fs::read_dir(root) {
            Ok(paths) => paths
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|d| d.file_name())
                .collect::<Vec<OsString>>(),
            Err(_) => vec![],
        }
    }

    // Custom function to generate diagnostics based on text content
    fn perform_diagnostics(&self, text: String) -> Vec<Diagnostic> {
        if text.contains("error") {
            vec![Diagnostic {
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 5,
                    },
                },
                severity: Some(DiagnosticSeverity::INFORMATION),
                code: Some(NumberOrString::String("100".into())),
                source: Some("tsm-language-server".into()),
                message: "Found the word 'error'.".into(),
                ..Diagnostic::default()
            }]
        } else {
            vec![]
        }
    }
}

trait ConvertToCompletionItem {
    fn to_completionitem(&self) -> Option<CompletionItem>;
}

impl ConvertToCompletionItem for OsString {
    fn to_completionitem(&self) -> Option<CompletionItem> {
        let label = self.to_string_lossy().to_string();
        let mut item = CompletionItem::new_simple(label.clone(), "Directory".to_string());
        item.kind = Some(CompletionItemKind::FOLDER);
        item.insert_text = Some(format!("\"{}\"", label));
        Some(item)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                completion_provider: Some(CompletionOptions::default()),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        inter_file_dependencies: false,
                        workspace_diagnostics: false,
                        ..DiagnosticOptions::default()
                    },
                )),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let docs = self.documents.read().unwrap();
        let content = match docs.get(&params.text_document_position.text_document.uri) {
            Some(text) => text,
            None => {
                return Ok(None);
            }
        };

        let wanted_line = content
            .text
            .split_terminator("\n")
            .enumerate()
            .find(|(_line_no, line_content)| line_content.contains(self.args.prefix.as_str()));

        match wanted_line {
            Some((line_no, _line_content)) => {
                let wanted_line_no: u32 = line_no.try_into().unwrap();
                if params.text_document_position.position.line == wanted_line_no {
                    let completions = Backend::get_files(&self.args.suggestionsdir)
                        .iter()
                        .map(|name| name.to_completionitem().unwrap())
                        .collect::<Vec<CompletionItem>>();
                    return Ok(Some(CompletionResponse::Array(completions)));
                } else {
                    return Ok(None);
                }
            }
            None => return Ok(None),
        };
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let docs = self.documents.write();
        docs.unwrap().insert(
            params.text_document.uri,
            Doc {
                text: params.text_document.text,
                ast: None,
            },
        );
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let docs = self.documents.write();
        let text = &params.content_changes.first().unwrap().text;

        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .expect("Error loading typescript grammar");
        let tree = parser.parse(text, None);

        docs.unwrap().insert(
            params.text_document.uri,
            Doc {
                text: text.to_string(),
                ast: tree.clone(),
            },
        );

        {
            if tree.clone().is_some() {
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("AST: {}", tree.unwrap().root_node()),
                    )
                    .await;
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let docs = self.documents.write();
        docs.unwrap().remove(&params.text_document.uri);
    }

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        let docs = self.documents.read().unwrap();
        match docs.get(&params.text_document.uri) {
            Some(text) => {
                return Ok(DocumentDiagnosticReportResult::Report(
                    DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                        full_document_diagnostic_report: FullDocumentDiagnosticReport {
                            items: self.perform_diagnostics(text.text.clone()),
                            ..FullDocumentDiagnosticReport::default()
                        },
                        ..RelatedFullDocumentDiagnosticReport::default()
                    }),
                ));
            }
            None => {
                return Ok(DocumentDiagnosticReportResult::Report(
                    DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                        full_document_diagnostic_report: FullDocumentDiagnosticReport {
                            items: vec![],
                            ..FullDocumentDiagnosticReport::default()
                        },
                        ..RelatedFullDocumentDiagnosticReport::default()
                    }),
                ))
            }
        };
    }
}
