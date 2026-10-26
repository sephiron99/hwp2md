use super::*;

const NONE: Sides = Sides {
    left: false,
    right: false,
    top: false,
    bottom: false,
};

fn cell(text: &str) -> ir::TableCell {
    ir::TableCell {
        blocks: if text.is_empty() {
            Vec::new()
        } else {
            vec![ir::Block::Paragraph {
                inlines: vec![ir::Inline::plain(text)],
            }]
        },
        colspan: 1,
        rowspan: 1,
    }
}

fn span(text: &str, colspan: u32, rowspan: u32) -> ir::TableCell {
    ir::TableCell {
        colspan,
        rowspan,
        ..cell(text)
    }
}

/// Rows of `(cell, sides)`.
fn table(spec: Vec<Vec<(ir::TableCell, Sides)>>) -> (Vec<ir::TableRow>, Vec<Vec<Option<Sides>>>) {
    let sides = spec
        .iter()
        .map(|r| r.iter().map(|(_, s)| Some(*s)).collect())
        .collect();
    let rows = spec
        .into_iter()
        .enumerate()
        .map(|(i, r)| ir::TableRow {
            cells: r.into_iter().map(|(c, _)| c).collect(),
            is_header: i == 0,
        })
        .collect();
    (rows, sides)
}

/// One line per block: `글` for a paragraph, `표 RxC [cells]` for a table.
fn summary(blocks: &[ir::Block]) -> Vec<String> {
    fn text(blocks: &[ir::Block]) -> String {
        blocks
            .iter()
            .map(|b| match b {
                ir::Block::Paragraph { inlines } => {
                    inlines.iter().map(|i| i.text.as_str()).collect::<String>()
                }
                other => format!("{other:?}"),
            })
            .collect::<Vec<_>>()
            .join("/")
    }
    blocks
        .iter()
        .map(|b| match b {
            ir::Block::Paragraph { .. } => text(std::slice::from_ref(b)),
            ir::Block::Table {
                rows, col_count, ..
            } => {
                let cells: Vec<String> = rows
                    .iter()
                    .map(|r| {
                        r.cells
                            .iter()
                            .map(|c| {
                                let t = text(&c.blocks);
                                if c.colspan > 1 || c.rowspan > 1 {
                                    format!("{t}({}x{})", c.rowspan, c.colspan)
                                } else {
                                    t
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .collect();
                format!("표 {}x{col_count} [{}]", rows.len(), cells.join(" | "))
            }
            other => format!("{other:?}"),
        })
        .collect()
}

fn run(spec: Vec<Vec<(ir::TableCell, Sides)>>) -> Vec<String> {
    let (rows, sides) = table(spec);
    summary(&lay_out(rows, &sides, 0, None))
}

#[test]
fn without_border_information_the_table_is_unchanged() {
    let rows = vec![ir::TableRow {
        cells: vec![cell("가")],
        is_header: true,
    }];
    let out = lay_out(rows, &[vec![None]], 1, None);
    assert_eq!(summary(&out), ["표 1x1 [가]"]);
}

#[test]
fn drawn_grid_stays_a_table() {
    let a = Sides::ALL;
    let out = run(vec![
        vec![(cell("가"), a), (cell("나"), a)],
        vec![(cell("다"), a), (cell("라"), a)],
    ]);
    assert_eq!(out, ["표 2x2 [가,나 | 다,라]"]);
}

#[test]
fn borderless_table_becomes_text_in_reading_order() {
    let out = run(vec![
        vec![(span("왼쪽", 1, 2), NONE), (cell("오른 위"), NONE)],
        vec![(cell("오른 아래"), NONE)],
    ]);
    assert_eq!(out, ["왼쪽", "오른 위", "오른 아래"]);
}

#[test]
fn borderless_title_row_is_text_above_the_drawn_rows() {
    let a = Sides::ALL;
    let out = run(vec![
        vec![(span("제목", 2, 1), NONE)],
        vec![(cell("가"), a), (cell("나"), a)],
        vec![(cell("다"), a), (cell("라"), a)],
    ]);
    assert_eq!(out, ["제목", "표 2x2 [가,나 | 다,라]"]);
}

#[test]
fn one_column_box_is_unwrapped_and_its_table_kept() {
    let a = Sides::ALL;
    let inner = ir::Block::Table {
        rows: vec![ir::TableRow {
            cells: vec![cell("안1"), cell("안2")],
            is_header: true,
        }],
        col_count: 2,
        inner_margin: None,
    };
    let mut boxed = cell("상자 글");
    boxed.blocks.push(inner);
    let out = run(vec![vec![(boxed, a)], vec![(cell("둘째 칸"), a)]]);
    assert_eq!(out, ["상자 글", "표 1x2 [안1,안2]", "둘째 칸"]);
}

#[test]
fn spacer_rows_and_empty_trailing_columns_are_dropped() {
    let a = Sides::ALL;
    let sides_only = Sides {
        left: true,
        right: true,
        ..NONE
    };
    let no_bottom = Sides { bottom: false, ..a };
    let no_top = Sides { top: false, ..a };
    let out = run(vec![
        vec![
            (cell("가"), no_bottom),
            (cell("나"), no_bottom),
            (cell(""), a),
        ],
        vec![
            (cell(""), sides_only),
            (cell(""), sides_only),
            (cell(""), a),
        ],
        vec![(cell("다"), no_top), (cell("라"), no_top), (cell(""), a)],
    ]);
    assert_eq!(out, ["표 2x2 [가,나 | 다,라]"]);
}

#[test]
fn grid_lines_no_cell_uses_are_merged() {
    let a = Sides::ALL;
    let out = run(vec![
        vec![(span("가", 2, 1), a), (cell("나"), a)],
        vec![(span("다", 2, 1), a), (cell("라"), a)],
    ]);
    assert_eq!(out, ["표 2x2 [가,나 | 다,라]"]);
}

#[test]
fn hole_left_by_a_dropped_cell_keeps_later_cells_in_place() {
    // Row 0 starts with an empty borderless indent cell, outside the drawn
    // lines; row 1 has text there.  The gap must not shift 가 and 나 left.
    let a = Sides::ALL;
    let out = run(vec![
        vec![(cell(""), NONE), (cell("가"), a), (cell("나"), a)],
        vec![
            (cell("들여쓰기 글"), NONE),
            (cell("다"), a),
            (cell("라"), a),
        ],
    ]);
    assert_eq!(out, ["표 2x3 [,가,나 | 들여쓰기 글,다,라]"]);
}

#[test]
fn border_fills_from_header() {
    let fills = parse_border_fills(
        r##"<hh:head xmlns:hh="h"><hh:borderFills>
          <hh:borderFill id="3"><hh:leftBorder type="SOLID" color="#000000"/>
            <hh:rightBorder type="NONE" color="#000000"/><hh:topBorder type="SOLID" color="#ffffff"/>
            <hh:bottomBorder type="DOT" color="000000"/></hh:borderFill>
          <hh:borderFill id="4"/>
        </hh:borderFills></hh:head>"##,
    );
    assert_eq!(
        fills["3"],
        Sides {
            left: true,
            right: false,
            top: false,
            bottom: true
        }
    );
    assert!(!fills.contains_key("4") || fills["4"] == NONE);
}
