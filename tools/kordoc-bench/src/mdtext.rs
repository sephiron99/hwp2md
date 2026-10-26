//! Markdown → plain text and table shapes, applied identically to both
//! converters' output.

use crate::reference::TableShape;

#[derive(Debug, Default)]
pub struct MdDoc {
    /// Visible text with Markdown/HTML markup removed.
    pub plain: String,
    /// Pipe tables and HTML tables (nested HTML tables included).
    pub tables: Vec<TableShape>,
    /// ATX heading lines (`#`, `##`, …).
    pub headings: usize,
}

pub fn analyze(md: &str) -> MdDoc {
    let md = strip_html_comments(md);
    let mut doc = MdDoc {
        tables: html_tables(&md),
        ..MdDoc::default()
    };

    let lines: Vec<&str> = md.lines().collect();
    let mut plain = String::with_capacity(md.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_start();

        // Pipe table: header row followed by a separator row.
        if line.starts_with('|') && lines.get(i + 1).is_some_and(|l| is_separator(l)) {
            let cols = pipe_cells(line);
            let mut rows = 1;
            plain.push_str(&inline_text(line));
            plain.push('\n');
            i += 2;
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                plain.push_str(&inline_text(lines[i]));
                plain.push('\n');
                rows += 1;
                i += 1;
            }
            doc.tables.push(TableShape { rows, cols });
            continue;
        }

        if is_rule(line) {
            i += 1;
            continue;
        }
        let mut rest = line;
        let hashes = rest.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && rest[hashes..].starts_with([' ', '\t']) {
            doc.headings += 1;
            rest = &rest[hashes..];
        }
        rest = strip_block_prefixes(rest);
        plain.push_str(&inline_text(rest));
        plain.push('\n');
        i += 1;
    }
    doc.plain = plain;
    doc
}

/// Characters that Markdown writers use as markup.  They are dropped from
/// both the reference and the output before tokens are compared.
pub fn strip_markup_chars(s: &str) -> String {
    s.replace("~~", "")
        .chars()
        .filter_map(|c| match c {
            '*' | '`' | '$' | '\\' => None,
            '|' => Some(' '),
            c => Some(c),
        })
        .collect()
}

fn strip_html_comments(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn is_rule(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

/// Number of cells in a pipe-table row (unescaped `|` separators).
fn pipe_cells(line: &str) -> u32 {
    let t = line.trim();
    let mut bars = 0u32;
    let mut prev = '\0';
    for c in t.chars() {
        if c == '|' && prev != '\\' {
            bars += 1;
        }
        prev = c;
    }
    let edges = u32::from(t.starts_with('|')) + u32::from(t.ends_with('|') && t.len() > 1);
    (bars + 1).saturating_sub(edges)
}

/// Drop blockquote, bullet, task and footnote-definition markers.
fn strip_block_prefixes(s: &str) -> &str {
    let mut t = s.trim_start();
    loop {
        if let Some(r) = t.strip_prefix('>') {
            t = r.trim_start();
            continue;
        }
        if let Some(r) = ["- ", "* ", "+ "].iter().find_map(|m| t.strip_prefix(m)) {
            t = r.trim_start();
            continue;
        }
        // Ordered-list numbers stay: they are visible text once rendered,
        // and documents often type "1." themselves.
        break;
    }
    if let Some(r) = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .find_map(|m| t.strip_prefix(m))
    {
        return r;
    }
    if t.starts_with("[^") {
        if let Some(end) = t.find("]:") {
            return t[end + 2..].trim_start();
        }
    }
    t
}

const BLOCK_TAGS: &[&str] = &[
    "br", "td", "th", "tr", "table", "thead", "tbody", "p", "div", "li", "ul", "ol",
];

/// Visible text of one line: links keep their text, images and footnote
/// references vanish, HTML tags go (block tags become a space), entities and
/// backslash escapes are decoded.
fn inline_text(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if chars.get(i + 1).is_some_and(char::is_ascii_punctuation) => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '!' if chars.get(i + 1) == Some(&'[') => match link_end(&chars, i + 1) {
                Some((_, end)) => i = end,
                None => {
                    out.push(c);
                    i += 1;
                }
            },
            '[' if chars.get(i + 1) == Some(&'^') => {
                match chars[i..].iter().position(|&c| c == ']') {
                    Some(p) => i += p + 1,
                    None => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            '[' => match link_end(&chars, i) {
                Some((text_end, end)) => {
                    out.push_str(&inline_text(
                        &chars[i + 1..text_end].iter().collect::<String>(),
                    ));
                    i = end;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            },
            '<' => match tag_at(&chars, i) {
                Some((name, end)) => {
                    if BLOCK_TAGS.contains(&name.as_str()) {
                        out.push(' ');
                    }
                    i = end;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            },
            '&' => match entity_at(&chars, i) {
                Some((decoded, end)) => {
                    out.push(decoded);
                    i = end;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            },
            '|' => {
                out.push(' ');
                i += 1;
            }
            '*' | '`' | '$' => i += 1,
            '~' if chars.get(i + 1) == Some(&'~') => i += 2,
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// For `[text](url)` starting at `open` (the `[`), return the index of the
/// closing `]` and the index just past `)`.
fn link_end(chars: &[char], open: usize) -> Option<(usize, usize)> {
    let mut depth = 0;
    let mut j = open;
    while j < chars.len() {
        match chars[j] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= chars.len() || chars.get(j + 1) != Some(&'(') {
        return None;
    }
    let close = chars[j + 2..].iter().position(|&c| c == ')')?;
    Some((j, j + 2 + close + 1))
}

/// An HTML tag `<name …>` / `</name>` starting at `i`: lowercase name and the
/// index just past `>`.  Only ASCII tag names count, so `<보기>` stays text.
fn tag_at(chars: &[char], i: usize) -> Option<(String, usize)> {
    let mut j = i + 1;
    if chars.get(j) == Some(&'/') {
        j += 1;
    }
    let start = j;
    while j < chars.len() && chars[j].is_ascii_alphanumeric() {
        j += 1;
    }
    if j == start || !chars[start].is_ascii_alphabetic() {
        return None;
    }
    let name: String = chars[start..j]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    if !matches!(chars.get(j), Some(' ' | '>' | '/' | '\t')) {
        return None;
    }
    let close = chars[j..].iter().position(|&c| c == '>')?;
    Some((name, j + close + 1))
}

fn entity_at(chars: &[char], i: usize) -> Option<(char, usize)> {
    let semi = chars[i..].iter().take(12).position(|&c| c == ';')?;
    let body: String = chars[i + 1..i + semi].iter().collect();
    let decoded = match body.as_str() {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" | "#39" => '\'',
        "nbsp" => ' ',
        b if b.starts_with("#x") || b.starts_with("#X") => {
            char::from_u32(u32::from_str_radix(&b[2..], 16).ok()?)?
        }
        b if b.starts_with('#') => char::from_u32(b[1..].parse().ok()?)?,
        _ => return None,
    };
    Some((decoded, i + semi + 1))
}

/// Shapes of all HTML `<table>`s (nested ones included), using a grid fill
/// so `colspan`/`rowspan` count the way they are laid out.
fn html_tables(md: &str) -> Vec<TableShape> {
    struct Builder {
        rows: Vec<Vec<(u32, u32)>>,
    }
    let lower = md.to_ascii_lowercase();
    let mut stack: Vec<Builder> = Vec::new();
    let mut done = Vec::new();
    let mut pos = 0;
    while let Some(off) = lower[pos..].find('<') {
        let start = pos + off;
        let Some(len) = lower[start..].find('>') else {
            break;
        };
        let tag = &lower[start + 1..start + len];
        pos = start + len + 1;
        let (closing, body) = match tag.strip_prefix('/') {
            Some(b) => (true, b),
            None => (false, tag),
        };
        let name: String = body
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        match (name.as_str(), closing) {
            ("table", false) => stack.push(Builder { rows: Vec::new() }),
            ("table", true) => {
                if let Some(b) = stack.pop() {
                    done.push(grid_shape(&b.rows));
                }
            }
            ("tr", false) => {
                if let Some(b) = stack.last_mut() {
                    b.rows.push(Vec::new());
                }
            }
            ("td" | "th", false) => {
                if let Some(b) = stack.last_mut() {
                    if b.rows.is_empty() {
                        b.rows.push(Vec::new());
                    }
                    let span = |attr: &str| {
                        body.find(attr)
                            .map(|p| {
                                body[p + attr.len()..]
                                    .trim_start_matches(['=', '"', '\'', ' '])
                                    .chars()
                                    .take_while(char::is_ascii_digit)
                                    .collect::<String>()
                            })
                            .and_then(|d| d.parse().ok())
                            .unwrap_or(1u32)
                            .max(1)
                    };
                    let cell = (span("colspan"), span("rowspan"));
                    b.rows.last_mut().expect("row exists").push(cell);
                }
            }
            _ => {}
        }
    }
    done
}

fn grid_shape(rows: &[Vec<(u32, u32)>]) -> TableShape {
    let mut occupied: Vec<Vec<bool>> = vec![Vec::new(); rows.len()];
    let mut cols = 0usize;
    for (r, row) in rows.iter().enumerate() {
        let mut c = 0usize;
        for &(cs, rs) in row {
            while occupied[r].get(c).copied().unwrap_or(false) {
                c += 1;
            }
            let span_end = (r + rs as usize).min(rows.len());
            for line in &mut occupied[r..span_end] {
                if line.len() < c + cs as usize {
                    line.resize(c + cs as usize, false);
                }
                for cell in &mut line[c..c + cs as usize] {
                    *cell = true;
                }
            }
            c += cs as usize;
            cols = cols.max(c);
        }
    }
    TableShape {
        rows: rows.len() as u32,
        cols: cols as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_markup() {
        let d = analyze(
            "# 제목\n\n- **굵게** [링크](http://x) ![image](image_001.png)\n\
             1. 하나[^1]\n> 인용 &amp; <span style=\"c\">색</span>\n[^1]: 각주 글\n",
        );
        assert_eq!(d.headings, 1);
        assert_eq!(
            d.plain.lines().collect::<Vec<_>>(),
            vec!["제목", "", "굵게 링크 ", "1. 하나", "인용 & 색", "각주 글"]
        );
    }

    #[test]
    fn pipe_and_html_tables() {
        let d = analyze(
            "| a | b | c |\n| --- | --- | --- |\n| 1 | 2 | 3 |\n\n\
             <table><tr><td colspan=\"2\">x</td></tr><tr><td rowspan=\"2\">y</td><td>z</td></tr><tr><td>w</td></tr></table>\n",
        );
        assert_eq!(
            d.tables,
            vec![
                TableShape { rows: 3, cols: 2 },
                TableShape { rows: 2, cols: 3 }
            ]
        );
        assert!(d.plain.contains(" a   b   c "));
        assert!(!d.plain.contains("td"));
    }

    #[test]
    fn korean_angle_brackets_stay_text() {
        assert_eq!(analyze("<보기> 참고\n").plain, "<보기> 참고\n");
    }
}
