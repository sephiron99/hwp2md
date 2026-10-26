//! Tests for `TableFormat::{Html, Grid, Repeat}` (v0.6.0 configurable table
//! format). Exercises the same 3-column x 2-row IR table with a
//! 2-column-spanning first cell in all three modes, per the deep-interview
//! spec's acceptance criteria (`specs/deep-interview-hwp2md-v060-table-format.md`).

use super::*;
use crate::convert::TableFormat;
use crate::ir;

fn plain(t: &str) -> ir::Inline {
    ir::Inline::plain(t)
}

fn cell(text: &str, colspan: u32) -> ir::TableCell {
    ir::TableCell {
        blocks: vec![ir::Block::Paragraph {
            inlines: vec![plain(text)],
        }],
        colspan,
        rowspan: 1,
    }
}

fn cell_rc(text: &str, colspan: u32, rowspan: u32) -> ir::TableCell {
    ir::TableCell {
        blocks: vec![ir::Block::Paragraph {
            inlines: vec![plain(text)],
        }],
        colspan,
        rowspan,
    }
}

/// A cell spanning 3 rows in column 0, paired with 3 distinct plain cells in
/// column 1 (one per row) — the pure-rowspan case, no colspan involved.
fn rowspan_only_table() -> ir::Document {
    let rows = vec![
        ir::TableRow {
            cells: vec![cell_rc("Tall", 1, 3), cell_rc("R1", 1, 1)],
            is_header: true,
        },
        ir::TableRow {
            cells: vec![cell_rc("R2", 1, 1)],
            is_header: false,
        },
        ir::TableRow {
            cells: vec![cell_rc("R3", 1, 1)],
            is_header: false,
        },
    ];
    ir::Document {
        sections: vec![ir::Section {
            blocks: vec![ir::Block::Table {
                rows,
                col_count: 2,
                inner_margin: None,
            }],
            page_layout: None,
            ..Default::default()
        }],
        ..ir::Document::new()
    }
}

/// 3 columns x 2 rows: row 0 has a colspan=2 cell ("Merged Header") plus a
/// plain cell ("Col C"); row 1 has three plain cells.
fn spanning_table() -> ir::Document {
    let rows = vec![
        ir::TableRow {
            cells: vec![cell("Merged Header", 2), cell("Col C", 1)],
            is_header: true,
        },
        ir::TableRow {
            cells: vec![cell("A", 1), cell("B", 1), cell("C", 1)],
            is_header: false,
        },
    ];
    ir::Document {
        sections: vec![ir::Section {
            blocks: vec![ir::Block::Table {
                rows,
                col_count: 3,
                inner_margin: None,
            }],
            page_layout: None,
            ..Default::default()
        }],
        ..ir::Document::new()
    }
}

// ---------------------------------------------------------------------------
// TableFormat::Html — current/default behaviour, unchanged.
// ---------------------------------------------------------------------------

#[test]
fn table_format_html_renders_html_table_for_colspan() {
    let doc = spanning_table();
    let md = write_markdown_with_table_format(&doc, false, TableFormat::Html);
    assert!(
        md.contains("<table>"),
        "colspan table must render as HTML <table> in Html mode; got: {md:?}"
    );
    assert!(
        md.contains("colspan=\"2\""),
        "HTML table must carry colspan=2 on the merged cell; got: {md:?}"
    );
    assert!(
        md.contains("Merged Header"),
        "merged cell text must appear; got: {md:?}"
    );
    assert!(
        !md.contains('+'),
        "Html mode must not emit grid-table syntax; got: {md:?}"
    );
}

#[test]
fn table_format_html_default_matches_two_arg_write_markdown() {
    let doc = spanning_table();
    let explicit = write_markdown_with_table_format(&doc, false, TableFormat::default());
    let implicit = write_markdown(&doc, false);
    assert_eq!(
        explicit, implicit,
        "write_markdown must be equivalent to write_markdown_with_table_format(.., Html)"
    );
}

// ---------------------------------------------------------------------------
// TableFormat::Grid — Pandoc-compatible grid table syntax.
// ---------------------------------------------------------------------------

#[test]
fn table_format_grid_renders_pandoc_grid_table() {
    let doc = spanning_table();
    let md = write_markdown_with_table_format(&doc, false, TableFormat::Grid);

    assert!(
        md.contains('+') && md.contains('-'),
        "Grid mode must emit `+---+` boundary lines; got: {md:?}"
    );
    assert!(
        md.contains('='),
        "Grid mode must emit a `=====` header separator; got: {md:?}"
    );
    assert!(
        !md.contains("<table>"),
        "Grid mode must not emit HTML; got: {md:?}"
    );
    assert!(
        md.contains("Merged Header"),
        "merged cell text must appear; got: {md:?}"
    );

    // The merged cell's own row must not carry an internal `|` divider
    // between the two columns it spans — locate the content line containing
    // "Merged Header" and confirm no `|` sits between it and the following
    // "Col C" cell (their columns are unmerged, so a `|` MUST separate them
    // from the *next* cell, but not internally under the span).
    let header_line = md
        .lines()
        .find(|l| l.contains("Merged Header"))
        .expect("a content line must contain the merged cell text");
    // Between "Merged Header" and "Col C" there must be exactly ONE `|` — the
    // genuine boundary before the unmerged "Col C" column. If colspan merging
    // were broken (columns rendered independently) there would be TWO `|`
    // characters there instead (one spurious internal divider plus the real
    // one).
    let after_text = header_line
        .split("Merged Header")
        .nth(1)
        .expect("text must be followed by trailing content");
    let boundary_pipes = after_text
        .split("Col C")
        .next()
        .expect("Col C must appear after the merged cell")
        .matches('|')
        .count();
    assert_eq!(
        boundary_pipes, 1,
        "exactly one `|` must separate the merged cell from Col C (internal span \
         boundary must be collapsed); got line: {header_line:?}"
    );

    // The border line immediately below the header row must merge the two
    // spanned columns into one continuous dash run (no interior `+` under
    // the span), while still dividing before the "Col C" column.
    let border_after_header = md
        .lines()
        .skip_while(|l| !l.contains("Merged Header"))
        .nth(1)
        .expect("a border line must follow the header content line");
    assert!(
        border_after_header.starts_with('+') && border_after_header.contains('='),
        "expected a `=====` header separator line right after the header row; got: {border_after_header:?}"
    );
}

// ---------------------------------------------------------------------------
// TableFormat::Repeat — merged content duplicated into each logical cell.
// ---------------------------------------------------------------------------

#[test]
fn table_format_repeat_duplicates_merged_cell_into_both_columns() {
    let doc = spanning_table();
    let md = write_markdown_with_table_format(&doc, false, TableFormat::Repeat);

    assert!(
        !md.contains("<table>"),
        "Repeat mode must not emit HTML; got: {md:?}"
    );
    assert!(
        !md.contains('+'),
        "Repeat mode must not emit grid-table syntax; got: {md:?}"
    );
    assert!(
        md.contains('|'),
        "Repeat mode must be a plain GFM pipe table; got: {md:?}"
    );

    let header_line = md
        .lines()
        .find(|l| l.contains("Merged Header"))
        .expect("a content line must contain the merged cell text");
    let occurrences = header_line.matches("Merged Header").count();
    assert_eq!(
        occurrences, 2,
        "merged cell content must be duplicated into both spanned columns; got line: {header_line:?}"
    );
    assert!(
        header_line.contains("Col C"),
        "the non-spanning cell in the same row must still appear once; got: {header_line:?}"
    );
}

// ---------------------------------------------------------------------------
// TableFormat::Grid — rowspan border-continuation regression (adversarial
// red-team finding: a pure-rowspan column's horizontal separator segments
// must render blank, not a dash run, matching Pandoc's own "Property"/
// "Temperature" rowspan example in the grid-tables manual section).
// ---------------------------------------------------------------------------

#[test]
fn table_format_grid_rowspan_continuation_border_is_blank_not_dashed() {
    let doc = rowspan_only_table();
    let md = write_markdown_with_table_format(&doc, false, TableFormat::Grid);

    let lines: Vec<&str> = md.lines().collect();
    let border_lines: Vec<&&str> = lines.iter().filter(|l| l.starts_with('+')).collect();
    assert_eq!(
        border_lines.len(),
        4,
        "expected 4 border lines (top, header separator, 2 body separators, \
         with the very last one being the bottom edge) for a 3-row table; got: {md:?}"
    );

    // Every INTERIOR border line (i.e. not the very first or very last, which
    // bound the whole table and are always full dash runs) must render the
    // "Tall" column's own segment as blank (no '-'/'=' inside it), since the
    // rowspan continues straight through those lines.
    for line in &border_lines[1..border_lines.len() - 1] {
        let first_plus = line.find('+').expect("line must start with '+'");
        let second_plus = line[first_plus + 1..]
            .find('+')
            .map(|p| p + first_plus + 1)
            .expect("an interior border line must have a second '+' after column 0");
        let col0_segment = &line[first_plus + 1..second_plus];
        assert!(
            col0_segment.chars().all(|c| c == ' '),
            "rowspan-continuation column's segment must be blank on interior \
             border lines, not a dash/equals run; got segment {col0_segment:?} in line {line:?}"
        );
    }

    // The very first (top) and very last (bottom) border lines bound the
    // whole table and must NOT be blank under column 0 (nothing continues
    // above the top edge or below the bottom edge).
    for line in [border_lines[0], border_lines[border_lines.len() - 1]] {
        let first_plus = line.find('+').expect("line must start with '+'");
        let second_plus = line[first_plus + 1..]
            .find('+')
            .map(|p| p + first_plus + 1)
            .expect("the outer border line must have a second '+' after column 0");
        let col0_segment = &line[first_plus + 1..second_plus];
        assert!(
            col0_segment.chars().all(|c| c == '-'),
            "the table's outer top/bottom border must be a full dash run \
             under every column, including the rowspan column; got segment \
             {col0_segment:?} in line {line:?}"
        );
    }
}

#[test]
fn table_inside_a_cell_nests_as_html() {
    let inner = ir::Block::Table {
        rows: vec![ir::TableRow {
            cells: vec![cell("안1", 1), cell("안2", 1)],
            is_header: true,
        }],
        col_count: 2,
        inner_margin: None,
    };
    let outer = ir::TableCell {
        blocks: vec![
            ir::Block::Paragraph {
                inlines: vec![plain("앞 <글>")],
            },
            inner,
        ],
        colspan: 1,
        rowspan: 1,
    };
    let mut doc = ir::Document::new();
    doc.sections.push(ir::Section {
        blocks: vec![ir::Block::Table {
            rows: vec![ir::TableRow {
                cells: vec![outer, cell("옆", 1)],
                is_header: true,
            }],
            col_count: 2,
            inner_margin: None,
        }],
        ..Default::default()
    });

    let html = write_markdown_with_table_format(&doc, false, TableFormat::Html);
    assert!(
        html.contains("<th>앞 &lt;글&gt; <table>"),
        "nested in place: {html}"
    );
    assert!(html.contains("<th>안1</th>"), "inner cells: {html}");
    assert!(
        !html.contains("&lt;table"),
        "inner table must not be escaped: {html}"
    );
    let first = html.find("<table>").expect("outer table");
    assert!(
        !html[first..html.rfind("</table>").expect("end")].contains("\n\n"),
        "no blank line inside the HTML block: {html}"
    );

    // Formats without HTML keep the inner table's text in the cell.
    let grid = write_markdown_with_table_format(&doc, false, TableFormat::Grid);
    assert!(grid.contains("앞 <글> 안1 안2"), "grid: {grid}");
}

#[test]
fn line_break_in_a_cell_does_not_break_the_row() {
    let mut doc = ir::Document::new();
    doc.sections.push(ir::Section {
        blocks: vec![ir::Block::Table {
            rows: vec![ir::TableRow {
                cells: vec![cell("첫 줄\n둘째 줄", 1), cell("옆", 1)],
                is_header: true,
            }],
            col_count: 2,
            inner_margin: None,
        }],
        ..Default::default()
    });
    let pipe = write_markdown_with_table_format(&doc, false, TableFormat::Html);
    assert!(pipe.contains("| 첫 줄<br>둘째 줄 | 옆 |"), "pipe: {pipe}");

    // Merged cells → HTML: the break must not leave a raw blank line either.
    if let ir::Block::Table { rows, .. } = &mut doc.sections[0].blocks[0] {
        rows[0].cells[0] = cell("첫 줄\n\n셋째 줄", 2);
    }
    let html = write_markdown_with_table_format(&doc, false, TableFormat::Html);
    assert!(
        html.contains("<th colspan=\"2\">첫 줄<br><br>셋째 줄</th>"),
        "html: {html}"
    );
}
