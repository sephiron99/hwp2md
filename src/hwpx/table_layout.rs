//! Turn an OWPML table into the tables a reader sees.
//!
//! Hancom documents often lay out a page with tables whose cell borders are
//! hidden.  On screen such a table is just text placed in a grid; emitted
//! as a Markdown table it becomes a grid of mostly empty cells that hides
//! the reading order.  So a table read from HWPX is split by its visible
//! cell borders (`<hh:borderFill>` in `header.xml`):
//!
//! * A row is a *table row* when one of its vertical cell edges is drawn,
//!   or when it holds two or more cells and lines run along its whole top
//!   and bottom.  Each run of table rows stays a table; the cells of the
//!   other rows become plain blocks, row by row.
//! * In a kept table, empty cells outside the drawn lines, empty spacer rows
//!   without a line above or below, and empty trailing columns are dropped,
//!   and grid lines no cell corner uses are merged away.
//! * A table left with one column is a box around content, not a grid: its
//!   cells' blocks are emitted instead.
//!
//! A cell whose border fill is unknown (no `borderFillIDRef`, or no
//! `header.xml`) counts as drawn on every side, and a table without any
//! border information is returned unchanged.

use std::collections::{BTreeSet, HashMap};

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::ir;

/// Which sides of a cell draw a line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Sides {
    pub(crate) left: bool,
    pub(crate) right: bool,
    pub(crate) top: bool,
    pub(crate) bottom: bool,
}

impl Sides {
    pub(crate) const ALL: Sides = Sides {
        left: true,
        right: true,
        top: true,
        bottom: true,
    };
}

/// Visible sides of every `<hh:borderFill>` in `header.xml`, by id.  A side
/// is visible when its element is present, its `type` is not `NONE` and its
/// colour is not white.
pub(crate) fn parse_border_fills(header_xml: &str) -> HashMap<String, Sides> {
    let mut fills = HashMap::new();
    let mut current: Option<String> = None;
    let mut reader = Reader::from_str(header_xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.local_name().as_ref() == b"borderFill" => {
                current = attr(e, "id");
                if let Some(id) = &current {
                    fills.insert(id.clone(), Sides::default());
                }
            }
            Ok(Event::End(ref e)) if e.local_name().as_ref() == b"borderFill" => current = None,
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                let Some(sides) = current.as_ref().and_then(|id| fills.get_mut(id)) else {
                    buf.clear();
                    continue;
                };
                let side = match e.local_name().as_ref() {
                    b"leftBorder" => &mut sides.left,
                    b"rightBorder" => &mut sides.right,
                    b"topBorder" => &mut sides.top,
                    b"bottomBorder" => &mut sides.bottom,
                    _ => {
                        buf.clear();
                        continue;
                    }
                };
                let ty = attr(e, "type").unwrap_or_default();
                let color = attr(e, "color").unwrap_or_default();
                let white = color.trim_start_matches('#').eq_ignore_ascii_case("FFFFFF");
                *side = !ty.is_empty() && !ty.eq_ignore_ascii_case("NONE") && !white;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    fills
}

fn attr(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name.as_bytes())
        .map(|a| a.unescape_value().unwrap_or_default().to_string())
}

/// Blocks that replace one parsed table: tables (the visible parts) and the
/// blocks of the cells outside them, in reading order.
///
/// `sides[r][i]` belongs to `rows[r].cells[i]`; `None` means unknown.
pub(crate) fn lay_out(
    rows: Vec<ir::TableRow>,
    sides: &[Vec<Option<Sides>>],
    col_count: usize,
    inner_margin: Option<ir::TableInnerMargin>,
) -> Vec<ir::Block> {
    if sides.iter().flatten().all(Option::is_none) {
        return vec![ir::Block::Table {
            rows,
            col_count,
            inner_margin,
        }];
    }
    let mut grid = Grid::new(rows, sides);
    grid.trim_trailing_cols();
    let board = Board::new(&grid);

    let mut out = Vec::new();
    for (band, r0, r1) in board.row_bands() {
        let starts: Vec<usize> = (0..grid.cells.len())
            .filter(|&i| (r0..=r1).contains(&grid.cells[i].r))
            .collect();
        if !band {
            for i in starts {
                out.extend(grid.take_blocks(i));
            }
            continue;
        }
        let Some(table) = board.visible_table(&grid, r0, r1) else {
            continue;
        };
        if table.cols == 1 {
            for (i, ..) in table.cells {
                out.extend(grid.take_blocks(i));
            }
        } else {
            out.push(table.build(&mut grid, inner_margin.clone()));
        }
        // Cells of the band that were not kept are empty; nothing to emit.
    }
    out
}

/// A cell placed on the table grid.
struct Placed {
    r: usize,
    c: usize,
    rs: usize,
    cs: usize,
    sides: Sides,
    filled: bool,
    cell: Option<ir::TableCell>,
}

struct Grid {
    rows: usize,
    cols: usize,
    /// In reading order: by row, then column.
    cells: Vec<Placed>,
}

impl Grid {
    /// Place cells row by row, skipping positions taken by cells spanning
    /// down from above (the same model as HTML tables).
    fn new(rows: Vec<ir::TableRow>, sides: &[Vec<Option<Sides>>]) -> Grid {
        let n_rows = rows.len();
        let mut taken: Vec<Vec<bool>> = vec![Vec::new(); n_rows];
        let mut cells = Vec::new();
        let mut cols = 0;
        for (r, row) in rows.into_iter().enumerate() {
            let mut c = 0;
            for (i, cell) in row.cells.into_iter().enumerate() {
                while taken[r].get(c).copied().unwrap_or(false) {
                    c += 1;
                }
                let cs = (cell.colspan as usize).max(1);
                let rs = (cell.rowspan as usize).max(1).min(n_rows - r);
                for line in taken.iter_mut().skip(r).take(rs) {
                    if line.len() < c + cs {
                        line.resize(c + cs, false);
                    }
                    line[c..c + cs].fill(true);
                }
                let side = sides
                    .get(r)
                    .and_then(|s| s.get(i))
                    .copied()
                    .flatten()
                    .unwrap_or(Sides::ALL);
                cells.push(Placed {
                    r,
                    c,
                    rs,
                    cs,
                    sides: side,
                    filled: has_content(&cell.blocks),
                    cell: Some(cell),
                });
                c += cs;
                cols = cols.max(c);
            }
        }
        Grid {
            rows: n_rows,
            cols,
            cells,
        }
    }

    /// Drop trailing columns where no cell with content starts.
    fn trim_trailing_cols(&mut self) {
        let mut keep = self.cols;
        while keep > 0 && !self.cells.iter().any(|p| p.c == keep - 1 && p.filled) {
            keep -= 1;
        }
        if keep == 0 || keep == self.cols {
            return;
        }
        self.cells.retain(|p| p.c < keep);
        for p in &mut self.cells {
            p.cs = p.cs.min(keep - p.c);
        }
        self.cols = keep;
    }

    fn take_blocks(&mut self, i: usize) -> Vec<ir::Block> {
        self.cells[i]
            .cell
            .take()
            .map(|c| c.blocks)
            .unwrap_or_default()
            .into_iter()
            .filter(|b| !is_blank(b))
            .collect()
    }
}

/// Which cell covers each grid position, for edge tests.
struct Board {
    rows: usize,
    cols: usize,
    owner: Vec<Option<usize>>,
    sides: Vec<Sides>,
}

impl Board {
    fn new(grid: &Grid) -> Board {
        let mut owner = vec![None; grid.rows * grid.cols];
        for (i, p) in grid.cells.iter().enumerate() {
            for r in p.r..p.r + p.rs {
                for c in p.c..p.c + p.cs {
                    owner[r * grid.cols + c] = Some(i);
                }
            }
        }
        Board {
            rows: grid.rows,
            cols: grid.cols,
            owner,
            sides: grid.cells.iter().map(|p| p.sides).collect(),
        }
    }

    fn at(&self, r: isize, c: isize) -> Option<usize> {
        if r < 0 || c < 0 || r as usize >= self.rows || c as usize >= self.cols {
            return None;
        }
        self.owner[r as usize * self.cols + c as usize]
    }

    fn side(&self, i: Option<usize>, pick: fn(&Sides) -> bool) -> bool {
        i.is_some_and(|i| pick(&self.sides[i]))
    }

    /// Is the vertical line at column boundary `x`, in row `r`, drawn?
    fn v_edge(&self, x: usize, r: usize) -> bool {
        let (r, x) = (r as isize, x as isize);
        let (l, rt) = (self.at(r, x - 1), self.at(r, x));
        l != rt && (self.side(l, |s| s.right) || self.side(rt, |s| s.left))
    }

    /// Is the horizontal line at row boundary `y`, in column `c`, drawn?
    fn h_edge(&self, y: usize, c: usize) -> bool {
        let (y, c) = (y as isize, c as isize);
        let (u, d) = (self.at(y - 1, c), self.at(y, c));
        u != d && (self.side(u, |s| s.bottom) || self.side(d, |s| s.top))
    }

    fn is_table_row(&self, r: usize) -> bool {
        if (0..=self.cols).any(|x| self.v_edge(x, r)) {
            return true;
        }
        let covering: BTreeSet<usize> = (0..self.cols)
            .filter_map(|c| self.owner[r * self.cols + c])
            .collect();
        covering.len() >= 2 && (0..self.cols).all(|c| self.h_edge(r, c) && self.h_edge(r + 1, c))
    }

    /// Runs of rows: `(is a table, first row, last row)`.
    fn row_bands(&self) -> Vec<(bool, usize, usize)> {
        let mut bands: Vec<(bool, usize, usize)> = Vec::new();
        for r in 0..self.rows {
            let band = self.is_table_row(r);
            match bands.last_mut() {
                Some(last) if last.0 == band => last.2 = r,
                _ => bands.push((band, r, r)),
            }
        }
        bands
    }

    /// The table drawn by rows `r0..=r1`.
    fn visible_table(&self, grid: &Grid, r0: usize, r1: usize) -> Option<Layout> {
        // Cells starting in the band, cut at its end: (index, r, c, rs, cs).
        let mut kept: Vec<(usize, usize, usize, usize, usize)> = grid
            .cells
            .iter()
            .enumerate()
            .filter(|(_, p)| (r0..=r1).contains(&p.r))
            .map(|(i, p)| (i, p.r, p.c, p.rs.min(r1 + 1 - p.r), p.cs))
            .collect();
        let filled = |i: usize| grid.cells[i].filled;

        // Columns the drawn lines reach; empty cells beyond them go.
        let mut reach: Option<(usize, usize)> = None;
        let mut widen = |a: usize, b: usize| {
            reach = Some(reach.map_or((a, b), |(x0, x1)| (x0.min(a), x1.max(b))));
        };
        for r in r0..=r1 {
            for x in 0..=self.cols {
                if self.v_edge(x, r) {
                    widen(x, x);
                }
            }
        }
        for y in r0..=r1 + 1 {
            for c in 0..self.cols {
                if self.h_edge(y, c) {
                    widen(c, c + 1);
                }
            }
        }
        kept.retain(|&(i, _, c, _, cs)| {
            filled(i) || reach.is_some_and(|(x0, x1)| c + cs > x0 && c < x1)
        });
        if kept.is_empty() {
            return None;
        }

        // Empty spacer rows with no line above or below are not rows.
        let x0 = kept.iter().map(|k| k.2).min()?;
        let x1 = kept.iter().map(|k| k.2 + k.4).max()?;
        let line_at = |y: usize| (x0..x1).any(|c| self.h_edge(y, c));
        let spacers: Vec<usize> = (r0..=r1)
            .filter(|&r| {
                let mut starting = kept.iter().filter(|k| k.1 == r).peekable();
                starting.peek().is_some()
                    && starting.all(|k| !filled(k.0))
                    && !(line_at(r) && line_at(r + 1))
            })
            .collect();
        if !spacers.is_empty() {
            let shift = |y: usize| y - spacers.iter().filter(|&&s| s < y).count();
            kept = kept
                .into_iter()
                .filter(|k| !spacers.contains(&k.1))
                .map(|(i, r, c, rs, cs)| (i, shift(r), c, shift(r + rs) - shift(r), cs))
                .filter(|k| k.3 > 0)
                .collect();
        }

        // Empty trailing columns of this table.
        let mut right = kept.iter().map(|k| k.2 + k.4).max()?;
        while right > 1 {
            let mut starting = kept.iter().filter(|k| k.2 == right - 1).peekable();
            if starting.peek().is_none() || starting.any(|k| filled(k.0)) {
                break;
            }
            if kept.iter().all(|k| k.2 == right - 1) {
                break;
            }
            kept.retain(|k| k.2 != right - 1);
            for k in &mut kept {
                if k.2 + k.4 == right {
                    k.4 -= 1;
                }
            }
            right -= 1;
        }

        // Merge grid lines no cell corner uses.
        let xs: Vec<usize> = kept
            .iter()
            .flat_map(|k| [k.2, k.2 + k.4])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let ys: Vec<usize> = kept
            .iter()
            .flat_map(|k| [k.1, k.1 + k.3])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let col = |x: usize| xs.binary_search(&x).unwrap_or(0);
        let row = |y: usize| ys.binary_search(&y).unwrap_or(0);
        let mut cells: Vec<(usize, usize, usize, usize, usize)> = kept
            .into_iter()
            .map(|(i, r, c, rs, cs)| {
                (
                    i,
                    row(r),
                    col(c),
                    row(r + rs) - row(r),
                    col(c + cs) - col(c),
                )
            })
            .collect();
        cells.sort_by_key(|k| (k.1, k.2));
        Some(Layout {
            rows: ys.len() - 1,
            cols: xs.len() - 1,
            cells,
        })
    }
}

/// A visible table: its size and its cells as `(index, r, c, rs, cs)`,
/// sorted by row then column.
struct Layout {
    rows: usize,
    cols: usize,
    cells: Vec<(usize, usize, usize, usize, usize)>,
}

impl Layout {
    /// Rebuild HTML-style rows; positions no kept cell covers get an empty
    /// cell so the cells after them stay in their columns.
    fn build(self, grid: &mut Grid, inner_margin: Option<ir::TableInnerMargin>) -> ir::Block {
        let mut covered = vec![false; self.rows * self.cols];
        let mut start: HashMap<(usize, usize), (usize, usize, usize)> = HashMap::new();
        for &(i, r, c, rs, cs) in &self.cells {
            start.insert((r, c), (i, rs, cs));
            for rr in r..r + rs {
                for cc in c..c + cs {
                    covered[rr * self.cols + cc] = true;
                }
            }
        }
        let mut rows = Vec::with_capacity(self.rows);
        for r in 0..self.rows {
            let mut cells = Vec::new();
            let mut c = 0;
            while c < self.cols {
                if let Some(&(i, rs, cs)) = start.get(&(r, c)) {
                    let blocks = grid.cells[i]
                        .cell
                        .take()
                        .map(|x| x.blocks)
                        .unwrap_or_default();
                    cells.push(ir::TableCell {
                        blocks,
                        colspan: cs as u32,
                        rowspan: rs as u32,
                    });
                    c += cs;
                } else if covered[r * self.cols + c] {
                    c += 1; // spanned from a row above
                } else {
                    cells.push(ir::TableCell {
                        blocks: Vec::new(),
                        colspan: 1,
                        rowspan: 1,
                    });
                    c += 1;
                }
            }
            rows.push(ir::TableRow {
                cells,
                is_header: r == 0,
            });
        }
        ir::Block::Table {
            rows,
            col_count: self.cols,
            inner_margin,
        }
    }
}

/// Only whitespace: nothing a reader would see.
fn is_blank(block: &ir::Block) -> bool {
    match block {
        ir::Block::Paragraph { inlines } => inlines
            .iter()
            .all(|i| i.footnote_ref.is_none() && i.ruby.is_none() && i.text.trim().is_empty()),
        _ => false,
    }
}

fn has_content(blocks: &[ir::Block]) -> bool {
    blocks.iter().any(|b| !is_blank(b))
}

#[cfg(test)]
#[path = "table_layout_tests.rs"]
mod tests;
