use std::fmt::Write as _;

use crate::convert::TableFormat;
use crate::ir;
use crate::url_util::is_safe_url_scheme;

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[must_use]
pub fn write_markdown(doc: &ir::Document, frontmatter: bool) -> String {
    write_markdown_with_table_format(doc, frontmatter, TableFormat::default())
}

/// Render `doc` to Markdown, choosing how tables with merged cells
/// (`colspan`/`rowspan` > 1) are rendered via `table_format`.
///
/// [`write_markdown`] is a thin wrapper around this function using
/// [`TableFormat::default`] (`Html`), which preserves the exact output of
/// every existing caller.
#[must_use]
pub fn write_markdown_with_table_format(
    doc: &ir::Document,
    frontmatter: bool,
    table_format: TableFormat,
) -> String {
    let mut out = String::new();

    if frontmatter {
        write_frontmatter(&mut out, &doc.metadata);
    }

    for (si, section) in doc.sections.iter().enumerate() {
        if si > 0 {
            out.push_str("\n---\n\n");
        }
        write_section(&mut out, section, table_format);
    }

    out
}

fn write_section(out: &mut String, section: &ir::Section, table_format: TableFormat) {
    // Emit header blocks wrapped in HTML comment markers before main content.
    if let Some(header_blocks) = &section.header {
        if !header_blocks.is_empty() {
            out.push_str("<!-- header -->\n");
            for block in header_blocks {
                write_block(out, block, 0, table_format);
            }
            out.push_str("<!-- /header -->\n\n");
        }
    }

    // Emit footer blocks wrapped in HTML comment markers before main content.
    if let Some(footer_blocks) = &section.footer {
        if !footer_blocks.is_empty() {
            out.push_str("<!-- footer -->\n");
            for block in footer_blocks {
                write_block(out, block, 0, table_format);
            }
            out.push_str("<!-- /footer -->\n\n");
        }
    }

    for block in &section.blocks {
        write_block(out, block, 0, table_format);
    }
}

fn write_frontmatter(out: &mut String, meta: &ir::Metadata) {
    out.push_str("---\n");
    if let Some(ref title) = meta.title {
        let _ = writeln!(out, "title: \"{}\"", escape_yaml(title));
    }
    if let Some(ref author) = meta.author {
        let _ = writeln!(out, "author: \"{}\"", escape_yaml(author));
    }
    if let Some(ref created) = meta.created {
        let _ = writeln!(out, "date: \"{created}\"");
    }
    if let Some(ref subject) = meta.subject {
        let _ = writeln!(out, "subject: \"{}\"", escape_yaml(subject));
    }
    if let Some(ref desc) = meta.description {
        let _ = writeln!(out, "description: \"{}\"", escape_yaml(desc));
    }
    if !meta.keywords.is_empty() {
        let escaped: Vec<String> = meta.keywords.iter().map(|k| escape_yaml(k)).collect();
        let _ = writeln!(out, "keywords: [{}]", escaped.join(", "));
    }
    out.push_str("---\n\n");
}

fn escape_yaml(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn write_block(out: &mut String, block: &ir::Block, indent: usize, table_format: TableFormat) {
    let prefix: String = "  ".repeat(indent);

    match block {
        ir::Block::Heading { level, inlines } => {
            let hashes = "#".repeat(*level as usize);
            let _ = writeln!(out, "{hashes} {}\n", render_inlines(inlines));
        }
        ir::Block::Paragraph { inlines } => {
            let text = render_inlines(inlines);
            if !text.trim().is_empty() {
                let safe = escape_paragraph_line_start(&text);
                let _ = writeln!(out, "{prefix}{safe}\n");
            }
        }
        ir::Block::Table {
            rows, col_count, ..
        } => {
            write_table(out, rows, *col_count, table_format);
        }
        ir::Block::CodeBlock { language, code } => {
            let lang = language.as_deref().unwrap_or("");
            let _ = writeln!(out, "{prefix}```{lang}");
            for line in code.lines() {
                // trim_end mirrors what CommonMark normalises on re-parse; preserves leading indent
                let _ = writeln!(out, "{prefix}{}", line.trim_end());
            }
            let _ = writeln!(out, "{prefix}```\n");
        }
        ir::Block::BlockQuote { blocks } => {
            for b in blocks {
                let mut inner = String::new();
                write_block(&mut inner, b, 0, table_format);
                for line in inner.lines() {
                    let _ = writeln!(out, "{prefix}> {line}");
                }
            }
            out.push('\n');
        }
        ir::Block::List {
            ordered,
            start,
            items,
        } => {
            write_list(out, items, *ordered, *start, indent, table_format);
            out.push('\n');
        }
        ir::Block::Image { src, alt } => {
            let _ = writeln!(out, "{prefix}![{alt}]({src})\n");
        }
        ir::Block::HorizontalRule => {
            let _ = writeln!(out, "{prefix}---\n");
        }
        ir::Block::PageBreak => {
            // HTML comment is invisible in rendered Markdown but survives a
            // round-trip through the parser, which preserves the marker as a
            // `Block::PageBreak`.  See `md::parser::node_to_block` for the
            // matching detection.
            let _ = writeln!(out, "{prefix}<!-- pagebreak -->\n");
        }
        ir::Block::Footnote { id, content } => {
            let _ = write!(out, "{prefix}[^{id}]: ");
            for (i, b) in content.iter().enumerate() {
                if i > 0 {
                    let _ = write!(out, "{prefix}    ");
                }
                let mut inner = String::new();
                write_block(&mut inner, b, 0, table_format);
                out.push_str(inner.trim_end());
                out.push('\n');
            }
            out.push('\n');
        }
        ir::Block::Math { display, tex } => {
            if *display {
                let _ = writeln!(out, "{prefix}$$\n{prefix}{tex}\n{prefix}$$\n");
            } else {
                let _ = writeln!(out, "{prefix}${tex}$\n");
            }
        }
    }
}

/// Escape Markdown metacharacters in plain inline text so they are treated
/// literally by GFM renderers.  Only characters that cause misinterpretation
/// inside a formatted inline span are escaped: backslash, backtick, asterisk,
/// underscore, tilde, and square brackets.  Parentheses are intentionally
/// left unescaped because they only matter when preceded by `]`, which cannot
/// occur in plain text after bracket-escaping.
fn escape_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' | '`' | '*' | '_' | '~' | '[' | ']' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn escape_paragraph_line_start(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if needs_line_start_escape(line) {
            out.push('\\');
        }
        out.push_str(line);
    }
    out
}

fn needs_line_start_escape(line: &str) -> bool {
    if line.starts_with('#') || line.starts_with('>') {
        return true;
    }
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return true;
    }
    if line == "---" || line == "***" || line == "___" {
        return true;
    }
    if let Some(rest) = line.split_once(". ") {
        if rest.0.chars().all(|c| c.is_ascii_digit()) && !rest.0.is_empty() {
            return true;
        }
    }
    false
}

fn render_inlines(inlines: &[ir::Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        // Code spans: wrap raw text in backticks; no other escaping needed.
        if inline.code {
            let _ = write!(out, "`{}`", inline.text);
            continue;
        }

        // Escape Markdown metacharacters in the visible text.
        let mut text = escape_inline(&inline.text);

        // Apply bold/italic only when the text is non-empty; wrapping an empty
        // string with `**` or `*` produces `****` / `**` which most GFM
        // renderers emit as literal asterisks rather than an empty span.
        if !text.is_empty() {
            if inline.bold && inline.italic {
                text = format!("***{text}***");
            } else if inline.bold {
                text = format!("**{text}**");
            } else if inline.italic {
                text = format!("*{text}*");
            }

            if inline.strikethrough {
                text = format!("~~{text}~~");
            }
            if inline.underline {
                text = format!("<u>{text}</u>");
            }
            if inline.superscript {
                text = format!("<sup>{text}</sup>");
            }
            if inline.subscript {
                text = format!("<sub>{text}</sub>");
            }
        }

        // Wrap in a <span> for non-black text color.  The span is applied after
        // all Markdown decoration so that bold/italic markers remain inside it
        // and GFM renderers process them correctly.
        if let Some(ref color) = inline.color {
            if !color.is_empty() && color.bytes().all(|b| b.is_ascii_hexdigit() || b == b'#') {
                text = format!("<span style=\"color:{color}\">{text}</span>");
            }
        }

        if let Some(ref annotation) = inline.ruby {
            if !annotation.is_empty() {
                text = format!("<ruby>{text}<rt>{}</rt></ruby>", escape_html(annotation));
            }
        }

        if let Some(ref url) = inline.link {
            if is_safe_url_scheme(url) {
                if url.contains(')') {
                    text = format!("[{text}](<{url}>)");
                } else {
                    text = format!("[{text}]({url})");
                }
            }
            // Unsafe schemes (e.g. javascript:) are dropped — emit the label only.
        }
        if let Some(ref id) = inline.footnote_ref {
            text = format!("{text}[^{id}]");
        }

        out.push_str(&text);
    }
    out
}

fn write_table(
    out: &mut String,
    rows: &[ir::TableRow],
    col_count: usize,
    table_format: TableFormat,
) {
    if rows.is_empty() {
        return;
    }

    let has_complex = rows
        .iter()
        .any(|r| r.cells.iter().any(|c| c.colspan > 1 || c.rowspan > 1))
        // A pipe table cannot hold a table; HTML can.
        || has_nested_table(rows);

    match table_format {
        TableFormat::Html => {
            if has_complex {
                write_html_table(out, rows, table_format);
            } else {
                write_plain_table(out, rows, col_count, table_format);
            }
        }
        TableFormat::Grid => write_grid_table(out, rows, col_count, table_format),
        TableFormat::Repeat => write_repeat_table(out, rows, col_count, table_format),
    }
}

/// Plain GFM pipe table (no HTML, no spans reflected — used for `Html`-mode
/// tables that have no merged cells, and reused by `Repeat`-mode after
/// merged-cell content has already been duplicated into every logical cell).
fn write_plain_table(
    out: &mut String,
    rows: &[ir::TableRow],
    col_count: usize,
    table_format: TableFormat,
) {
    let cols = col_count.max(rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));

    for (ri, row) in rows.iter().enumerate() {
        out.push('|');
        for ci in 0..cols {
            let cell_text = if ci < row.cells.len() {
                cell_to_text(&row.cells[ci], table_format)
            } else {
                String::new()
            };
            let _ = write!(out, " {cell_text} |");
        }
        out.push('\n');

        if ri == 0 {
            out.push('|');
            for _ in 0..cols {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.push('\n');
}

fn write_html_table(out: &mut String, rows: &[ir::TableRow], table_format: TableFormat) {
    out.push_str("<table>\n");
    for row in rows {
        out.push_str("  <tr>\n");
        let tag = if row.is_header { "th" } else { "td" };
        for cell in &row.cells {
            let mut attrs = String::new();
            if cell.colspan > 1 {
                let _ = write!(attrs, " colspan=\"{}\"", cell.colspan);
            }
            if cell.rowspan > 1 {
                let _ = write!(attrs, " rowspan=\"{}\"", cell.rowspan);
            }
            let text = cell_to_html(cell, table_format);
            let _ = writeln!(out, "    <{tag}{attrs}>{text}</{tag}>");
        }
        out.push_str("  </tr>\n");
    }
    out.push_str("</table>\n\n");
}

/// A logical cell placed on the fine-grained `rows × cols` occupancy grid
/// used by [`write_grid_table`] and [`write_repeat_table`]. `(r0, c0)` is the
/// cell's top-left origin; it occupies `rowspan` rows and `colspan` columns
/// starting there.
struct OccCell {
    r0: usize,
    c0: usize,
    colspan: usize,
    text: String,
}

/// Build a `rows.len() × cols` occupancy grid mapping every fine-grid
/// position to the index of the [`OccCell`] that owns it (`None` for
/// malformed/short rows). Each cell's flattened text is rendered once via
/// [`cell_to_text`] and reused for every position it occupies.
fn build_occupancy_grid(
    rows: &[ir::TableRow],
    cols: usize,
    table_format: TableFormat,
) -> (Vec<Vec<Option<usize>>>, Vec<OccCell>) {
    let row_count = rows.len();
    let mut owner: Vec<Vec<Option<usize>>> = vec![vec![None; cols]; row_count];
    let mut cells: Vec<OccCell> = Vec::new();

    for (ri, row) in rows.iter().enumerate() {
        let mut ci = 0usize;
        for cell in &row.cells {
            while ci < cols && owner[ri][ci].is_some() {
                ci += 1;
            }
            if ci >= cols {
                break;
            }
            let colspan = (cell.colspan.max(1) as usize).min(cols - ci);
            let rowspan = cell.rowspan.max(1) as usize;
            let id = cells.len();
            cells.push(OccCell {
                r0: ri,
                c0: ci,
                colspan,
                text: cell_to_text(cell, table_format),
            });
            for dr in 0..rowspan {
                let r2 = ri + dr;
                if r2 >= row_count {
                    break;
                }
                for dc in 0..colspan {
                    let c2 = ci + dc;
                    if c2 >= cols {
                        break;
                    }
                    owner[r2][c2] = Some(id);
                }
            }
            ci += colspan;
        }
    }

    (owner, cells)
}

/// Render `rows` as a Pandoc-compatible grid table (`+---+---+`, `=====`
/// header separator), correctly merging borders for cells with `colspan` or
/// `rowspan` > 1 per Pandoc's grid-table syntax (see the "Cells can span
/// multiple columns or rows" example in the Pandoc manual). Cell content is
/// rendered on a single line (no in-cell word-wrap); a merged cell's text is
/// shown once, on its own origin row.
fn write_grid_table(
    out: &mut String,
    rows: &[ir::TableRow],
    col_count: usize,
    table_format: TableFormat,
) {
    let cols = col_count.max(rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));
    if cols == 0 || rows.is_empty() {
        return;
    }
    let row_count = rows.len();
    let (owner, cells) = build_occupancy_grid(rows, cols, table_format);

    const MIN_COL_WIDTH: usize = 3;
    let mut col_width = vec![MIN_COL_WIDTH; cols];
    for cell in &cells {
        if cell.colspan == 1 {
            col_width[cell.c0] = col_width[cell.c0].max(cell.text.chars().count());
        }
    }
    // Inflate the last column of a span when the merged cell's own text would
    // otherwise overflow the width already claimed by non-spanning cells.
    //
    // A merged field's text-padding width for a `colspan`-column span is
    // `sum(col_width) + 3 * (colspan - 1)`, not `sum(col_width) + (colspan - 1)`:
    // removing each of the `colspan - 1` internal dividers turns what used to
    // be `<pad><width><pad>|<pad><width><pad>` (2 cell fields + 1 divider =
    // `col_width[a] + col_width[b] + 5`) into one field
    // `<pad><width><pad>` occupying the SAME total line width, so its padded
    // text-width is `col_width[a] + col_width[b] + 5 - 2 = col_width[a] +
    // col_width[b] + 3` — i.e. `sum + 3` per merged boundary, hence the `3 *`
    // factor here (this keeps every border/content line the same total width
    // regardless of how cells are merged, so `+`/`|` positions stay aligned
    // across rows).
    for cell in &cells {
        if cell.colspan > 1 {
            let merged: usize = col_width[cell.c0..cell.c0 + cell.colspan]
                .iter()
                .sum::<usize>()
                + 3 * (cell.colspan - 1);
            let need = cell.text.chars().count();
            if need > merged {
                let last = cell.c0 + cell.colspan - 1;
                col_width[last] += need - merged;
            }
        }
    }

    // Whether the vertical boundary between columns `c-1` and `c` (1..cols)
    // is a genuine cell edge on row `r` (different owners on either side).
    let row_break = |r: usize, c: usize| -> bool { owner[r][c - 1] != owner[r][c] };

    // Draw one horizontal border line. `above`/`below` are the row indices
    // immediately adjacent to this line (`None` at the top/bottom edge of
    // the table). A column's own segment renders blank (not `fill`) when the
    // *same* cell occupies that column on both `above` and `below` — i.e. a
    // rowspan continues straight through this line at that column (see the
    // "Temperature"/"Property" rowspan example in the Pandoc manual, where
    // the continuation column shows blank/wrapped-content, never a dash
    // run). An internal column boundary gets `+` when either adjacent row
    // has a genuine cell edge there; otherwise it continues using whichever
    // fill character its own (matching) segment used.
    let hline = |out: &mut String, above: Option<usize>, below: Option<usize>, fill: char| {
        let seg_fill: Vec<char> = (0..cols)
            .map(|c| {
                let continues = match (above, below) {
                    (Some(a), Some(b)) => owner[a][c].is_some() && owner[a][c] == owner[b][c],
                    _ => false,
                };
                if continues {
                    ' '
                } else {
                    fill
                }
            })
            .collect();

        out.push('+');
        for (c, (width, fc)) in col_width.iter().zip(seg_fill.iter()).enumerate() {
            for _ in 0..*width + 2 {
                out.push(*fc);
            }
            if c + 1 < cols {
                let needs_break = above.is_some_and(|r| row_break(r, c + 1))
                    || below.is_some_and(|r| row_break(r, c + 1));
                out.push(if needs_break { '+' } else { *fc });
            }
        }
        out.push('+');
        out.push('\n');
    };

    hline(out, None, Some(0), '-');
    for (ri, row_owner) in owner.iter().enumerate() {
        out.push('|');
        let mut ci = 0usize;
        while ci < cols {
            match row_owner[ci] {
                Some(id) => {
                    let cell = &cells[id];
                    let span_end = cell.c0 + cell.colspan;
                    let width: usize =
                        col_width[cell.c0..span_end].iter().sum::<usize>() + 3 * (cell.colspan - 1);
                    // Content is shown once, on the cell's own origin row;
                    // continuation rows of a rowspan render blank padding.
                    let text = if cell.r0 == ri {
                        cell.text.as_str()
                    } else {
                        ""
                    };
                    let _ = write!(out, " {text:<width$} ");
                    if span_end < cols {
                        out.push(if row_break(ri, span_end) { '|' } else { ' ' });
                    }
                    ci = span_end;
                }
                None => {
                    let width = col_width[ci];
                    let _ = write!(out, " {:<width$} ", "");
                    if ci + 1 < cols {
                        out.push(if row_break(ri, ci + 1) { '|' } else { ' ' });
                    }
                    ci += 1;
                }
            }
        }
        out.push('|');
        out.push('\n');

        let below = if ri + 1 < row_count {
            Some(ri + 1)
        } else {
            None
        };
        let fill = if ri == 0 { '=' } else { '-' };
        hline(out, Some(ri), below, fill);
    }
    out.push('\n');
}

/// Render `rows` as a plain GFM pipe table with no spans: every cell with
/// `colspan`/`rowspan` > 1 has its text duplicated into each logical grid
/// position it covers, so the output is a fully rectangular table.
fn write_repeat_table(
    out: &mut String,
    rows: &[ir::TableRow],
    col_count: usize,
    table_format: TableFormat,
) {
    let cols = col_count.max(rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));
    if cols == 0 || rows.is_empty() {
        return;
    }
    let (owner, cells) = build_occupancy_grid(rows, cols, table_format);

    for (ri, row_owner) in owner.iter().enumerate() {
        out.push('|');
        for cell_owner in row_owner.iter().take(cols) {
            let cell_text = cell_owner.map_or("", |id| cells[id].text.as_str());
            let _ = write!(out, " {cell_text} |");
        }
        out.push('\n');

        if ri == 0 {
            out.push('|');
            for _ in 0..cols {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.push('\n');
}

fn has_nested_table(rows: &[ir::TableRow]) -> bool {
    rows.iter().any(|r| {
        r.cells.iter().any(|c| {
            c.blocks
                .iter()
                .any(|b| matches!(b, ir::Block::Table { .. }))
        })
    })
}

/// A cell of an HTML table: its text escaped, and any table inside it
/// written as a nested `<table>` in place.
fn cell_to_html(cell: &ir::TableCell, table_format: TableFormat) -> String {
    let mut parts = Vec::new();
    let mut text_run: Vec<ir::Block> = Vec::new();
    let flush_text = |run: &mut Vec<ir::Block>, parts: &mut Vec<String>| {
        if run.is_empty() {
            return;
        }
        let text = blocks_to_text(run, table_format).replace('|', "\\|");
        run.clear();
        if !text.is_empty() {
            parts.push(escape_html(&text));
        }
    };
    for block in &cell.blocks {
        if let ir::Block::Table { rows, .. } = block {
            flush_text(&mut text_run, &mut parts);
            let mut nested = String::new();
            write_html_table(&mut nested, rows, table_format);
            // No blank line inside: it would end the Markdown HTML block.
            parts.push(nested.trim_end().to_string());
        } else {
            text_run.push(block.clone());
        }
    }
    flush_text(&mut text_run, &mut parts);
    parts.join(" ")
}

fn cell_to_text(cell: &ir::TableCell, table_format: TableFormat) -> String {
    blocks_to_text(&cell.blocks, table_format).replace('|', "\\|")
}

/// The text of a cell's blocks on one line.  A table inside (only met
/// outside HTML output, which nests it instead) becomes its cells' text.
fn blocks_to_text(blocks: &[ir::Block], table_format: TableFormat) -> String {
    let mut texts = Vec::new();
    for block in blocks {
        match block {
            ir::Block::Paragraph { inlines } => {
                texts.push(render_inlines(inlines));
            }
            ir::Block::Table { rows, .. } => {
                let cells = rows.iter().flat_map(|r| &r.cells);
                let text: Vec<String> = cells
                    .map(|c| blocks_to_text(&c.blocks, table_format))
                    .filter(|t| !t.is_empty())
                    .collect();
                texts.push(text.join(" "));
            }
            ir::Block::Heading { .. }
            | ir::Block::CodeBlock { .. }
            | ir::Block::BlockQuote { .. }
            | ir::Block::List { .. }
            | ir::Block::Image { .. }
            | ir::Block::HorizontalRule
            | ir::Block::PageBreak
            | ir::Block::Footnote { .. }
            | ir::Block::Math { .. } => {
                let mut s = String::new();
                write_block(&mut s, block, 0, table_format);
                texts.push(s.trim().to_string());
            }
        }
    }
    texts.join(" ")
}

fn write_list(
    out: &mut String,
    items: &[ir::ListItem],
    ordered: bool,
    start: u32,
    indent: usize,
    table_format: TableFormat,
) {
    let prefix_str: String = "  ".repeat(indent);
    for (i, item) in items.iter().enumerate() {
        let base_marker = if ordered {
            format!("{}.", start as usize + i)
        } else {
            "-".to_string()
        };

        // For task list items, append the checkbox token immediately after the
        // bullet/number, e.g. `- [x] ` or `- [ ] `.  For normal items the
        // marker is used unchanged.
        let marker = match item.checked {
            Some(true) => format!("{base_marker} [x]"),
            Some(false) => format!("{base_marker} [ ]"),
            None => base_marker,
        };

        for (bi, block) in item.blocks.iter().enumerate() {
            let mut inner = String::new();
            write_block(&mut inner, block, 0, table_format);
            let inner = inner.trim_end();
            if bi == 0 {
                let _ = writeln!(out, "{prefix_str}{marker} {inner}");
            } else {
                let cont_indent = " ".repeat(marker.len() + 1);
                let _ = writeln!(out, "{prefix_str}{cont_indent}{inner}");
            }
        }

        if !item.children.is_empty() {
            write_list(out, &item.children, ordered, 1, indent + 1, table_format);
        }
    }
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "writer_tests_para.rs"]
mod tests_para;

#[cfg(test)]
#[path = "writer_tests_yaml.rs"]
mod tests_yaml;

#[cfg(test)]
#[path = "writer_tests_inline.rs"]
mod tests_inline;

#[cfg(test)]
#[path = "writer_tests_inline_adv.rs"]
mod tests_inline_adv;

#[cfg(test)]
#[path = "writer_tests_block.rs"]
mod tests_block;

#[cfg(test)]
#[path = "writer_tests_edge.rs"]
mod tests_edge;

#[cfg(test)]
#[path = "writer_tests_frontmatter.rs"]
mod tests_frontmatter;

#[cfg(test)]
#[path = "writer_tests_table_adv.rs"]
mod tests_table_adv;

#[cfg(test)]
#[path = "writer_tests_table_format.rs"]
mod tests_table_format;
