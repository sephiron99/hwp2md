//! The MCP tools.  All of them only read files inside the sandbox.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hwp2md::convert::{read_document, TableFormat};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, Json, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::doc::{self, DocInfo, TableOut};
use crate::paging::page;
use crate::sandbox::Sandbox;

/// Most files `list_hwpx` returns in one answer.
const MAX_LISTED: usize = 500;

#[derive(Clone)]
pub struct Hwp2mdServer {
    sandbox: Arc<Sandbox>,
    /// Page size for `hwpx_to_markdown` and the size cap for `hwpx_tables`.
    max_chars: usize,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ToMarkdownArgs {
    /// Path of the `.hwpx` file: absolute, or relative to the first allowed folder.
    pub path: String,
    /// How tables with merged cells are written: `html` (default), `grid` or `repeat`.
    #[serde(default)]
    pub table_format: Option<TableFormatArg>,
    /// Start with a YAML front matter block holding the title and author.
    #[serde(default)]
    pub frontmatter: Option<bool>,
    /// Character offset to start from (from a previous answer's next offset).
    #[serde(default)]
    pub offset: Option<usize>,
    /// Most characters to return (capped by the server's limit).
    #[serde(default)]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TableFormatArg {
    Html,
    Grid,
    Repeat,
}

impl From<TableFormatArg> for TableFormat {
    fn from(f: TableFormatArg) -> Self {
        match f {
            TableFormatArg::Html => TableFormat::Html,
            TableFormatArg::Grid => TableFormat::Grid,
            TableFormatArg::Repeat => TableFormat::Repeat,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathArgs {
    /// Path of the `.hwpx` file: absolute, or relative to the first allowed folder.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TablesArgs {
    /// Path of the `.hwpx` file: absolute, or relative to the first allowed folder.
    pub path: String,
    /// Only this table (its `index`); all tables when absent.
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArgs {
    /// Folder to search: absolute, or relative to the first allowed folder.
    /// Defaults to the first allowed folder.
    #[serde(default)]
    pub dir: Option<String>,
    /// Search subfolders too (default true).
    #[serde(default)]
    pub recursive: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TablesOut {
    /// Number of tables in the document.
    pub total: usize,
    pub tables: Vec<TableOut>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListOut {
    pub files: Vec<FileOut>,
    /// More files matched than `files` holds.
    pub truncated: bool,
    /// Legacy `.hwp` files found; they cannot be read (save them as `.hwpx`).
    pub hwp_skipped: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FileOut {
    pub path: String,
    pub bytes: u64,
}

#[tool_router]
impl Hwp2mdServer {
    pub fn new(sandbox: Sandbox, max_chars: usize) -> Self {
        Self {
            sandbox: Arc::new(sandbox),
            max_chars: max_chars.max(1),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "hwpx_to_markdown",
        description = "Convert a Hangul .hwpx document to Markdown. Long documents come in pages: \
                       when the answer ends with a next offset, call again with that offset.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn hwpx_to_markdown(&self, args: Parameters<ToMarkdownArgs>) -> Result<String, String> {
        let args = args.0;
        let path = self.sandbox.resolve(&args.path)?;
        let format = args.table_format.map_or(TableFormat::Html, Into::into);
        let frontmatter = args.frontmatter.unwrap_or(false);
        let markdown = blocking(move || {
            let doc = read_document(&path).map_err(|e| e.to_string())?;
            Ok(hwp2md::md::write_markdown_with_table_format(
                &doc,
                frontmatter,
                format,
            ))
        })
        .await?;
        let limit = args
            .max_chars
            .map_or(self.max_chars, |m| m.min(self.max_chars));
        let p = page(&markdown, args.offset.unwrap_or(0), limit);
        let mut text = p.text.to_string();
        if let Some(next) = p.next {
            text.push_str(&format!(
                "\n\n[hwp2md: characters {}..{} of {}. Call again with offset={next} for the rest.]",
                p.start, next, p.total
            ));
        }
        Ok(text)
    }

    #[tool(
        name = "hwpx_info",
        description = "Title, author and counts (sections, headings, paragraphs, tables, images, \
                       footnotes, characters) of a .hwpx document, without its text.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn hwpx_info(&self, args: Parameters<PathArgs>) -> Result<Json<DocInfo>, String> {
        let path = self.sandbox.resolve(&args.0.path)?;
        blocking(move || {
            let doc = read_document(&path).map_err(|e| e.to_string())?;
            Ok(Json(doc::info(&doc, path.display().to_string())))
        })
        .await
    }

    #[tool(
        name = "hwpx_tables",
        description = "The tables of a .hwpx document as rows of cells (text, colspan, rowspan). \
                       Only tables drawn on screen count; borderless layout grids are text. \
                       Pass `index` for one table.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn hwpx_tables(&self, args: Parameters<TablesArgs>) -> Result<Json<TablesOut>, String> {
        let args = args.0;
        let path = self.sandbox.resolve(&args.path)?;
        let all = blocking(move || {
            let doc = read_document(&path).map_err(|e| e.to_string())?;
            Ok(doc::tables(&doc))
        })
        .await?;
        let total = all.len();
        let tables: Vec<TableOut> = match args.index {
            Some(i) if i >= total => {
                return Err(format!("no table {i}: the document has {total} tables"))
            }
            Some(i) => all.into_iter().filter(|t| t.index == i).collect(),
            None => all,
        };
        let size = serde_json::to_string(&tables).map_or(0, |s| s.chars().count());
        if size > self.max_chars {
            return Err(format!(
                "the {total} tables are {size} characters, over the limit of {}; \
                 ask for one table with `index` (0..{total})",
                self.max_chars
            ));
        }
        Ok(Json(TablesOut { total, tables }))
    }

    #[tool(
        name = "list_hwpx",
        description = "List the .hwpx files in a folder this server may read.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_hwpx(&self, args: Parameters<ListArgs>) -> Result<Json<ListOut>, String> {
        let args = args.0;
        let dir = match &args.dir {
            Some(d) => self.sandbox.resolve(d)?,
            None => self.sandbox.roots()[0].clone(),
        };
        if !dir.is_dir() {
            return Err(format!("{} is not a folder", dir.display()));
        }
        let recursive = args.recursive.unwrap_or(true);
        let sandbox = Arc::clone(&self.sandbox);
        blocking(move || Ok(Json(list(&dir, recursive, &sandbox)))).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Hwp2mdServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("hwp2md-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(format!(
                "Reads Hangul (Hancom Office) .hwpx documents and converts them to Markdown \
                 for reading. Files are read only from: {}. Paths may be absolute or relative \
                 to the first of these folders. Use list_hwpx to find documents, hwpx_info for \
                 a quick look, hwpx_to_markdown for the text (long documents come in pages), \
                 and hwpx_tables for tables as data. Legacy binary .hwp files are not supported: \
                 ask the user to save them as .hwpx in Hancom Office.",
                self.sandbox.describe()
            ))
    }
}

/// Run blocking conversion work off the async runtime.
async fn blocking<T, F>(work: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("conversion failed: {e}"))?
}

fn list(dir: &Path, recursive: bool, sandbox: &Sandbox) -> ListOut {
    let mut out = ListOut {
        files: Vec::new(),
        truncated: false,
        hwp_skipped: 0,
    };
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            // Follow nothing outside the sandbox (symbolic links included).
            let Ok(path) = sandbox.resolve(&entry.path().to_string_lossy()) else {
                continue;
            };
            if path.is_dir() {
                if recursive {
                    stack.push(path);
                }
                continue;
            }
            match path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref()
            {
                Some("hwpx") => {
                    if out.files.len() == MAX_LISTED {
                        out.truncated = true;
                        continue;
                    }
                    let bytes = entry.metadata().map_or(0, |m| m.len());
                    out.files.push(FileOut {
                        path: path.display().to_string(),
                        bytes,
                    });
                }
                Some("hwp") => out.hwp_skipped += 1,
                _ => {}
            }
        }
    }
    out.files.sort_by(|a, b| a.path.cmp(&b.path));
    out
}
