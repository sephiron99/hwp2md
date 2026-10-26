//! Ground truth taken straight from the HWPX XML.
//!
//! This deliberately shares no code with either converter: it opens the ZIP,
//! reads `Contents/sectionN.xml` in order and collects the text of every
//! `<hp:t>` element, grouped by the innermost `<hp:p>` paragraph that holds
//! it.  Each paragraph is tagged with where it lives (body, table cell, text
//! box, footnote, header/footer, hidden comment) so scores can be split.
//! Guide text of an unfilled click-here field (누름틀 안내문: `CLICK_HERE`
//! with `dirty≠1` whose text is still its `Direction` guide) is split off
//! into its own unit — Hancom does not print it.  Other text in such a
//! field (a preset value such as `0000.00.00.`) is ordinary text.
//!
//! Tables are the ones Hancom draws on screen (see [`crate::visible`]): each
//! `<hp:tbl>` is split by its visible cell borders (`borderFill` in
//! `Contents/header.xml`) into zero or more visible tables and plain text
//! rows.  Text in those rows counts as body text, not table cells.

use crate::visible::{self, RawCell, Sides};
use anyhow::{bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Where a paragraph lives in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Body,
    Cell,
    TextBox,
    Note,
    HeaderFooter,
    Hidden,
    Placeholder,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Body,
        Kind::Cell,
        Kind::TextBox,
        Kind::Note,
        Kind::HeaderFooter,
        Kind::Hidden,
        Kind::Placeholder,
    ];

    /// Visible main content: what a converter must not lose.
    pub fn is_main(self) -> bool {
        matches!(self, Kind::Body | Kind::Cell | Kind::TextBox)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Body => "본문",
            Kind::Cell => "표 칸",
            Kind::TextBox => "글상자",
            Kind::Note => "각주·미주",
            Kind::HeaderFooter => "머리말·꼬리말",
            Kind::Hidden => "숨은 설명",
            Kind::Placeholder => "누름틀 안내문",
        }
    }
}

/// One paragraph of reference text.
#[derive(Debug, Clone)]
pub struct Unit {
    pub text: String,
    pub kind: Kind,
}

/// Rows × columns of a visible table (reference) or of a Markdown table
/// after colspan/rowspan layout (output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableShape {
    pub rows: u32,
    pub cols: u32,
}

#[derive(Debug, Default)]
pub struct Reference {
    /// Paragraphs in document order (order of their opening tag).
    pub units: Vec<Unit>,
    /// Visible tables outside headers/footers and hidden comments; a table
    /// nested in a cell comes before the table holding it.
    pub tables: Vec<TableShape>,
}

pub fn extract(path: &Path) -> Result<Reference> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("not a ZIP archive")?;

    if let Ok(mut manifest) = zip.by_name("META-INF/manifest.xml") {
        let mut text = String::new();
        manifest.read_to_string(&mut text).ok();
        if text.contains("encryption-data") {
            bail!("encrypted HWPX (배포용 암호화)");
        }
    }

    let mut sections: Vec<(u32, String)> = zip
        .file_names()
        .filter_map(|name| {
            let num = name
                .strip_prefix("Contents/section")?
                .strip_suffix(".xml")?;
            num.parse().ok().map(|n| (n, name.to_string()))
        })
        .collect();
    sections.sort();
    if sections.is_empty() {
        bail!("no Contents/sectionN.xml in archive");
    }

    let borders = match zip.by_name("Contents/header.xml") {
        Ok(mut f) => {
            let mut xml = Vec::new();
            f.read_to_end(&mut xml)?;
            border_fills(&xml).context("Contents/header.xml")?
        }
        Err(_) => HashMap::new(),
    };

    let mut out = Reference::default();
    for (_, name) in sections {
        let mut xml = Vec::new();
        zip.by_name(&name)?.read_to_end(&mut xml)?;
        parse_section(&xml, &borders, &mut out).with_context(|| name.clone())?;
    }
    out.units.retain(|u| !u.text.trim().is_empty());
    Ok(out)
}

/// Visible sides of every `<hh:borderFill>`, by id.  A side is visible when
/// its `type` is not `NONE` and its colour is not white.
fn border_fills(xml: &[u8]) -> Result<HashMap<String, Sides>> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut fills = HashMap::new();
    let mut current: Option<String> = None;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.local_name().as_ref() == b"borderFill" => {
                current = attr(&e, b"id");
                if let Some(id) = &current {
                    fills.insert(id.clone(), Sides::default());
                }
            }
            Event::End(e) if e.local_name().as_ref() == b"borderFill" => current = None,
            Event::Empty(e) | Event::Start(e) => {
                if let Some(sides) = current.as_ref().and_then(|id| fills.get_mut(id)) {
                    let side = match e.local_name().as_ref() {
                        b"leftBorder" => Some(&mut sides.left),
                        b"rightBorder" => Some(&mut sides.right),
                        b"topBorder" => Some(&mut sides.top),
                        b"bottomBorder" => Some(&mut sides.bottom),
                        _ => None,
                    };
                    if let Some(side) = side {
                        let ty = attr(&e, b"type").unwrap_or_default().to_ascii_uppercase();
                        let color = attr(&e, b"color").unwrap_or_default().to_ascii_uppercase();
                        *side = !ty.is_empty() && ty != "NONE" && color != "#FFFFFF";
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(fills)
}

fn attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name)
        .map(|a| a.unescape_value().unwrap_or_default().into_owned())
}

fn num_attr(e: &BytesStart, name: &[u8]) -> Option<usize> {
    attr(e, name)?.trim().parse().ok()
}

/// Is this `<hp:fieldBegin>` an unfilled click-here field?
fn is_click_here(e: &BytesStart) -> bool {
    let mut click_here = false;
    let mut dirty = false;
    for attr in e.attributes().flatten() {
        match attr.key.local_name().as_ref() {
            b"type" => click_here = attr.value.eq_ignore_ascii_case(b"CLICK_HERE"),
            b"dirty" => dirty = attr.value.as_ref() == b"1",
            _ => {}
        }
    }
    click_here && !dirty
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
            return (!guide.is_empty()).then_some(guide);
        }
    }
    None
}

/// A `<hp:fieldBegin>` whose parameters are being read.
struct FieldBegin {
    click_here: bool,
    /// The first `Direction` parameter (empty means no guide).
    direction: Option<String>,
    command: Option<String>,
    /// Name and text of the `<hp:stringParam>` being read.
    param: Option<(Vec<u8>, String)>,
}

/// An open field.  `held` is set for an unfilled click-here field with a
/// guide: its text waits for `fieldEnd`, which drops it as guide text when
/// it is exactly the guide.
struct Field {
    held: Option<Held>,
}

struct Held {
    guide: String,
    text: String,
    line_break: bool,
    /// Open paragraphs at `fieldBegin`: a field still open when its
    /// paragraph ends is not guide text.
    depth: usize,
}

/// Elements (lower-cased) that make a cell more than plain text, so it
/// cannot be a fraction term.
const OBJECTS: &[&[u8]] = &[
    b"tbl",
    b"pic",
    b"shape",
    b"drawingobject",
    b"rect",
    b"ellipse",
    b"polygon",
    b"line",
    b"arc",
    b"curve",
    b"connectline",
    b"container",
    b"textart",
    b"ole",
    b"unknownobject",
    b"video",
    b"chart",
    b"drawtext",
    b"equation",
    b"checkbtn",
    b"radiobtn",
    b"footnote",
    b"endnote",
];

/// Objects that draw something even without text: a cell holding one is
/// not empty.  (Images count through `<hc:img binaryItemIDRef>`.)
const VISIBLE_OBJECTS: &[&[u8]] = &[
    b"equation",
    b"checkbtn",
    b"radiobtn",
    b"footnote",
    b"endnote",
];

/// Nesting counters for the elements that decide a paragraph's [`Kind`].
#[derive(Default)]
struct Ctx {
    cell: usize,
    text_box: usize,
    note: usize,
    header_footer: usize,
    hidden: usize,
}

impl Ctx {
    fn counter(&mut self, local: &[u8]) -> Option<&mut usize> {
        match local {
            b"drawText" => Some(&mut self.text_box),
            b"footNote" | b"endNote" => Some(&mut self.note),
            b"header" | b"footer" | b"masterPage" => Some(&mut self.header_footer),
            b"hiddenComment" => Some(&mut self.hidden),
            _ => None,
        }
    }

    fn kind(&self) -> Kind {
        if self.hidden > 0 {
            Kind::Hidden
        } else if self.header_footer > 0 {
            Kind::HeaderFooter
        } else if self.note > 0 {
            Kind::Note
        } else if self.text_box > 0 {
            Kind::TextBox
        } else if self.cell > 0 {
            Kind::Cell
        } else {
            Kind::Body
        }
    }

    fn tables_visible(&self) -> bool {
        self.hidden == 0 && self.header_footer == 0
    }
}

/// An `<hp:tbl>` being read.
#[derive(Default)]
struct OpenTable {
    /// Outside headers/footers and hidden comments.
    counted: bool,
    /// `<hp:tr>` elements seen so far.
    rows: usize,
    cells: Vec<RawCell>,
    /// Per cell: its [`Kind::Cell`] units.
    units: Vec<Vec<usize>>,
    /// Per cell: visible tables nested anywhere inside it.
    nested: Vec<Vec<TableShape>>,
}

/// An `<hp:tc>` being read.
#[derive(Default)]
struct OpenCell {
    raw: RawCell,
    /// Open paragraphs when the cell started: a paragraph opened at this
    /// depth is one of the cell's own.
    depth: usize,
    paragraphs: usize,
    object: bool,
    /// Text of the cell's own paragraphs, for the fraction test.
    text: String,
    line_break: bool,
    /// [`Kind::Cell`] units of this cell, plus those of the text rows of
    /// tables nested in it (they read as this cell's text).
    units: Vec<usize>,
    nested: Vec<TableShape>,
}

struct Parser<'a> {
    borders: &'a HashMap<String, Sides>,
    out: &'a mut Reference,
    ctx: Ctx,
    /// Indices (into `out.units`) of the currently open paragraphs; text
    /// goes to the innermost one.
    open: Vec<usize>,
    in_t: usize,
    begin: Option<FieldBegin>,
    fields: Vec<Field>,
    tables: Vec<OpenTable>,
    cells: Vec<OpenCell>,
}

pub(crate) fn parse_section(
    xml: &[u8],
    borders: &HashMap<String, Sides>,
    out: &mut Reference,
) -> Result<()> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut p = Parser {
        borders,
        out,
        ctx: Ctx::default(),
        open: Vec::new(),
        in_t: 0,
        begin: None,
        fields: Vec::new(),
        tables: Vec::new(),
        cells: Vec::new(),
    };
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => p.start(&e),
            Event::Empty(e) => {
                p.start(&e);
                p.end(e.local_name().as_ref());
            }
            Event::End(e) => p.end(e.local_name().as_ref()),
            Event::Text(e) if p.wants_text() => p.raw_text(&e.unescape()?),
            Event::CData(e) if p.wants_text() => {
                p.raw_text(&String::from_utf8_lossy(&e.into_inner()))
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

impl Parser<'_> {
    fn start(&mut self, e: &BytesStart) {
        let name = e.local_name();
        let name = name.as_ref();
        if let Some(cell) = self.cells.last_mut() {
            let lower = name.to_ascii_lowercase();
            if OBJECTS.contains(&lower.as_slice()) {
                cell.object = true;
            }
            if VISIBLE_OBJECTS.contains(&lower.as_slice())
                || (name == b"img" && attr(e, b"binaryItemIDRef").is_some())
            {
                cell.raw.content = true;
            }
        }
        match name {
            b"p" => {
                let kind = self.ctx.kind();
                self.out.units.push(Unit {
                    text: String::new(),
                    kind,
                });
                let idx = self.out.units.len() - 1;
                if let Some(cell) = self.cells.last_mut() {
                    if self.open.len() == cell.depth {
                        cell.paragraphs += 1;
                    }
                    if kind == Kind::Cell {
                        cell.units.push(idx);
                    }
                }
                self.open.push(idx);
            }
            b"t" => self.in_t += 1,
            b"tbl" => self.tables.push(OpenTable {
                counted: self.ctx.tables_visible(),
                ..OpenTable::default()
            }),
            b"tr" => {
                if let Some(t) = self.tables.last_mut() {
                    t.rows += 1;
                }
            }
            b"tc" => {
                self.ctx.cell += 1;
                let sides = attr(e, b"borderFillIDRef")
                    .and_then(|id| self.borders.get(&id).copied())
                    .unwrap_or_default();
                let tr = self.tables.last().map_or(0, |t| t.rows.saturating_sub(1));
                self.cells.push(OpenCell {
                    raw: RawCell {
                        tr,
                        row_span: 1,
                        col_span: 1,
                        sides,
                        ..RawCell::default()
                    },
                    depth: self.open.len(),
                    ..OpenCell::default()
                });
            }
            b"cellAddr" => {
                if let Some(cell) = self.cells.last_mut() {
                    cell.raw.col_addr = num_attr(e, b"colAddr");
                    cell.raw.row_addr = num_attr(e, b"rowAddr");
                }
            }
            b"cellSpan" => {
                if let Some(cell) = self.cells.last_mut() {
                    cell.raw.col_span = num_attr(e, b"colSpan").unwrap_or(1).max(1);
                    cell.raw.row_span = num_attr(e, b"rowSpan").unwrap_or(1).max(1);
                }
            }
            b"fieldBegin" => {
                self.begin = Some(FieldBegin {
                    click_here: is_click_here(e),
                    direction: None,
                    command: None,
                    param: None,
                })
            }
            b"stringParam" => {
                if let Some(b) = &mut self.begin {
                    b.param = attr(e, b"name").map(|n| (n.into_bytes(), String::new()));
                }
            }
            b"fieldEnd" => self.field_end(),
            // Spacing elements inside <hp:t> stand for visible whitespace.
            b"tab" | b"fwSpace" | b"nbSpace" | b"lineBreak" if self.in_t > 0 => {
                self.text(" ", name == b"lineBreak")
            }
            other => {
                if let Some(c) = self.ctx.counter(other) {
                    *c += 1;
                }
            }
        }
    }

    fn end(&mut self, name: &[u8]) {
        match name {
            b"p" => {
                self.release_fields();
                self.open.pop();
            }
            b"stringParam" => {
                if let Some(b) = &mut self.begin {
                    match b.param.take() {
                        Some((n, text)) if n == b"Direction" && b.direction.is_none() => {
                            b.direction = Some(text)
                        }
                        Some((n, text)) if n == b"Command" => {
                            if let Some(g) = command_guide(&text) {
                                b.command = Some(g);
                            }
                        }
                        _ => {}
                    }
                }
            }
            b"fieldBegin" => self.field_begin(),
            b"t" => self.in_t = self.in_t.saturating_sub(1),
            b"tc" => {
                self.ctx.cell = self.ctx.cell.saturating_sub(1);
                self.close_cell();
            }
            b"tbl" => self.close_table(),
            other => {
                if let Some(c) = self.ctx.counter(other) {
                    *c = c.saturating_sub(1);
                }
            }
        }
    }

    fn wants_text(&self) -> bool {
        self.in_t > 0 || self.begin.as_ref().is_some_and(|b| b.param.is_some())
    }

    fn raw_text(&mut self, s: &str) {
        if let Some((_, text)) = self.begin.as_mut().and_then(|b| b.param.as_mut()) {
            text.push_str(s);
        } else if self.in_t > 0 {
            self.text(s, false);
        }
    }

    fn text(&mut self, s: &str, line_break: bool) {
        if let Some(held) = self.fields.iter_mut().rev().find_map(|f| f.held.as_mut()) {
            held.text.push_str(s);
            held.line_break |= line_break;
            return;
        }
        let Some(&i) = self.open.last() else { return };
        self.out.units[i].text.push_str(s);
        let shown = !matches!(self.ctx.kind(), Kind::Hidden | Kind::HeaderFooter);
        if let Some(cell) = self.cells.last_mut() {
            if shown && s.chars().any(|c| !c.is_whitespace()) {
                cell.raw.content = true;
            }
            if self.open.len() == cell.depth + 1 {
                cell.text.push_str(s);
                cell.line_break |= line_break;
            }
        }
    }

    fn close_cell(&mut self) {
        let Some(mut cell) = self.cells.pop() else {
            return;
        };
        let text = cell.text.trim();
        cell.raw.fraction_term = cell.paragraphs == 1
            && !cell.object
            && !cell.line_break
            && !text.is_empty()
            && text.chars().count() <= 40;
        // A nested table with content fills the cell holding it.
        if cell.raw.content {
            if let Some(parent) = self.cells.last_mut() {
                parent.raw.content = true;
            }
        }
        if let Some(t) = self.tables.last_mut() {
            t.cells.push(cell.raw);
            t.units.push(cell.units);
            t.nested.push(cell.nested);
        }
    }

    fn close_table(&mut self) {
        let Some(t) = self.tables.pop() else { return };
        let layout = visible::layout(&t.cells, t.rows);
        let mut shapes = Vec::new();
        let mut text_units = Vec::new();
        for (i, band) in layout.band.iter().enumerate() {
            // Tables nested in a trimmed empty column go with it.
            if band.is_some() {
                shapes.extend_from_slice(&t.nested[i]);
            }
            if *band != Some(true) {
                text_units.extend_from_slice(&t.units[i]);
            }
        }
        shapes.extend(
            layout
                .tables
                .iter()
                .map(|&(rows, cols)| TableShape { rows, cols }),
        );
        if !t.counted {
            shapes.clear();
        }
        // Text rows read as the text around the table: the enclosing cell's
        // text, or body text at the top level.
        match self.cells.last_mut() {
            Some(parent) => {
                parent.nested.extend(shapes);
                parent.units.extend(text_units);
            }
            None => {
                self.out.tables.extend(shapes);
                for i in text_units {
                    self.out.units[i].kind = Kind::Body;
                }
            }
        }
    }

    fn field_begin(&mut self) {
        let Some(b) = self.begin.take() else { return };
        let guide = match b.direction {
            Some(d) => (!d.is_empty()).then_some(d),
            None => b.command,
        };
        let held = guide.filter(|_| b.click_here).map(|guide| Held {
            guide,
            text: String::new(),
            line_break: false,
            depth: self.open.len(),
        });
        self.fields.push(Field { held });
    }

    fn field_end(&mut self) {
        let Some(Field { held: Some(held) }) = self.fields.pop() else {
            return;
        };
        let t = &held.text;
        if !t.is_empty() && (*t == held.guide || t.trim_end() == held.guide) {
            let kind = if self.ctx.kind() == Kind::Hidden {
                Kind::Hidden
            } else {
                Kind::Placeholder
            };
            self.out.units.push(Unit {
                text: held.text,
                kind,
            });
        } else {
            self.text(&held.text, held.line_break);
        }
    }

    /// The paragraph is ending: text held by fields opened in it is
    /// ordinary text.
    fn release_fields(&mut self) {
        let depth = self.open.len();
        for i in (0..self.fields.len()).rev() {
            let opened_here = self.fields[i]
                .held
                .as_ref()
                .is_some_and(|h| h.depth == depth);
            if opened_here {
                let held = self.fields[i].held.take().expect("checked above");
                self.text(&held.text, held.line_break);
            }
        }
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

    fn parse(xml: &str) -> Reference {
        let mut r = Reference::default();
        let borders: HashMap<String, Sides> =
            [("7".to_string(), ALL), ("1".to_string(), Sides::default())].into();
        parse_section(xml.as_bytes(), &borders, &mut r).unwrap();
        r.units.retain(|u| !u.text.trim().is_empty());
        r
    }

    fn cell(fill: &str, row: usize, col: usize, text: &str) -> String {
        format!(
            r#"<hp:tc borderFillIDRef="{fill}"><hp:subList><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p></hp:subList><hp:cellAddr colAddr="{col}" rowAddr="{row}"/><hp:cellSpan colSpan="1" rowSpan="1"/></hp:tc>"#
        )
    }

    fn units(r: &Reference) -> Vec<(&str, Kind)> {
        r.units.iter().map(|u| (u.text.trim(), u.kind)).collect()
    }

    #[test]
    fn paragraphs_cells_notes_and_spacing() {
        let r = parse(&format!(
            r#"<hs:sec xmlns:hs="s" xmlns:hp="p">
              <hp:p><hp:run><hp:t>머리<hp:fwSpace/>글</hp:t></hp:run>
                <hp:run><hp:ctrl><hp:footNote><hp:subList><hp:p><hp:run><hp:t>각주</hp:t></hp:run></hp:p></hp:subList></hp:footNote></hp:ctrl></hp:run>
                <hp:run><hp:t> 뒤</hp:t></hp:run></hp:p>
              <hp:p><hp:run><hp:tbl rowCnt="1" colCnt="2"><hp:tr>{}{}</hp:tr></hp:tbl></hp:run></hp:p>
              <hp:p><hp:run><hp:ctrl><hp:header><hp:subList><hp:p><hp:run><hp:t>머리말</hp:t></hp:run></hp:p></hp:subList></hp:header></hp:ctrl></hp:run></hp:p>
            </hs:sec>"#,
            cell("7", 0, 0, "칸 &amp; 글"),
            cell("7", 0, 1, "둘"),
        ));
        assert_eq!(
            units(&r),
            vec![
                ("머리 글 뒤", Kind::Body),
                ("각주", Kind::Note),
                ("칸 & 글", Kind::Cell),
                ("둘", Kind::Cell),
                ("머리말", Kind::HeaderFooter),
            ]
        );
        assert_eq!(r.tables, vec![TableShape { rows: 1, cols: 2 }]);
    }

    #[test]
    fn borderless_frame_is_body_text_and_its_inner_table_counts() {
        // A borderless 1×2 frame; its right cell holds a drawn 2×2 table.
        let inner = format!(
            r#"<hp:tbl rowCnt="2" colCnt="2"><hp:tr>{}{}</hp:tr><hp:tr>{}{}</hp:tr></hp:tbl>"#,
            cell("7", 0, 0, "가"),
            cell("7", 0, 1, "나"),
            cell("7", 1, 0, "다"),
            cell("7", 1, 1, "라"),
        );
        let r = parse(&format!(
            r#"<hs:sec xmlns:hs="s" xmlns:hp="p"><hp:p><hp:run><hp:tbl rowCnt="1" colCnt="2"><hp:tr>
              {}
              <hp:tc borderFillIDRef="1"><hp:subList>
                <hp:p><hp:run><hp:t>틀 오른쪽</hp:t></hp:run></hp:p>
                <hp:p><hp:run>{inner}</hp:run></hp:p>
              </hp:subList><hp:cellAddr colAddr="1" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/></hp:tc>
            </hp:tr></hp:tbl></hp:run></hp:p></hs:sec>"#,
            cell("1", 0, 0, "틀 왼쪽"),
        ));
        assert_eq!(
            units(&r),
            vec![
                ("틀 왼쪽", Kind::Body),
                ("틀 오른쪽", Kind::Body),
                ("가", Kind::Cell),
                ("나", Kind::Cell),
                ("다", Kind::Cell),
                ("라", Kind::Cell),
            ]
        );
        assert_eq!(r.tables, vec![TableShape { rows: 2, cols: 2 }]);
    }

    #[test]
    fn click_here_text_is_a_placeholder_only_while_it_is_the_guide() {
        let r = parse(
            r#"<hs:sec xmlns:hs="s" xmlns:hp="p"><hp:p>
              <hp:run><hp:t>마약류(예 : </hp:t></hp:run>
              <hp:run><hp:ctrl><hp:fieldBegin type="CLICK_HERE" dirty="0"><hp:parameters>
                <hp:stringParam name="Direction">마약류 기재</hp:stringParam></hp:parameters></hp:fieldBegin></hp:ctrl><hp:t>마약류 기재</hp:t></hp:run>
              <hp:run><hp:ctrl><hp:fieldEnd/></hp:ctrl><hp:t>) 투약 </hp:t></hp:run>
              <hp:run><hp:ctrl><hp:fieldBegin type="CLICK_HERE" dirty="0"><hp:parameters>
                <hp:stringParam name="Command">Clickhere:set:20:Direction:wstring:2:일자 HelpState:wstring:0: </hp:stringParam></hp:parameters></hp:fieldBegin></hp:ctrl><hp:t>0000.00.00.</hp:t></hp:run>
              <hp:run><hp:ctrl><hp:fieldEnd/></hp:ctrl></hp:run>
              <hp:run><hp:ctrl><hp:fieldBegin type="CLICK_HERE" dirty="0"/></hp:ctrl><hp:t> 대상자</hp:t></hp:run>
              <hp:run><hp:ctrl><hp:fieldEnd/></hp:ctrl></hp:run>
            </hp:p></hs:sec>"#,
        );
        assert_eq!(
            units(&r),
            vec![
                ("마약류(예 : ) 투약 0000.00.00. 대상자", Kind::Body),
                ("마약류 기재", Kind::Placeholder)
            ]
        );
        assert_eq!(
            command_guide("a Direction:wstring:5:성명 입력 b"),
            Some("성명 입력".into())
        );
        assert_eq!(command_guide("Direction:wstring:0:"), None);
    }

    #[test]
    fn border_sides_from_header() {
        let fills = border_fills(
            br##"<hh:head xmlns:hh="h"><hh:borderFills>
              <hh:borderFill id="3"><hh:leftBorder type="SOLID" color="#000000"/><hh:rightBorder type="NONE" color="#000000"/>
                <hh:topBorder type="SOLID" color="#FFFFFF"/><hh:bottomBorder type="DOT" color="#ff0000"/></hh:borderFill>
            </hh:borderFills></hh:head>"##,
        )
        .unwrap();
        assert_eq!(
            fills["3"],
            Sides {
                left: true,
                right: false,
                top: false,
                bottom: true
            }
        );
    }
}
