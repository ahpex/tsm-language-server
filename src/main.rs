use clap::{arg, command, Parser};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::sync::{Arc, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug)]
struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, String>>>, // To store opened documents
    args: CliArgs,
}

impl Backend {
    fn new(client: Client, args: CliArgs) -> Self {
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
        docs.unwrap()
            .insert(params.text_document.uri, params.text_document.text);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let docs = self.documents.write();
        docs.unwrap().insert(
            params.text_document.uri,
            params.content_changes.first().unwrap().text.clone(),
        );
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let docs = self.documents.write();
        docs.unwrap().remove(&params.text_document.uri);
    }
}

/// tsm-language-server
#[derive(Parser, Debug)]
#[command(version,about,long_about=None)]
struct CliArgs {
    /// Directory to provide as suggestions
    #[arg(short, long, default_value = ".")]
    suggestionsdir: String,

    /// Prefix to search for in code editor
    #[arg(short, long, default_value = "xyz")]
    prefix: String,

    #[arg(short, long)]
    stdio: bool, // Needed for LSP start
}

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend::new(client, args));
    Server::new(stdin, stdout, socket).serve(service).await;
}
