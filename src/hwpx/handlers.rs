use crate::heading_style::parse_heading_style;
use crate::ir::{self, InlineFormat};

use super::super::table_layout;
use super::context::{
    apply_charpr_attrs, flush_active_paragraph_scope, flush_cell_paragraph,
    flush_footnote_paragraph, flush_list_item_paragraph, flush_nested_scope,
    flush_paragraph_staged, CodeLangHint, FieldBegin, ParseContext, RubyPart, StagedBlock,
};

// Handles all XML element parse logic; splitting would lose locality.
#[allow(clippy::too_many_lines)]
pub(super) fn handle_start_element(
    local: &str,
    e: &quick_xml::events::BytesStart,
    ctx: &mut ParseContext,
    staged: &mut Vec<StagedBlock>,
) {
    match local {
        "p" | "hp:p" => {
            // A paragraph inside a paragraph (footnote, table cell, text
            // box …): keep the outer one to resume when this one ends.
            if ctx.in_paragraph {
                let saved = ctx.suspend_paragraph();
                ctx.para_stack.push(saved);
            }
            ctx.new_paragraph_id();
            ctx.in_paragraph = true;
            ctx.current_text.clear();
            ctx.current_inlines.clear();
            ctx.heading_level = None;
            ctx.current_para_pr_id = None;
            ctx.current_num_pr_id = None;
            ctx.para_max_font_height = 0;
            ctx.para_max_font_height_bold = false;

            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                let val = attr.unescape_value().unwrap_or_default().to_string();
                match key {
                    "styleIDRef" | "hp:styleIDRef" => {
                        if let Some(level) = parse_hwpx_style_ref(&val) {
                            ctx.heading_level = Some(level);
                        }
                    }
                    "paraPrIDRef" | "hp:paraPrIDRef" => {
                        ctx.current_para_pr_id = Some(val);
                    }
                    "numPrIDRef" | "hp:numPrIDRef" => {
                        ctx.current_num_pr_id = Some(val);
                    }
                    _ => {}
                }
            }
        }
        "run" | "hp:run" => {
            ctx.in_run = true;
            ctx.fmt.reset();
        }
        "charPr" | "hp:charPr" => apply_charpr_attrs(e, ctx),
        "t" | "hp:t" => {
            ctx.in_text = true;
        }
        "tbl" | "hp:tbl" => {
            // Text before a table in the same paragraph (top level or a
            // cell's) comes before it.
            if ctx.in_paragraph {
                if let Some(before) = flush_active_paragraph_scope(ctx) {
                    staged.push(before);
                }
            }
            // A table inside another one: set the outer table aside until
            // this one ends.
            if ctx.table.active {
                let outer = std::mem::take(&mut ctx.table);
                ctx.table_stack.push(outer);
            }
            ctx.table.active = true;
            ctx.table.rows.clear();
            ctx.table.row_sides.clear();
            ctx.table.col_count = 0;
            ctx.table.inner_margin = None;
            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                if key == "colCnt" || key == "hp:colCnt" {
                    if let Ok(n) = attr.unescape_value().unwrap_or_default().parse::<usize>() {
                        ctx.table.col_count = n;
                    }
                }
            }
        }
        "inMargin" | "hp:inMargin" if ctx.table.active => ctx.table.parse_in_margin(e),
        "tr" | "hp:tr" => {
            ctx.table.current_row_cells.clear();
            ctx.table.current_row_sides.clear();
        }
        "tc" | "hp:tc" => {
            ctx.table.current_cell_sides = e
                .attributes()
                .flatten()
                .find(|a| a.key.local_name().as_ref() == b"borderFillIDRef")
                .and_then(|a| {
                    ctx.border_fills
                        .get(a.unescape_value().unwrap_or_default().as_ref())
                })
                .copied();
            ctx.table.in_cell = true;
            ctx.table.cell_blocks.clear();
            ctx.table.cell_inlines.clear();
            ctx.table.cell_text.clear();
            ctx.table.current_colspan = 1;
            ctx.table.current_rowspan = 1;
        }
        "ol" => {
            ctx.list.active = true;
            ctx.list.ordered = true;
            ctx.list.items.clear();
        }
        "ul" => {
            ctx.list.active = true;
            ctx.list.ordered = false;
            ctx.list.items.clear();
        }
        "li" | "hp:li" => {
            ctx.list.in_item = true;
            ctx.list.item_blocks.clear();
            ctx.list.item_inlines.clear();
            ctx.list.item_text.clear();
        }
        "equation" | "hp:equation" | "eqEdit" | "hp:eqEdit" => {
            ctx.in_equation = true;
            ctx.equation_text.clear();
        }
        "fieldBegin" | "hp:fieldBegin" => begin_field(e, ctx),
        "stringParam" | "hp:stringParam" => {
            if let Some(begin) = ctx.field_begin.as_mut() {
                let name = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.local_name().as_ref() == b"name")
                    .map(|a| a.unescape_value().unwrap_or_default().to_string());
                begin.param = name.map(|n| (n, String::new()));
            }
        }
        "ruby" | "hp:ruby" => {
            ctx.in_ruby = true;
            ctx.ruby_base_text.clear();
            ctx.ruby_annotation_text.clear();
            ctx.ruby_current_part = RubyPart::None;
        }
        "rubyText" | "hp:rubyText" => {
            ctx.ruby_current_part = RubyPart::Annotation;
        }
        "baseText" | "hp:baseText" => {
            ctx.ruby_current_part = RubyPart::Base;
        }
        "fn" | "hp:fn" | "footnote" | "hp:footnote" | "en" | "hp:en" | "endnote" | "hp:endnote"
        | "footNote" | "hp:footNote" | "endNote" | "hp:endNote" => {
            let mut id = String::new();
            let mut number = String::new();
            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                let val = || attr.unescape_value().unwrap_or_default().to_string();
                match key {
                    "id" | "hp:id" | "noteId" | "hp:noteId" if id.is_empty() => id = val(),
                    "number" | "hp:number" => number = val(),
                    _ => {}
                }
            }
            // OWPML notes (`<hp:ctrl><hp:footNote number="1">`) sit where
            // the note mark is drawn and have no separate reference: number
            // them and put the reference here.  Endnotes number on their own.
            if matches!(local, "footNote" | "hp:footNote" | "endNote" | "hp:endNote") {
                if id.is_empty() {
                    if number.is_empty() {
                        ctx.unnumbered_notes += 1;
                        number = format!("n{}", ctx.unnumbered_notes);
                    }
                    id = if local.ends_with("endNote") {
                        format!("e{number}")
                    } else {
                        number
                    };
                }
                if ctx.in_paragraph && !ctx.footnote.active {
                    ctx.push_inline(ir::Inline::footnote_ref(id.clone()));
                }
            }
            ctx.footnote.active = true;
            ctx.footnote.id = id;
            ctx.footnote.blocks.clear();
            ctx.footnote.inlines.clear();
            ctx.footnote.text.clear();
        }
        "headerFooter" | "hp:headerFooter" => {
            ctx.header_footer.active = true;
            ctx.header_footer.in_header = false;
            ctx.header_footer.in_footer = false;
            ctx.header_footer.text.clear();
            ctx.header_footer.inlines.clear();
            ctx.header_footer.header_blocks.clear();
            ctx.header_footer.footer_blocks.clear();
            ctx.header_footer.hf_type = None;

            // Parse the `type` attribute (e.g., "both", "even", "odd").
            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                if key == "type" || key == "hp:type" {
                    let val = attr.unescape_value().unwrap_or_default().to_string();
                    ctx.header_footer.hf_type = Some(val.into());
                }
            }
        }
        "header" | "hp:header" if ctx.header_footer.active => {
            ctx.header_footer.in_header = true;
        }
        "footer" | "hp:footer" if ctx.header_footer.active => {
            ctx.header_footer.in_footer = true;
        }
        "secPr" | "hp:secPr" => {
            ctx.page_layout.has_sec_pr = true;
        }
        "pagePr" | "hp:pagePr" => {
            ctx.page_layout.parse_page_pr(e);
        }
        "pageSize" | "hp:pageSize" => {
            ctx.page_layout.parse_page_size(e);
        }
        "margin" | "hp:margin" => {
            ctx.page_layout.parse_margin(e);
        }
        _ => {}
    }
}

// Handles all XML element parse logic; splitting would lose locality.
#[allow(clippy::too_many_lines)]
pub(super) fn handle_end_element(
    local: &str,
    ctx: &mut ParseContext,
    staged: &mut Vec<StagedBlock>,
) {
    match local {
        "p" | "hp:p" => {
            if !flush_nested_scope(ctx) {
                if let Some(sb) = flush_paragraph_staged(ctx) {
                    staged.push(sb);
                }
            }
            if let Some(outer) = ctx.para_stack.pop() {
                ctx.resume_paragraph(outer);
            } else {
                ctx.in_paragraph = false;
                staged.extend(ctx.after_paragraph.drain(..).map(StagedBlock::Plain));
            }
        }
        "run" | "hp:run" => {
            ctx.in_run = false;
        }
        "t" | "hp:t" => {
            ctx.in_text = false;
            // Drain from the correct buffer.  Header/footer paragraphs route
            // text into `header_footer.text`; every other context uses
            // `current_text`.
            let text = if ctx.header_footer.in_either_active() {
                std::mem::take(&mut ctx.header_footer.text)
            } else {
                std::mem::take(&mut ctx.current_text)
            };
            if !text.is_empty() {
                let inline = ir::Inline::with_formatting(text, &InlineFormat::from(&ctx.fmt))
                    .with_font_name(ctx.fmt.font_name.clone())
                    .with_link(if ctx.in_hyperlink {
                        ctx.hyperlink_url.clone()
                    } else {
                        None
                    });
                ctx.push_inline(inline);
            }
        }
        "tbl" | "hp:tbl" => {
            let col_count = ctx.table.col_count.max(
                ctx.table
                    .rows
                    .iter()
                    .map(|r| r.cells.len())
                    .max()
                    .unwrap_or(0),
            );
            let inner_margin = ctx.table.inner_margin.take();
            let rows = std::mem::take(&mut ctx.table.rows);
            let sides = std::mem::take(&mut ctx.table.row_sides);
            ctx.table.active = false;
            // The tables a reader sees, and the text of borderless parts.
            let blocks = if rows.is_empty() {
                Vec::new()
            } else {
                table_layout::lay_out(rows, &sides, col_count, inner_margin)
            };
            if let Some(outer) = ctx.table_stack.pop() {
                // Back in the outer table; this one belongs to its open cell.
                ctx.table = outer;
                for block in blocks {
                    if let Some(block) = ctx.push_block_scoped(block) {
                        staged.push(StagedBlock::Plain(block));
                    }
                }
            } else {
                staged.extend(blocks.into_iter().map(StagedBlock::Plain));
            }
        }
        "tr" | "hp:tr" => {
            let cells = std::mem::take(&mut ctx.table.current_row_cells);
            ctx.table.rows.push(ir::TableRow {
                cells,
                is_header: ctx.table.rows.is_empty(),
            });
            let sides = std::mem::take(&mut ctx.table.current_row_sides);
            ctx.table.row_sides.push(sides);
        }
        "tc" | "hp:tc" => {
            flush_cell_paragraph(ctx, CodeLangHint::Plain);
            let blocks = std::mem::take(&mut ctx.table.cell_blocks);
            ctx.table.current_row_cells.push(ir::TableCell {
                blocks,
                colspan: ctx.table.current_colspan,
                rowspan: ctx.table.current_rowspan,
            });
            let sides = ctx.table.current_cell_sides.take();
            ctx.table.current_row_sides.push(sides);
            ctx.table.in_cell = false;
        }
        "li" | "hp:li" => {
            flush_list_item_paragraph(ctx, CodeLangHint::Plain);
            let blocks = std::mem::take(&mut ctx.list.item_blocks);
            ctx.list.items.push(ir::ListItem {
                blocks,
                children: Vec::new(),
                checked: None,
            });
            ctx.list.in_item = false;
        }
        "ol" | "ul" => {
            if !ctx.list.items.is_empty() {
                let items = std::mem::take(&mut ctx.list.items);
                staged.push(StagedBlock::Plain(ir::Block::List {
                    ordered: ctx.list.ordered,
                    start: 1,
                    items,
                }));
            }
            ctx.list.active = false;
        }
        "equation" | "hp:equation" | "eqEdit" | "hp:eqEdit" => {
            if !ctx.equation_text.is_empty() {
                // DESIGN: HWPX equation text is stored verbatim as `tex` without
                // any EQEDIT → LaTeX conversion.  A Hancom-authored HWPX file can
                // contain EQEDIT script syntax (e.g. "a over b") that is NOT
                // valid LaTeX; we pass it through untransformed.
                // See the integration test `hwpx_equation_eqedit_syntax_stored_verbatim_not_converted`
                // which pins this behavior.
                let tex = std::mem::take(&mut ctx.equation_text);
                staged.push(StagedBlock::Plain(ir::Block::Math { display: true, tex }));
            }
            ctx.in_equation = false;
        }
        "stringParam" | "hp:stringParam" => {
            if let Some(begin) = ctx.field_begin.as_mut() {
                match begin.param.take() {
                    Some((name, text)) if name == "Direction" && begin.direction.is_none() => {
                        begin.direction = Some(text);
                    }
                    Some((name, text)) if name == "Command" => {
                        if let Some(guide) = command_guide(&text) {
                            begin.command_guide = Some(guide);
                        }
                    }
                    _ => {}
                }
            }
        }
        "fieldBegin" | "hp:fieldBegin" => ctx.open_field(),
        "fieldEnd" | "hp:fieldEnd" => {
            ctx.in_hyperlink = false;
            ctx.hyperlink_url = None;
            ctx.close_field();
        }
        "header" | "hp:header" => {
            ctx.header_footer.in_header = false;
        }
        "footer" | "hp:footer" => {
            ctx.header_footer.in_footer = false;
        }
        "headerFooter" | "hp:headerFooter" => {
            ctx.header_footer.active = false;
        }
        "rubyText" | "hp:rubyText" | "baseText" | "hp:baseText" => {
            ctx.ruby_current_part = RubyPart::None;
        }
        "ruby" | "hp:ruby" => {
            let base = std::mem::take(&mut ctx.ruby_base_text);
            let annotation = std::mem::take(&mut ctx.ruby_annotation_text);
            if !base.is_empty() || !annotation.is_empty() {
                let inline = ir::Inline::with_formatting(base, &InlineFormat::from(&ctx.fmt))
                    .with_ruby(if annotation.is_empty() {
                        None
                    } else {
                        Some(annotation)
                    });
                ctx.push_inline(inline);
            }
            ctx.in_ruby = false;
            ctx.ruby_current_part = RubyPart::None;
        }
        "fn" | "hp:fn" | "footnote" | "hp:footnote" | "en" | "hp:en" | "endnote" | "hp:endnote"
        | "footNote" | "hp:footNote" | "endNote" | "hp:endNote" => {
            flush_footnote_paragraph(ctx, CodeLangHint::Plain);
            if ctx.footnote.blocks.is_empty() {
                ctx.footnote.id.clear();
            } else {
                let id = std::mem::take(&mut ctx.footnote.id);
                let content = std::mem::take(&mut ctx.footnote.blocks);
                let note = ir::Block::Footnote { id, content };
                // A note read inside a paragraph goes after that paragraph.
                if ctx.in_paragraph {
                    ctx.after_paragraph.push(note);
                } else {
                    staged.push(StagedBlock::Plain(note));
                }
            }
            ctx.footnote.active = false;
        }
        _ => {}
    }
}

pub(super) fn handle_text(text: &str, ctx: &mut ParseContext) {
    if let Some((_, param)) = ctx.field_begin.as_mut().and_then(|b| b.param.as_mut()) {
        param.push_str(text);
        return;
    }
    if ctx.in_equation {
        ctx.equation_text.push_str(text);
        return;
    }
    if ctx.in_ruby {
        match ctx.ruby_current_part {
            RubyPart::Base => ctx.ruby_base_text.push_str(text),
            RubyPart::Annotation => ctx.ruby_annotation_text.push_str(text),
            RubyPart::None => {}
        }
        return;
    }
    if ctx.in_run && ctx.in_text {
        ctx.active_text_buf().push_str(text);
    }
}

// Handles all XML element parse logic; splitting would lose locality.
#[allow(clippy::too_many_lines)]
pub(super) fn handle_empty_element(
    local: &str,
    e: &quick_xml::events::BytesStart,
    ctx: &mut ParseContext,
    staged: &mut Vec<StagedBlock>,
) {
    match local {
        "img" | "hp:img" | "picture" | "hp:picture" => {
            let mut src = String::new();
            let mut alt = String::new();
            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                let val = attr.unescape_value().unwrap_or_default().to_string();
                match key {
                    "src" | "href" | "hp:href" | "binaryItemIDRef" | "hp:binaryItemIDRef" => {
                        src = val;
                    }
                    "alt" => alt = val,
                    _ => {}
                }
            }
            if !src.is_empty() {
                let img = ir::Block::Image { src, alt };
                if let Some(block) = ctx.push_block_scoped(img) {
                    staged.push(StagedBlock::Plain(block));
                }
            }
        }
        "lineBreak" | "hp:lineBreak" => {
            ctx.active_text_buf().push('\n');
        }
        "cellAddr" | "hp:cellAddr" => {
            // `cellAddr` carries `colAddr`/`rowAddr` plus span attrs in older
            // HWPX files.  Read spans here for backward compatibility; when
            // `cellSpan` is also present it comes after `cellAddr` per OWPML
            // element order, so its values overwrite these safely.
            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                let val = attr.unescape_value().unwrap_or_default();
                match key {
                    "colSpan" | "hp:colSpan" => {
                        if let Ok(n) = val.parse::<u32>() {
                            if n >= 1 {
                                ctx.table.current_colspan = n;
                            }
                        }
                    }
                    "rowSpan" | "hp:rowSpan" => {
                        if let Ok(n) = val.parse::<u32>() {
                            if n >= 1 {
                                ctx.table.current_rowspan = n;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        "cellSpan" | "hp:cellSpan" => {
            // Authoritative colspan/rowspan per the OWPML spec.
            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                let val = attr.unescape_value().unwrap_or_default();
                match key {
                    "colSpan" | "hp:colSpan" => {
                        if let Ok(n) = val.parse::<u32>() {
                            if n >= 1 {
                                ctx.table.current_colspan = n;
                            }
                        }
                    }
                    "rowSpan" | "hp:rowSpan" => {
                        if let Ok(n) = val.parse::<u32>() {
                            if n >= 1 {
                                ctx.table.current_rowspan = n;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        "inMargin" | "hp:inMargin" if ctx.table.active => ctx.table.parse_in_margin(e),
        "charPr" | "hp:charPr" => apply_charpr_attrs(e, ctx),
        "fieldBegin" | "hp:fieldBegin" => {
            begin_field(e, ctx);
            ctx.open_field();
        }
        "fieldEnd" | "hp:fieldEnd" => {
            ctx.in_hyperlink = false;
            ctx.hyperlink_url = None;
            ctx.close_field();
        }
        "noteRef" | "hp:noteRef" => {
            let mut note_id = String::new();
            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                if key == "noteId" || key == "hp:noteId" || key == "id" || key == "hp:id" {
                    note_id = attr.unescape_value().unwrap_or_default().to_string();
                    break;
                }
            }
            if !note_id.is_empty() {
                ctx.push_inline(ir::Inline::footnote_ref(note_id));
            }
        }
        "ctrl" | "hp:ctrl" => {
            let mut ctrl_kind = String::new();
            let mut id_ref = String::new();
            for attr in e.attributes().flatten() {
                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                let val = attr.unescape_value().unwrap_or_default().to_string();
                match key {
                    "id" | "hp:id" => ctrl_kind = val,
                    "idRef" | "hp:idRef" => id_ref = val,
                    _ => {}
                }
            }
            if (ctrl_kind == "fn" || ctrl_kind == "en") && !id_ref.is_empty() {
                ctx.push_inline(ir::Inline::footnote_ref(id_ref));
            } else if is_page_break_ctrl(&ctrl_kind) {
                // Forced page break.  First flush whatever inline run is
                // pending in the active scope (top-level / footnote /
                // list-item / table-cell) so that mid-paragraph ordering
                // `text · ctrl · text` survives as
                // `Paragraph(text), PageBreak, Paragraph(text)` rather
                // than being merged into a single paragraph.
                if let Some(pending) = flush_active_paragraph_scope(ctx) {
                    staged.push(pending);
                }
                let pb = ir::Block::PageBreak;
                if let Some(block) = ctx.push_block_scoped(pb) {
                    staged.push(StagedBlock::Plain(block));
                }
            }
        }
        "pageSize" | "hp:pageSize" => {
            ctx.page_layout.parse_page_size(e);
        }
        "margin" | "hp:margin" => {
            ctx.page_layout.parse_margin(e);
        }
        _ => {}
    }
}

/// `<hp:fieldBegin>`: a hyperlink starts its link; every field starts
/// collecting its parameters (read until `</hp:fieldBegin>`).
fn begin_field(e: &quick_xml::events::BytesStart, ctx: &mut ParseContext) {
    let mut field_type = String::new();
    let mut command = String::new();
    let mut dirty = false;
    for attr in e.attributes().flatten() {
        let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
        let val = attr.unescape_value().unwrap_or_default().to_string();
        match key {
            "type" | "hp:type" => field_type = val,
            "command" | "hp:command" => command = val,
            "dirty" | "hp:dirty" => dirty = val == "1",
            _ => {}
        }
    }
    if field_type == "HYPERLINK" && !command.is_empty() {
        ctx.in_hyperlink = true;
        ctx.hyperlink_url = Some(command);
    }
    ctx.field_begin = Some(FieldBegin {
        click_here: field_type.eq_ignore_ascii_case("CLICK_HERE") && !dirty,
        ..FieldBegin::default()
    });
}

/// The guide inside a click-here `Command` parameter:
/// `…Direction:wstring:<n>:<n characters>…`.
fn command_guide(command: &str) -> Option<String> {
    const KEY: &str = "Direction:wstring:";
    let mut rest = command;
    while let Some(at) = rest.find(KEY) {
        rest = &rest[at + KEY.len()..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(':') {
            let n: usize = rest[..digits].parse().ok()?;
            let guide: String = rest[digits + 1..].chars().take(n).collect();
            return Some(guide).filter(|g| !g.is_empty());
        }
    }
    None
}

/// Recognise the `id` attribute of an `<hp:ctrl/>` element that signals a
/// forced page break.
///
/// Hancom Office and the OWPML reference accept several spellings depending
/// on the producing tool; we treat any of `newPage`, `pageBreak`, or `cnpb`
/// (column / new-paragraph break) as equivalent so that documents originating
/// from third-party converters round-trip correctly.
fn is_page_break_ctrl(id: &str) -> bool {
    matches!(id, "newPage" | "pageBreak" | "cnpb")
}

/// Parse an HWPX `styleIDRef` attribute value as a heading level (1–6).
///
/// HWPX `styleIDRef` values can be either:
/// - A style name string (e.g. `"Heading1"`, `"개요 2"`) — delegated to the
///   shared [`parse_heading_style`] function.
/// - A bare numeric style-ID string (e.g. `"2"`) where the value directly
///   encodes the heading level.
///
/// Returns `Some(level)` if recognized as a heading level in 1–6, `None`
/// otherwise.
fn parse_hwpx_style_ref(style_ref: &str) -> Option<u8> {
    // Try the shared style-name parser first.
    if let Some(level) = parse_heading_style(style_ref) {
        return Some(level);
    }
    // Fall back to bare numeric level for documents that use integer IDs.
    if let Ok(n) = style_ref.parse::<u8>() {
        if (1..=6).contains(&n) {
            return Some(n);
        }
    }
    None
}

#[cfg(test)]
mod style_ref_tests {
    use super::*;

    #[test]
    fn numeric_in_range_maps_to_level() {
        assert_eq!(parse_hwpx_style_ref("2"), Some(2));
        assert_eq!(parse_hwpx_style_ref("6"), Some(6));
    }

    #[test]
    fn numeric_out_of_range_is_none() {
        assert_eq!(parse_hwpx_style_ref("0"), None);
        assert_eq!(parse_hwpx_style_ref("7"), None);
        assert_eq!(parse_hwpx_style_ref("10"), None);
    }

    #[test]
    fn name_takes_precedence_over_numeric() {
        assert_eq!(parse_hwpx_style_ref("Outline 3"), Some(3));
        assert_eq!(parse_hwpx_style_ref("heading 2"), Some(2));
    }

    #[test]
    fn empty_and_garbage_is_none() {
        assert_eq!(parse_hwpx_style_ref(""), None);
        assert_eq!(parse_hwpx_style_ref("body"), None);
        assert_eq!(parse_hwpx_style_ref("abc123"), None);
    }
}
