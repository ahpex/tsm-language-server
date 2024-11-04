use clap::{arg, command, Parser};
use tower_lsp::{LspService, Server};

mod backend;
use backend::Backend;

mod parser;

/// tsm-language-server
#[derive(Parser, Debug)]
#[command(version,about,long_about=None)]
pub struct CliArgs {
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
