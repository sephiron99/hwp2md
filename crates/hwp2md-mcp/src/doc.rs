//! Facts about a parsed document: counts for `hwpx_info`, tables for
//! `hwpx_tables`.

use hwp2md::ir::{Block, Document, Inline, TableRow};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Default, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DocInfo {
    pub path: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub sections: usize,
    pub headings: usize,
    pub paragraphs: usize,
    /// Tables the reader keeps (drawn on screen), nested ones included.
    pub tables: usize,
    pub images: usize,
    pub footnotes: usize,
    /// Characters of text in the body, tables and notes.
    pub characters: usize,
}

pub fn info(doc: &Document, path: String) -> DocInfo {
    let mut info = DocInfo {
        path,
        title: doc.metadata.title.clone(),
        author: doc.metadata.author.clone(),
        sections: doc.sections.len(),
        ..DocInfo::default()
    };
    for section in &doc.sections {
        count(&section.blocks, &mut info);
    }
    info
}

fn text_len(inlines: &[Inline]) -> usize {
    inlines.iter().map(|i| i.text.chars().count()).sum()
}

fn count(blocks: &[Block], info: &mut DocInfo) {
    for block in blocks {
        match block {
            Block::Heading { inlines, .. } => {
                info.headings += 1;
                info.characters += text_len(inlines);
            }
            Block::Paragraph { inlines } => {
                info.paragraphs += 1;
                info.characters += text_len(inlines);
            }
            Block::Table { rows, .. } => {
                info.tables += 1;
                for cell in rows.iter().flat_map(|r| &r.cells) {
                    count(&cell.blocks, info);
                }
            }
            Block::CodeBlock { code, .. } => info.characters += code.chars().count(),
            Block::BlockQuote { blocks } => count(blocks, info),
            Block::List { items, .. } => {
                for item in items {
                    count(&item.blocks, info);
                    for child in &item.children {
                        count(&child.blocks, info);
                    }
                }
            }
            Block::Image { .. } => info.images += 1,
            Block::Footnote { content, .. } => {
                info.footnotes += 1;
                count(content, info);
            }
            Block::Math { tex, .. } => info.characters += tex.chars().count(),
            Block::HorizontalRule | Block::PageBreak => {}
        }
    }
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct TableOut {
    /// Position among the document's tables (outer before inner).
    pub index: usize,
    /// Logical grid size, merged cells counted by their span.
    pub rows: usize,
    pub cols: usize,
    /// Rows of cells as written; a merged cell appears once, with its span.
    pub cells: Vec<Vec<CellOut>>,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CellOut {
    /// The cell's text, one line per paragraph; a table inside the cell
    /// shows as `[table N]`, N being that table's `index`.
    pub text: String,
    #[serde(skip_serializing_if = "is_one")]
    pub colspan: u32,
    #[serde(skip_serializing_if = "is_one")]
    pub rowspan: u32,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if signature
fn is_one(n: &u32) -> bool {
    *n == 1
}

/// Every table of the document, outer tables before the tables in their
/// cells.
pub fn tables(doc: &Document) -> Vec<TableOut> {
    let mut out = Vec::new();
    for section in &doc.sections {
        collect(&section.blocks, &mut out);
    }
    out
}

fn collect(blocks: &[Block], out: &mut Vec<TableOut>) {
    for block in blocks {
        match block {
            Block::Table { rows, .. } => {
                table(rows, out);
            }
            Block::BlockQuote { blocks } => collect(blocks, out),
            Block::List { items, .. } => {
                for item in items {
                    collect(&item.blocks, out);
                    for child in &item.children {
                        collect(&child.blocks, out);
                    }
                }
            }
            Block::Footnote { content, .. } => collect(content, out),
            _ => {}
        }
    }
}

/// Add one table (and, after it, the tables in its cells); its index.
fn table(rows: &[TableRow], out: &mut Vec<TableOut>) -> usize {
    let index = out.len();
    out.push(TableOut {
        index,
        rows: rows.len(),
        cols: 0,
        cells: Vec::new(),
    });
    let mut cells = Vec::with_capacity(rows.len());
    let mut cols = 0;
    for row in rows {
        let width: u32 = row.cells.iter().map(|c| c.colspan.max(1)).sum();
        cols = cols.max(width as usize);
        let mut line = Vec::with_capacity(row.cells.len());
        for cell in &row.cells {
            line.push(CellOut {
                text: cell_text(&cell.blocks, out),
                colspan: cell.colspan.max(1),
                rowspan: cell.rowspan.max(1),
            });
        }
        cells.push(line);
    }
    out[index].cols = cols;
    out[index].cells = cells;
    index
}

fn cell_text(blocks: &[Block], out: &mut Vec<TableOut>) -> String {
    let mut lines = Vec::new();
    for block in blocks {
        match block {
            Block::Heading { inlines, .. } | Block::Paragraph { inlines } => {
                let line: String = inlines.iter().map(|i| i.text.as_str()).collect();
                if !line.trim().is_empty() {
                    lines.push(line);
                }
            }
            Block::Table { rows, .. } => {
                let nested = table(rows, out);
                lines.push(format!("[table {nested}]"));
            }
            Block::Image { alt, .. } if alt.is_empty() => lines.push("[image]".into()),
            Block::Image { alt, .. } => lines.push(format!("[image: {alt}]")),
            Block::List { items, .. } => {
                for item in items {
                    lines.push(cell_text(&item.blocks, out));
                }
            }
            Block::BlockQuote { blocks } => lines.push(cell_text(blocks, out)),
            Block::CodeBlock { code, .. } => lines.push(code.clone()),
            Block::Math { tex, .. } => lines.push(tex.clone()),
            Block::Footnote { .. } | Block::HorizontalRule | Block::PageBreak => {}
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hwp2md::ir::{Section, TableCell};

    fn para(text: &str) -> Block {
        Block::Paragraph {
            inlines: vec![Inline::plain(text)],
        }
    }

    fn cell(blocks: Vec<Block>, colspan: u32) -> TableCell {
        TableCell {
            blocks,
            colspan,
            rowspan: 1,
        }
    }

    fn doc(blocks: Vec<Block>) -> Document {
        let mut d = Document::new();
        d.sections.push(Section {
            blocks,
            ..Default::default()
        });
        d
    }

    #[test]
    fn nested_tables_are_listed_after_their_table_and_referenced() {
        let inner = Block::Table {
            rows: vec![TableRow {
                cells: vec![cell(vec![para("안")], 1)],
                is_header: true,
            }],
            col_count: 1,
            inner_margin: None,
        };
        let outer = Block::Table {
            rows: vec![
                TableRow {
                    cells: vec![cell(vec![para("제목")], 2)],
                    is_header: true,
                },
                TableRow {
                    cells: vec![cell(vec![para("가")], 1), cell(vec![para("앞"), inner], 1)],
                    is_header: false,
                },
            ],
            col_count: 2,
            inner_margin: None,
        };
        let d = doc(vec![para("본문"), outer]);

        let t = tables(&d);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].rows, t[0].cols), (2, 2));
        assert_eq!(t[0].cells[0][0].colspan, 2);
        assert_eq!(t[0].cells[1][1].text, "앞\n[table 1]");
        assert_eq!(t[1].cells[0][0].text, "안");

        let i = info(&d, "x.hwpx".into());
        assert_eq!((i.tables, i.paragraphs), (2, 5));
        assert_eq!(i.characters, "본문제목가앞안".chars().count());
    }
}
