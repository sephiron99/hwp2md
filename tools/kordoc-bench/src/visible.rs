//! Visible tables: one HWPX `<hp:tbl>` split into the tables Hancom draws.
//!
//! Hancom documents often lay out a page with a table whose borders are
//! hidden, so the number of `<hp:tbl>` elements is not the number of tables
//! a reader sees.  This follows the definition of kordoc's
//! `bench/ref/visible-tables.mjs` (written independently from it):
//!
//! 1. A unit edge is visible when either cell touching it draws that side
//!    (borderFill side `type≠NONE`, colour ≠ `#FFFFFF`).  The inside of a
//!    merged cell is not an edge.
//! 2. Fractions: two stacked one-row cells over the same columns, each a
//!    single short text-only paragraph, with a visible line between them
//!    that does not run on into the next column, and no other visible side,
//!    merge into one cell.  Two full-width cells only count when the table
//!    holds nothing else.
//! 3. Row bands: a row is a table row when any of its vertical unit edges is
//!    visible, or when two or more cells cover it and its top and bottom
//!    lines are visible across the whole width.  Each run of table rows is
//!    one visible table; the other rows are plain text.
//! 4. A visible table keeps the cells that start in its run (spans cut at the
//!    run's end), drops empty cells outside the columns its visible edges
//!    reach, drops empty spacer rows without a line above or below, trims
//!    empty trailing columns and collapses grid lines no cell corner uses.
//!
//! Before all this, empty trailing columns of the whole table are trimmed.
//! Tables nested in cells are handled by the caller, one table at a time.

use std::collections::{BTreeSet, HashMap, HashSet};

/// Which sides of a cell draw a visible line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sides {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

/// One `<hp:tc>` as written in the file.
#[derive(Clone, Debug, Default)]
pub struct RawCell {
    /// Index of the `<hp:tr>` holding it.
    pub tr: usize,
    pub row_addr: Option<usize>,
    pub col_addr: Option<usize>,
    pub row_span: usize,
    pub col_span: usize,
    pub sides: Sides,
    /// Holds text or an object (image, equation, non-empty nested table …).
    pub content: bool,
    /// Could be a fraction term: one text-only paragraph of 1–40 characters.
    pub fraction_term: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// `(rows, cols)` of each visible table, top to bottom.
    pub tables: Vec<(u32, u32)>,
    /// Per raw cell: `Some(true)` in a table row, `Some(false)` in a text
    /// row, `None` when trimmed away as part of an empty trailing column.
    pub band: Vec<Option<bool>>,
}

pub fn layout(cells: &[RawCell], tr_count: usize) -> Layout {
    let mut band = vec![None; cells.len()];
    if cells.is_empty() {
        return Layout {
            tables: Vec::new(),
            band,
        };
    }
    let (rows, mut cols, mut anchors) = build_grid(cells, tr_count);
    trim_trailing_cols(&mut anchors, &mut cols);

    let mut board = Board::new(rows, cols, &anchors);
    board.merge_fractions();

    let segments = board.row_bands();
    for a in &anchors {
        let in_band = segments
            .iter()
            .find(|s| (s.r0..=s.r1).contains(&(a.r as isize)))
            .map(|s| s.band);
        for &m in &a.members {
            band[m] = in_band;
        }
    }
    let tables = segments
        .iter()
        .filter(|s| s.band)
        .filter_map(|s| board.visible_table(s.r0, s.r1))
        .collect();
    Layout { tables, band }
}

/// A grid position owned by one or more raw cells (the first one wins).
struct Anchor {
    r: usize,
    c: usize,
    rs: usize,
    cs: usize,
    sides: Sides,
    content: bool,
    fraction_term: bool,
    members: Vec<usize>,
}

impl Anchor {
    fn new(i: usize, cell: &RawCell, r: usize, c: usize, rs: usize, cs: usize) -> Self {
        Anchor {
            r,
            c,
            rs,
            cs,
            sides: cell.sides,
            content: cell.content,
            fraction_term: cell.fraction_term,
            members: vec![i],
        }
    }
}

/// Place the cells: by `cellAddr` when the table has addresses, otherwise by
/// walking rows left to right around cells spanning down from above.
fn build_grid(cells: &[RawCell], tr_count: usize) -> (usize, usize, Vec<Anchor>) {
    let mut anchors: Vec<Anchor> = Vec::new();
    let (mut rows, mut cols) = (0, 0);
    if cells
        .iter()
        .any(|c| c.row_addr.is_some() && c.col_addr.is_some())
    {
        let mut owner: HashMap<(usize, usize), usize> = HashMap::new();
        for (i, cell) in cells.iter().enumerate() {
            let r = cell.row_addr.unwrap_or(cell.tr);
            let mut c = cell.col_addr.unwrap_or(0);
            if cell.col_addr.is_none() {
                while owner.contains_key(&(r, c)) {
                    c += 1;
                }
            }
            // Two cells claiming one position: the first keeps it, the
            // second's content joins it.
            if let Some(&a) = owner.get(&(r, c)) {
                let a = &mut anchors[a];
                a.content |= cell.content;
                a.fraction_term = false;
                a.members.push(i);
                continue;
            }
            let mut cs = cell.col_span.max(1);
            if let Some(dc) = (1..cs).find(|dc| owner.contains_key(&(r, c + dc))) {
                cs = dc;
            }
            let mut rs = cell.row_span.max(1);
            if let Some(dr) =
                (1..rs).find(|dr| (0..cs).any(|dc| owner.contains_key(&(r + dr, c + dc))))
            {
                rs = dr;
            }
            for dr in 0..rs {
                for dc in 0..cs {
                    owner.insert((r + dr, c + dc), anchors.len());
                }
            }
            anchors.push(Anchor::new(i, cell, r, c, rs, cs));
            rows = rows.max(r + rs);
            cols = cols.max(c + cs);
        }
    } else {
        rows = cells
            .iter()
            .map(|c| c.tr + 1)
            .max()
            .unwrap_or(0)
            .max(tr_count);
        let mut occupied = vec![Vec::<bool>::new(); rows];
        let mut current = usize::MAX;
        let mut c = 0;
        for (i, cell) in cells.iter().enumerate() {
            let r = cell.tr;
            if r != current {
                current = r;
                c = 0;
            }
            while occupied[r].get(c).copied().unwrap_or(false) {
                c += 1;
            }
            let (rs, cs) = (cell.row_span.max(1), cell.col_span.max(1));
            anchors.push(Anchor::new(i, cell, r, c, rs, cs));
            for row in occupied.iter_mut().skip(r).take(rs) {
                if row.len() < c + cs {
                    row.resize(c + cs, false);
                }
                row[c..c + cs].fill(true);
            }
            c += cs;
            cols = cols.max(c);
        }
    }
    (rows, cols, anchors)
}

/// Drop trailing columns where no cell with content starts.
fn trim_trailing_cols(anchors: &mut Vec<Anchor>, cols: &mut usize) {
    let mut keep = *cols;
    while keep > 0 && !anchors.iter().any(|a| a.c == keep - 1 && a.content) {
        keep -= 1;
    }
    if keep > 0 && keep < *cols {
        anchors.retain(|a| a.c < keep);
        for a in anchors.iter_mut() {
            a.cs = a.cs.min(keep - a.c);
        }
        *cols = keep;
    }
}

#[derive(Clone, Copy)]
struct Cell {
    r: isize,
    c: isize,
    rs: isize,
    cs: isize,
    sides: Sides,
    empty: bool,
    fraction_term: bool,
    fraction: bool,
    dead: bool,
}

struct Segment {
    band: bool,
    r0: isize,
    r1: isize,
}

/// The grid with, for every position, the index of the cell covering it.
struct Board {
    rows: isize,
    cols: isize,
    own: Vec<isize>,
    cells: Vec<Cell>,
}

impl Board {
    fn new(rows: usize, cols: usize, anchors: &[Anchor]) -> Self {
        let cells = anchors
            .iter()
            .map(|a| Cell {
                r: a.r as isize,
                c: a.c as isize,
                rs: a.rs.min(rows - a.r) as isize,
                cs: a.cs.min(cols.saturating_sub(a.c)) as isize,
                sides: a.sides,
                empty: !a.content,
                fraction_term: a.fraction_term,
                fraction: false,
                dead: false,
            })
            .collect();
        let mut board = Board {
            rows: rows as isize,
            cols: cols as isize,
            own: vec![-1; rows * cols],
            cells,
        };
        for i in 0..board.cells.len() {
            board.place(i);
        }
        board
    }

    fn place(&mut self, i: usize) {
        let v = self.cells[i];
        for r in v.r..v.r + v.rs {
            for c in v.c..v.c + v.cs {
                self.own[(r * self.cols + c) as usize] = i as isize;
            }
        }
    }

    fn at(&self, r: isize, c: isize) -> isize {
        if r < 0 || c < 0 || r >= self.rows || c >= self.cols {
            -1
        } else {
            self.own[(r * self.cols + c) as usize]
        }
    }

    fn sides(&self, i: isize) -> Sides {
        if i < 0 {
            Sides::default()
        } else {
            self.cells[i as usize].sides
        }
    }

    /// Vertical unit edge at column boundary `x`, row `r`.
    fn v_edge(&self, x: isize, r: isize) -> bool {
        let (l, rt) = (self.at(r, x - 1), self.at(r, x));
        l != rt && (self.sides(l).right || self.sides(rt).left)
    }

    /// Horizontal unit edge at row boundary `y`, column `c`.
    fn h_edge(&self, y: isize, c: isize) -> bool {
        let (u, d) = (self.at(y - 1, c), self.at(y, c));
        u != d && (self.sides(u).bottom || self.sides(d).top)
    }

    /// Rule 2.  All pairs are judged on the original edges, then merged.
    fn merge_fractions(&mut self) {
        let n = self.cells.len();
        let mut pairs = Vec::new();
        for i in 0..n {
            let u = self.cells[i];
            if u.rs != 1 || !u.fraction_term {
                continue;
            }
            let j = self.at(u.r + 1, u.c);
            if j < 0 {
                continue;
            }
            let d = self.cells[j as usize];
            if d.r != u.r + 1 || d.c != u.c || d.cs != u.cs || d.rs != 1 || !d.fraction_term {
                continue;
            }
            let (y, c0, c1) = (u.r + 1, u.c, u.c + u.cs);
            let ok = !self.h_edge(y, c0 - 1)
                && !self.h_edge(y, c1)
                && (c0..c1)
                    .all(|c| self.h_edge(y, c) && !self.h_edge(u.r, c) && !self.h_edge(y + 1, c))
                && !self.v_edge(c0, u.r)
                && !self.v_edge(c1, u.r)
                && !self.v_edge(c0, y)
                && !self.v_edge(c1, y)
                && !(c0 == 0 && c1 == self.cols && n > 2);
            if ok {
                pairs.push((i, j as usize));
            }
        }
        for (i, j) in pairs {
            let v = &mut self.cells[i];
            v.rs = 2;
            v.sides = Sides::default();
            v.fraction = true;
            self.cells[j].dead = true;
            self.place(i);
        }
    }

    /// Rule 3.
    fn is_table_row(&self, r: isize) -> bool {
        if (0..=self.cols).any(|x| self.v_edge(x, r)) {
            return true;
        }
        let covering: HashSet<isize> = (0..self.cols)
            .map(|c| self.at(r, c))
            .filter(|&i| i >= 0)
            .collect();
        covering.len() >= 2 && (0..self.cols).all(|c| self.h_edge(r, c) && self.h_edge(r + 1, c))
    }

    fn row_bands(&self) -> Vec<Segment> {
        let mut segments: Vec<Segment> = Vec::new();
        for r in 0..self.rows {
            let band = self.is_table_row(r);
            match segments.last_mut() {
                Some(s) if s.band == band => s.r1 = r,
                _ => segments.push(Segment { band, r0: r, r1: r }),
            }
        }
        segments
    }

    /// Rule 4: the visible table of the row band `r0..=r1`.
    fn visible_table(&self, r0: isize, r1: isize) -> Option<(u32, u32)> {
        let members: Vec<Cell> = self
            .cells
            .iter()
            .filter(|v| !v.dead && (r0..=r1).contains(&v.r))
            .map(|v| Cell {
                rs: v.rs.min(r1 + 1 - v.r),
                ..*v
            })
            .collect();
        let (mut xmin, mut xmax) = (isize::MAX, isize::MIN);
        for r in r0..=r1 {
            for x in 0..=self.cols {
                if self.v_edge(x, r) {
                    xmin = xmin.min(x);
                    xmax = xmax.max(x);
                }
            }
        }
        for y in r0..=r1 + 1 {
            for c in 0..self.cols {
                if self.h_edge(y, c) {
                    xmin = xmin.min(c);
                    xmax = xmax.max(c + 1);
                }
            }
        }
        let filled = |v: &Cell| v.fraction || !v.empty;
        let mut keep: Vec<Cell> = members
            .into_iter()
            .filter(|v| filled(v) || (v.c + v.cs > xmin && v.c < xmax))
            .collect();
        if keep.is_empty() {
            return None;
        }

        // Empty spacer rows with no line above or below are not rows.
        let xs = keep.iter().map(|v| v.c).min()?;
        let xe = keep.iter().map(|v| v.c + v.cs).max()?;
        let line_at = |y: isize| (xs..xe).any(|c| self.h_edge(y, c));
        let drop: Vec<isize> = (r0..=r1)
            .filter(|&r| {
                let starts: Vec<&Cell> = keep.iter().filter(|v| v.r == r).collect();
                !starts.is_empty()
                    && !starts.iter().any(|v| filled(v))
                    && !(line_at(r) && line_at(r + 1))
            })
            .collect();
        if !drop.is_empty() {
            let shift = |y: isize| y - drop.iter().filter(|&&d| d < y).count() as isize;
            keep = keep
                .into_iter()
                .filter(|v| !drop.contains(&v.r))
                .map(|v| Cell {
                    r: shift(v.r),
                    rs: shift(v.r + v.rs) - shift(v.r),
                    ..v
                })
                .filter(|v| v.rs > 0)
                .collect();
        }
        if keep.is_empty() {
            return None;
        }

        // Empty trailing columns of this visible table.
        let mut right = keep.iter().map(|v| v.c + v.cs).max()?;
        while right > 1 {
            let starts: Vec<&Cell> = keep.iter().filter(|v| v.c == right - 1).collect();
            if starts.is_empty() || starts.iter().any(|v| filled(v)) {
                break;
            }
            let remaining: Vec<Cell> = keep.iter().filter(|v| v.c != right - 1).copied().collect();
            if remaining.is_empty() {
                break;
            }
            keep = remaining
                .into_iter()
                .map(|v| {
                    if v.c + v.cs == right {
                        Cell { cs: v.cs - 1, ..v }
                    } else {
                        v
                    }
                })
                .collect();
            right -= 1;
        }

        // Grid lines no cell corner uses collapse.
        let xs: BTreeSet<isize> = keep.iter().flat_map(|v| [v.c, v.c + v.cs]).collect();
        let ys: BTreeSet<isize> = keep.iter().flat_map(|v| [v.r, v.r + v.rs]).collect();
        Some(((ys.len() - 1) as u32, (xs.len() - 1) as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Sides = Sides {
        left: true,
        right: true,
        top: true,
        bottom: true,
    };
    const NONE: Sides = Sides {
        left: false,
        right: false,
        top: false,
        bottom: false,
    };

    /// Cells of a plain grid, row by row; `spec[r][c]` is (sides, content).
    fn grid(spec: &[&[(Sides, bool)]]) -> Vec<RawCell> {
        let mut cells = Vec::new();
        for (r, row) in spec.iter().enumerate() {
            for (c, &(sides, content)) in row.iter().enumerate() {
                cells.push(RawCell {
                    tr: r,
                    row_addr: Some(r),
                    col_addr: Some(c),
                    row_span: 1,
                    col_span: 1,
                    sides,
                    content,
                    fraction_term: false,
                });
            }
        }
        cells
    }

    #[test]
    fn bordered_grid_is_one_table() {
        let x = (ALL, true);
        let l = layout(&grid(&[&[x, x, x], &[x, x, x]]), 2);
        assert_eq!(l.tables, vec![(2, 3)]);
        assert!(l.band.iter().all(|&b| b == Some(true)));
    }

    #[test]
    fn borderless_frame_is_text() {
        let x = (NONE, true);
        let l = layout(&grid(&[&[x, x], &[x, x]]), 2);
        assert_eq!(l.tables, vec![]);
        assert!(l.band.iter().all(|&b| b == Some(false)));
    }

    #[test]
    fn borderless_title_row_above_a_table() {
        // Row 0: one title cell over both columns, no lines; rows 1–2 drawn.
        let mut cells = grid(&[
            &[(NONE, true)],
            &[(ALL, true), (ALL, true)],
            &[(ALL, true), (ALL, true)],
        ]);
        cells[0].col_span = 2;
        let l = layout(&cells, 3);
        assert_eq!(l.tables, vec![(2, 2)]);
        assert_eq!(l.band[0], Some(false));
        assert!(l.band[1..].iter().all(|&b| b == Some(true)));
    }

    #[test]
    fn stacked_short_cells_with_only_a_rule_between_are_a_fraction() {
        let mut cells = grid(&[&[(NONE, true)], &[(NONE, true)]]);
        cells[0].sides.bottom = true;
        for c in &mut cells {
            c.fraction_term = true;
        }
        let l = layout(&cells, 2);
        assert_eq!(l.tables, vec![], "a fraction is an equation, not a table");
        assert!(l.band.iter().all(|&b| b == Some(false)));
    }

    #[test]
    fn empty_spacer_row_without_lines_is_dropped() {
        // Side borders only on the spacer row: it is inside the band but has
        // no line above or below, so it does not count as a row.
        let x = (ALL, true);
        let side = Sides {
            left: true,
            right: true,
            ..NONE
        };
        let mut cells = grid(&[&[x, x], &[(side, false), (side, false)], &[x, x]]);
        // Rows 0 and 2 have no line facing the spacer row.
        cells[0].sides.bottom = false;
        cells[1].sides.bottom = false;
        cells[4].sides.top = false;
        cells[5].sides.top = false;
        let l = layout(&cells, 3);
        assert_eq!(l.tables, vec![(2, 2)]);
    }

    #[test]
    fn empty_trailing_columns_are_trimmed() {
        let x = (ALL, true);
        let e = (ALL, false);
        let l = layout(&grid(&[&[x, x, e], &[x, x, e]]), 2);
        assert_eq!(l.tables, vec![(2, 2)]);
        assert_eq!(l.band[2], None);
    }

    #[test]
    fn cells_without_addresses_flow_around_row_spans() {
        // | A (2 rows) | B     |
        // |            | C | D |  → C and D go right of A: 2×3
        let cell = |tr, row_span| RawCell {
            tr,
            row_span,
            col_span: 1,
            sides: ALL,
            content: true,
            ..Default::default()
        };
        let cells = vec![cell(0, 2), cell(0, 1), cell(1, 1), cell(1, 1)];
        assert_eq!(layout(&cells, 2).tables, vec![(2, 3)]);
    }
}
