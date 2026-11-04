//! `hwp2md-mcp`: an MCP server, over stdio, that reads Hangul `.hwpx`
//! documents as Markdown for Claude and other MCP clients.
//!
//! ```text
//! claude mcp add hwp2md -- hwp2md-mcp --root ~/문서
//! ```
//!
//! Only files inside the `--root` folders can be read.  Standard output
//! carries the protocol; logs go to standard error.

mod doc;
mod paging;
mod sandbox;
mod server;

use std::path::PathBuf;

use clap::Parser;
use rmcp::ServiceExt;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "MCP server (stdio) that reads Hangul .hwpx documents as Markdown"
)]
struct Args {
    /// Folder the server may read from; repeat for more. Defaults to the
    /// current folder. Relative paths from clients start at the first one.
    #[arg(long = "root", value_name = "DIR")]
    roots: Vec<PathBuf>,
    /// Most characters in one answer: the page size of hwpx_to_markdown
    /// and the size limit of hwpx_tables.
    #[arg(long, default_value_t = 40_000)]
    max_chars: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();
    let roots = if args.roots.is_empty() {
        vec![std::env::current_dir()?]
    } else {
        args.roots
    };
    let sandbox = sandbox::Sandbox::new(&roots)?;
    tracing::info!("serving .hwpx files from {}", sandbox.describe());

    let service = server::Hwp2mdServer::new(sandbox, args.max_chars)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
