use crate::parser::LspParser;
use crate::CliArgs;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::{env, fs};
use tower_lsp::jsonrpc::Result;
use tower_lsp::Client;
use tower_lsp::{lsp_types::*, LanguageServer};

pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, String>>>, // To store opened documents
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

    fn get_files(root: &str) -> Vec<String> {
        match fs::read_dir(root) {
            Ok(paths) => paths
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|d| d.file_name().to_string_lossy().to_string())
                .collect::<Vec<String>>(),
            Err(_) => vec![],
        }
    }

    fn perform_diagnostics(&self, source_code: &str) -> Vec<Diagnostic> {
        let used_folders = LspParser::parse_code(source_code);
        let available_folders = Backend::get_files(&self.args.suggestionsdir);

        used_folders
            .iter()
            .filter(|af| !available_folders.contains(af))
            .map(|invalid_folder| Diagnostic {
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
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("100".into())),
                source: Some("tsm-language-server".into()),
                message: format!("'{}' is not a valid folder", invalid_folder),
                ..Diagnostic::default()
            })
            .collect()
    }
}

trait ConvertToCompletionItem {
    fn to_completionitem(&self) -> Option<CompletionItem>;
}

impl ConvertToCompletionItem for String {
    fn to_completionitem(&self) -> Option<CompletionItem> {
        let label = self;
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
        let cli_args: Vec<String> = env::args().collect();

        {
            self.client
                .log_message(
                    MessageType::INFO,
                    format!("Server initialized with arguments {:?}", cli_args),
                )
                .await;
        }
    }

    async fn shutdown(&self) -> Result<()> {
        let docs = self.documents.write();
        docs.unwrap().clear();

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
        let uri = params.text_document.uri;

        docs.unwrap()
            .insert(uri.to_owned(), params.text_document.text.clone());

        let used_folders = LspParser::parse_code(params.text_document.text.as_str());
        let available_folders = Backend::get_files(&self.args.suggestionsdir);

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "Used folders: {:?}\nAvailable folders: {:?}",
                    used_folders, available_folders
                ),
            )
            .await;

        self.client
            .publish_diagnostics(
                uri,
                self.perform_diagnostics(params.text_document.text.as_str()),
                None,
            )
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        {
            let docs = self.documents.write();
            let text = &params.content_changes.first().unwrap().text;

            docs.unwrap()
                .insert(params.text_document.uri.clone(), text.to_string());
        }

        let used_folders = LspParser::parse_code(&params.content_changes.first().unwrap().text);
        let available_folders = Backend::get_files(&self.args.suggestionsdir);

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "Used folders: {:?}\nAvailable folders: {:?}",
                    used_folders, available_folders
                ),
            )
            .await;

        self.client
            .publish_diagnostics(
                params.text_document.uri,
                self.perform_diagnostics(params.content_changes.first().unwrap().text.as_str()),
                None,
            )
            .await;
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

        let diagnostics = match docs.get(&params.text_document.uri) {
            Some(source_code) => self.perform_diagnostics(source_code),
            None => vec![],
        };

        return Ok(DocumentDiagnosticReportResult::Report(
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    items: diagnostics,
                    ..FullDocumentDiagnosticReport::default()
                },
                ..RelatedFullDocumentDiagnosticReport::default()
            }),
        ));
    }
}
