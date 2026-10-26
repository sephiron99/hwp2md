//! Score one converter's Markdown against the XML reference.
//!
//! Text is compared on a *key*: letters and digits only (Unicode), so
//! whitespace, punctuation and Markdown markup never decide a match.  Each
//! reference paragraph is looked up in the output key in document order;
//! when the whole paragraph is not found, 12-character windows of it are
//! looked up instead, so one inserted footnote marker does not zero a long
//! paragraph.  Spacing is checked separately with whitespace tokens (어절).
//!
//! The search position follows every hit of a main paragraph (forward or
//! backward), and short paragraphs (< 6 key characters) are only looked up
//! near it, so a two-letter table cell cannot match somewhere random.

use crate::mdtext::{strip_markup_chars, MdDoc};
use crate::reference::{Kind, Reference, TableShape};
use std::collections::HashMap;

/// Window length used for the fallback lookup and the n-gram index.
const W: usize = 12;
/// Paragraphs shorter than this are only searched near the current position
/// (a 2-character paragraph would otherwise match anywhere).
const SHORT: usize = 6;
const NEAR_BACK: usize = 200;
const NEAR_FWD: usize = 400;

#[derive(Debug, Default, Clone)]
pub struct DocScore {
    /// Reference key characters per kind: (total, matched).
    pub chars: HashMap<Kind, (usize, usize)>,
    /// Output key characters and how many were covered by a reference match.
    pub md_chars: usize,
    pub md_covered: usize,
    /// Main paragraphs found, and the longest run of them in reference order.
    pub order_found: usize,
    pub order_lis: usize,
    /// 어절 (whitespace tokens): reference (main), output, and matches.
    pub tok_ref: usize,
    pub tok_out: usize,
    pub tok_recall_hit: usize,
    pub tok_precision_hit: usize,
    /// Tables with two or more columns (reference: visible tables).
    pub tables_ref: usize,
    pub tables_out: usize,
    pub tables_shape_hit: usize,
    pub headings: usize,
    /// Main paragraphs not fully found: (kind, text, matched chars, total).
    pub missed: Vec<(Kind, String, usize, usize)>,
    /// Longest runs of output key characters that matched no reference text.
    pub noise_runs: Vec<String>,
}

impl DocScore {
    /// Score for a converter that produced no output (failure): everything
    /// in the reference counts as missing.
    pub fn failed(reference: &Reference) -> DocScore {
        let mut s = DocScore::default();
        for u in &reference.units {
            s.chars.entry(u.kind).or_default().0 += key(&u.text).len();
        }
        s.tok_ref = main_tokens(reference).values().sum();
        s.tables_ref = ref_tables(reference).count();
        s
    }

    pub fn main_chars(&self) -> (usize, usize) {
        self.chars
            .iter()
            .filter(|(k, _)| k.is_main())
            .fold((0, 0), |(t, m), (_, &(tt, mm))| (t + tt, m + mm))
    }
}

/// Letters and digits only.
fn key(s: &str) -> Vec<char> {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn hash(w: &[char]) -> u64 {
    w.iter().fold(0xcbf2_9ce4_8422_2325, |h, &c| {
        (h ^ u64::from(c)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Output key plus an index from every W-character window to its positions.
struct Haystack {
    chars: Vec<char>,
    index: HashMap<u64, Vec<u32>>,
    covered: Vec<bool>,
}

impl Haystack {
    fn new(text: &str) -> Haystack {
        let chars = key(text);
        let mut index: HashMap<u64, Vec<u32>> = HashMap::new();
        if chars.len() >= W {
            for p in 0..=chars.len() - W {
                index
                    .entry(hash(&chars[p..p + W]))
                    .or_default()
                    .push(p as u32);
            }
        }
        let covered = vec![false; chars.len()];
        Haystack {
            chars,
            index,
            covered,
        }
    }

    fn matches_at(&self, p: usize, needle: &[char]) -> bool {
        self.chars.get(p..p + needle.len()) == Some(needle)
    }

    /// Find `needle` (at least W long), preferring the first hit at or after
    /// `cursor`, else the first hit anywhere.
    fn find_indexed(&self, needle: &[char], cursor: usize) -> Option<usize> {
        let candidates = self.index.get(&hash(&needle[..W]))?;
        let split = candidates.partition_point(|&p| (p as usize) < cursor);
        candidates[split..]
            .iter()
            .chain(&candidates[..split])
            .take(10_000)
            .map(|&p| p as usize)
            .find(|&p| self.matches_at(p, needle))
    }

    /// Find a short `needle` close to `cursor`.
    fn find_near(&self, needle: &[char], cursor: usize) -> Option<usize> {
        let n = needle.len();
        let start = cursor.saturating_sub(NEAR_BACK);
        let end = (cursor + NEAR_FWD).min(self.chars.len());
        if end < start + n {
            return None;
        }
        // Forward from the cursor first, then the short stretch behind it.
        (cursor.max(start)..=end - n)
            .chain(start..cursor.min(end - n + 1))
            .find(|&p| self.matches_at(p, needle))
    }

    fn find(&self, needle: &[char], cursor: usize) -> Option<usize> {
        if needle.len() >= W {
            self.find_indexed(needle, cursor)
        } else {
            self.find_near(needle, cursor)
        }
    }

    fn cover(&mut self, p: usize, n: usize) {
        for c in &mut self.covered[p..p + n] {
            *c = true;
        }
    }
}

pub fn score(reference: &Reference, out: &MdDoc) -> DocScore {
    let mut s = DocScore {
        headings: out.headings,
        ..DocScore::default()
    };
    let mut hay = Haystack::new(&out.plain);
    let mut cursor = 0usize;
    let mut first_positions: Vec<usize> = Vec::new();

    for unit in &reference.units {
        let needle = key(&unit.text);
        let n = needle.len();
        if n == 0 {
            continue;
        }
        let main = unit.kind.is_main();
        let mut matched = 0usize;
        let mut first: Option<usize> = None;

        if n < SHORT {
            if let Some(p) = hay.find_near(&needle, cursor) {
                hay.cover(p, n);
                matched = n;
                first = Some(p);
                if main {
                    cursor = p + n;
                }
            }
        } else if let Some(p) = hay.find(&needle, cursor) {
            hay.cover(p, n);
            matched = n;
            first = Some(p);
            // Long text is distinctive: follow it even backwards, so one
            // early mismatch cannot leave the cursor stranded ahead.
            if main {
                cursor = p + n;
            }
        } else if n > W {
            // Window fallback: W-character pieces, the last one aligned to
            // the end so the tail is covered too.
            let mut unit_hit = vec![false; n];
            let mut starts: Vec<usize> = (0..n - W + 1).step_by(W).collect();
            if starts.last() != Some(&(n - W)) {
                starts.push(n - W);
            }
            for st in starts {
                if let Some(p) = hay.find_indexed(&needle[st..st + W], cursor) {
                    // Grow the hit while unit and output keep agreeing, so
                    // text right up to an inserted/deleted spot still counts.
                    let mut back = 0;
                    while back < st && back < p && needle[st - back - 1] == hay.chars[p - back - 1]
                    {
                        back += 1;
                    }
                    let mut fwd = W;
                    while st + fwd < n
                        && p + fwd < hay.chars.len()
                        && needle[st + fwd] == hay.chars[p + fwd]
                    {
                        fwd += 1;
                    }
                    hay.cover(p - back, back + fwd);
                    for h in &mut unit_hit[st - back..st + fwd] {
                        *h = true;
                    }
                    first.get_or_insert(p - back);
                    if main {
                        cursor = p + fwd;
                    }
                }
            }
            matched = unit_hit.iter().filter(|&&h| h).count();
        }

        let entry = s.chars.entry(unit.kind).or_default();
        entry.0 += n;
        entry.1 += matched;
        if main {
            if let Some(p) = first {
                first_positions.push(p);
            }
            if matched < n {
                s.missed.push((unit.kind, unit.text.clone(), matched, n));
            }
        }
    }

    s.md_chars = hay.chars.len();
    s.md_covered = hay.covered.iter().filter(|&&c| c).count();
    let mut runs: Vec<String> = Vec::new();
    let mut start = None;
    for (i, &c) in hay.covered.iter().chain([&true]).enumerate() {
        match (c, start) {
            (false, None) => start = Some(i),
            (true, Some(st)) => {
                if i - st >= 8 {
                    runs.push(hay.chars[st..i].iter().collect());
                }
                start = None;
            }
            _ => {}
        }
    }
    runs.sort_by_key(|r| std::cmp::Reverse(r.chars().count()));
    runs.truncate(20);
    s.noise_runs = runs;
    s.order_found = first_positions.len();
    s.order_lis = lis_len(&first_positions);

    // 어절: multiset overlap of whitespace tokens.
    let ref_main = main_tokens(reference);
    let ref_all = tokens(
        reference
            .units
            .iter()
            .filter(|u| u.kind != Kind::Hidden)
            .map(|u| u.text.as_str()),
    );
    let out_tokens = tokens(std::iter::once(out.plain.as_str()));
    s.tok_ref = ref_main.values().sum();
    s.tok_out = out_tokens.values().sum();
    s.tok_recall_hit = overlap(&ref_main, &out_tokens);
    s.tok_precision_hit = overlap(&out_tokens, &ref_all);

    // Tables with ≥ 2 columns (visible ones in the reference): count and
    // exact (rows, cols) shape matches.
    let ref_tables: Vec<TableShape> = ref_tables(reference).collect();
    let out_tables: Vec<TableShape> = out.tables.iter().copied().filter(|t| t.cols >= 2).collect();
    s.tables_ref = ref_tables.len();
    s.tables_out = out_tables.len();
    s.tables_shape_hit = overlap(&shape_counts(&ref_tables), &shape_counts(&out_tables));
    s
}

fn ref_tables(reference: &Reference) -> impl Iterator<Item = TableShape> + '_ {
    reference.tables.iter().copied().filter(|t| t.cols >= 2)
}

fn shape_counts(tables: &[TableShape]) -> HashMap<(u32, u32), usize> {
    let mut m = HashMap::new();
    for t in tables {
        *m.entry((t.rows, t.cols)).or_insert(0) += 1;
    }
    m
}

fn main_tokens(reference: &Reference) -> HashMap<String, usize> {
    tokens(
        reference
            .units
            .iter()
            .filter(|u| u.kind.is_main())
            .map(|u| u.text.as_str()),
    )
}

/// Whitespace tokens that contain at least one letter or digit.
fn tokens<'a>(texts: impl Iterator<Item = &'a str>) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for text in texts {
        for tok in strip_markup_chars(text).split_whitespace() {
            if tok.chars().any(char::is_alphanumeric) {
                *m.entry(tok.to_string()).or_insert(0) += 1;
            }
        }
    }
    m
}

/// Σ min(a[k], b[k]) over keys of `a`.
fn overlap<K: std::hash::Hash + Eq>(a: &HashMap<K, usize>, b: &HashMap<K, usize>) -> usize {
    a.iter()
        .map(|(k, &n)| n.min(b.get(k).copied().unwrap_or(0)))
        .sum()
}

/// Length of the longest strictly increasing subsequence.
fn lis_len(xs: &[usize]) -> usize {
    let mut tails: Vec<usize> = Vec::new();
    for &x in xs {
        let i = tails.partition_point(|&t| t < x);
        if i == tails.len() {
            tails.push(x);
        } else {
            tails[i] = x;
        }
    }
    tails.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdtext::analyze;
    use crate::reference::Unit;

    fn reference(units: &[(&str, Kind)]) -> Reference {
        Reference {
            units: units
                .iter()
                .map(|&(t, kind)| Unit {
                    text: t.to_string(),
                    kind,
                })
                .collect(),
            tables: vec![TableShape { rows: 2, cols: 2 }],
        }
    }

    #[test]
    fn perfect_output_scores_full() {
        let r = reference(&[
            ("수사 과정에서의 피의자 인권 보장", Kind::Body),
            ("칸 하나", Kind::Cell),
            ("칸 둘", Kind::Cell),
        ]);
        let md = analyze(
            "# 수사 과정에서의 피의자 인권 보장\n\n| 칸 하나 | 칸 둘 |\n| --- | --- |\n| a | b |\n",
        );
        let s = score(&r, &md);
        assert_eq!(s.main_chars().0, s.main_chars().1);
        assert_eq!(s.order_lis, 3);
        assert_eq!(s.tok_recall_hit, s.tok_ref);
        assert_eq!((s.tables_ref, s.tables_out, s.tables_shape_hit), (1, 1, 1));
        assert_eq!(s.headings, 1);
    }

    #[test]
    fn inserted_marker_keeps_most_of_a_long_paragraph() {
        let body = "체포영장 발부의 요건에 관하여 형사소송법은 피의자가 죄를 범하였다고 의심할만한 상당한 이유가 있을 것을 규정하고 있다";
        let r = reference(&[(body, Kind::Body)]);
        let md = analyze(&body.replacen("형사소송법은", "형사소송법은 (주: 1) 각주)", 1));
        let (total, matched) = score(&r, &md).main_chars();
        assert!(matched * 10 >= total * 8, "{matched}/{total}");
    }

    #[test]
    fn lost_spacing_shows_in_tokens_not_chars() {
        let r = reference(&[("’18. 3. 7.(수) 회의", Kind::Body)]);
        let s = score(&r, &analyze("’18.3.7.(수) 회의\n"));
        assert_eq!(s.main_chars().0, s.main_chars().1);
        assert_eq!((s.tok_ref, s.tok_recall_hit), (4, 1));
    }

    #[test]
    fn missing_text_and_noise() {
        let r = reference(&[
            ("남아 있는 문단입니다", Kind::Body),
            ("사라진 문단입니다만", Kind::Body),
        ]);
        let s = score(&r, &analyze("남아 있는 문단입니다\n\n엉뚱한 글자\n"));
        let (total, matched) = s.main_chars();
        assert!(matched < total);
        assert!(s.md_covered < s.md_chars);
    }

    /// A paragraph the output moved to the end must not strand the search
    /// position there: the next long paragraph pulls it back, so the short
    /// cells that follow are still found.
    #[test]
    fn cursor_recovers_after_out_of_order_paragraph() {
        let moved = "맨 끝으로 옮겨진 문단입니다 순서가 바뀌었습니다";
        let next = "위의 사람에 대한 피의사건에 관하여 아래와 같이 확인하다";
        let r = reference(&[
            ("별지 제1호 서식 첫째", Kind::Body),
            (moved, Kind::Body),
            (next, Kind::Body),
            ("합의", Kind::Cell),
            ("여부", Kind::Cell),
        ]);
        let filler = "다른 서식의 본문 ".repeat(80);
        let md = format!("별지 제1호 서식 첫째\n\n{next}\n\n합의 | 여부\n\n{filler}\n\n{moved}\n");
        let s = score(&r, &analyze(&md));
        let cells = s.chars[&Kind::Cell];
        assert_eq!(cells.0, cells.1, "{:?}", s.missed);
    }

    #[test]
    fn lis() {
        assert_eq!(lis_len(&[1, 5, 2, 3, 9, 4]), 4);
        assert_eq!(lis_len(&[]), 0);
    }
}
