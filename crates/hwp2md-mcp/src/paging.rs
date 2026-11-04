//! Long Markdown is handed out in pages so one answer stays within what a
//! model can take in.  Offsets count characters, not bytes.

#[derive(Debug, PartialEq, Eq)]
pub struct Page<'a> {
    pub text: &'a str,
    /// Character offset where this page starts.
    pub start: usize,
    /// Character offset to ask for next, when more text follows.
    pub next: Option<usize>,
    pub total: usize,
}

/// The page of `text` starting at character `offset`, at most `max_chars`
/// long.  A page that does not reach the end stops after the last line
/// break in its second half, so lines are not cut in two.
pub fn page(text: &str, offset: usize, max_chars: usize) -> Page<'_> {
    let total = text.chars().count();
    let start = offset.min(total);
    let max_chars = max_chars.max(1);
    let byte_at = |chars: usize| {
        text.char_indices()
            .nth(chars)
            .map_or(text.len(), |(i, _)| i)
    };
    let from = byte_at(start);
    if total - start <= max_chars {
        return Page {
            text: &text[from..],
            start,
            next: None,
            total,
        };
    }
    let mut to = byte_at(start + max_chars);
    let window = &text[from..to];
    if let Some(nl) = window.rfind('\n') {
        if window[..nl].chars().count() >= max_chars / 2 {
            to = from + nl + 1;
        }
    }
    let end = start + text[from..to].chars().count();
    Page {
        text: &text[from..to],
        start,
        next: Some(end),
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_page() {
        let p = page("가나다", 0, 10);
        assert_eq!((p.text, p.next, p.total), ("가나다", None, 3));
    }

    #[test]
    fn pages_cover_the_text_and_break_at_lines() {
        let text = "첫째 줄입니다\n둘째 줄입니다\n셋째 줄입니다\n";
        let mut out = String::new();
        let mut offset = 0;
        loop {
            let p = page(text, offset, 10);
            assert!(p.text.chars().count() <= 10);
            out.push_str(p.text);
            match p.next {
                Some(n) => offset = n,
                None => break,
            }
        }
        assert_eq!(out, text);
        assert_eq!(page(text, 0, 10).text, "첫째 줄입니다\n");
    }

    #[test]
    fn offset_past_the_end_is_empty() {
        let p = page("abc", 10, 5);
        assert_eq!((p.text, p.start, p.next), ("", 3, None));
    }
}
