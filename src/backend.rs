use crate::parser::LspParser;
use crate::CliArgs;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::{env, fs};
use tower_lsp::jsonrpc::Result;
use tower_lsp::{lsp_types, Client};
use tower_lsp::{lsp_types::*, LanguageServer};

pub struct MyRange(pub tree_sitter::Range);

impl From<MyRange> for lsp_types::Range {
    fn from(value: MyRange) -> Self {
        lsp_types::Range {
            start: {
                Position {
                    line: value.0.start_point.row as u32,
                    character: value.0.start_point.column as u32,
                }
            },
            end: {
                Position {
                    line: value.0.end_point.row as u32,
                    character: value.0.end_point.column as u32,
                }
            },
        }
    }
}

pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, String>>>, // To store opened documents
    args: CliArgs,
    push_diagnostics: Arc<RwLock<bool>>,
}

impl Backend {
    pub fn new(client: Client, args: CliArgs) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            args,
            push_diagnostics: Arc::new(RwLock::new(false)),
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
        let used_folders = LspParser::parse_code(source_code, &self.args.varname);
        let available_folders = Backend::get_files(&self.args.suggestionsdir);

        used_folders
            .iter()
            .filter(|used_folder| !available_folders.contains(&used_folder.text))
            .map(|invalid_folder| Diagnostic {
                range: MyRange(invalid_folder.range).into(),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("100".into())),
                source: Some("tsm-language-server".into()),
                message: format!(
                    "'{}' is not a valid folder, valid folders are those in '{}'",
                    invalid_folder.text, self.args.suggestionsdir
                ),
                data: Some(serde_json::value::Value::String(
                    invalid_folder.text.clone(),
                )),
                ..Diagnostic::default()
            })
            .collect()
    }

    fn get_best_matches(user_input: &str, possible_matches: &[&str], top_n: usize) -> Vec<String> {
        let matcher = SkimMatcherV2::default();
        let mut matches_with_scores: Vec<(&str, i64)> = possible_matches
            .iter()
            .filter_map(|&s| matcher.fuzzy_match(s, user_input).map(|score| (s, score)))
            .collect();

        matches_with_scores.sort_by(|a, b| b.1.cmp(&a.1));

        matches_with_scores
            .into_iter()
            .take(top_n)
            .map(|(s, _)| s.to_string())
            .collect::<Vec<String>>()
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
        item.insert_text = Some(label.into());
        Some(item)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let push_diagnostics = params
            .capabilities
            .text_document
            .as_ref()
            .unwrap()
            .publish_diagnostics
            .is_some();

        {
            let mut push_diag = self.push_diagnostics.write().unwrap();
            *push_diag = push_diagnostics;
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                completion_provider: Some(CompletionOptions::default()),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
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
        if let Ok(mut docs) = self.documents.write() {
            docs.clear();
        }

        Ok(())
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let docs = match self.documents.read() {
            Ok(docs) => docs,
            Err(_) => return Ok(None),
        };

        let content = match docs.get(&params.text_document_position.text_document.uri) {
            Some(text) => text,
            None => {
                return Ok(None);
            }
        };

        let all_items: Vec<crate::parser::PositionalText> =
            LspParser::parse_code(content, &self.args.varname);
        let all_completions = all_items
            .iter()
            .find(|item| {
                item.range.start_point.row == params.text_document_position.position.line as usize
                    && (item.range.start_point.column
                        < params.text_document_position.position.character as usize
                        && item.range.end_point.column
                            > params.text_document_position.position.character as usize)
            })
            .map(|_item_at_position| {
                let completions = Backend::get_files(&self.args.suggestionsdir)
                    .iter()
                    .map(|name| name.to_completionitem().unwrap())
                    .collect::<Vec<CompletionItem>>();
                CompletionResponse::Array(completions)
            });

        Ok(all_completions)
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let docs = self.documents.write();
        let uri = params.text_document.uri;

        docs.unwrap()
            .insert(uri.to_owned(), params.text_document.text.clone());

        let push_diagnostics = {
            let push_diag = self.push_diagnostics.read().unwrap();
            *push_diag
        };

        if push_diagnostics {
            self.client
                .publish_diagnostics(
                    uri,
                    self.perform_diagnostics(params.text_document.text.as_str()),
                    None,
                )
                .await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        {
            // capabilities are configured with TextDocumentSyncKind::FULL, so we know that the first change is the full content
            let text = match params.content_changes.first() {
                Some(change) => change.text.clone(),
                None => return,
            };

            if let Ok(mut docs) = self.documents.write() {
                docs.insert(params.text_document.uri.clone(), text);
            }
        }

        let push_diagnostics = {
            let push_diag = self.push_diagnostics.read().unwrap();
            *push_diag
        };

        if push_diagnostics {
            self.client
                .publish_diagnostics(
                    params.text_document.uri,
                    self.perform_diagnostics(params.content_changes.first().unwrap().text.as_str()),
                    None,
                )
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let docs = self.documents.write();
        docs.unwrap().remove(&params.text_document.uri);
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<Vec<CodeActionOrCommand>>> {
        let folders = Backend::get_files(&self.args.suggestionsdir);
        let available_folders: Vec<&str> = folders.iter().map(|s| s.as_str()).collect();
        let mut actions: Vec<CodeActionOrCommand> = Vec::new();

        // Loop through diagnostics in the current document
        for diagnostic in &params.context.diagnostics {
            let data = diagnostic
                .data
                .as_ref()
                .unwrap_or(&serde_json::to_value("").unwrap())
                .clone();
            let user_input = data.as_str().unwrap();

            let best_matches = Backend::get_best_matches(user_input, &available_folders, 15);

            for best_match in best_matches {
                let edit = TextEdit {
                    range: diagnostic.range,
                    new_text: format!("\"{}\"", best_match),
                };

                // Create a workspace edit to apply the text edit
                let edit = WorkspaceEdit {
                    changes: Some(
                        vec![(params.text_document.uri.clone(), vec![edit])]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                };

                // Build the code action with the edit
                let code_action = CodeAction {
                    title: format!("Use folder {}", best_match),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(edit),
                    ..Default::default()
                };

                actions.push(tower_lsp::lsp_types::CodeActionOrCommand::CodeAction(
                    code_action,
                ));
            }
        }

        Ok(Some(actions))
    }
}
