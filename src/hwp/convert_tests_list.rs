//! Tests for HWP binary list detection and grouping.
//!
//! Tests cover:
//! 1. `detect_list_kind` — per-paragraph classification
//! 2. `detect_list_kind_from_text` (via `detect_list_kind`) — text heuristics
//! 3. `hwp_to_ir` integration — grouping of consecutive list paragraphs

use super::{detect_list_kind, hwp_to_ir, leading_ordinal, ListKind};
use crate::hwp::model::{DocInfo, FileHeader, HwpDocument, HwpParagraph, HwpSection, ParaShape};
use crate::ir;
use std::collections::HashMap;

// -- helpers ------------------------------------------------------------------

fn make_para(text: &str) -> HwpParagraph {
    HwpParagraph {
        text: text.to_string(),
        char_shape_ids: Vec::new(),
        para_shape_id: 0,
        style_id: 0,
        controls: Vec::new(),
        raw_para_text: None,
    }
}

fn make_para_with_ps(text: &str, ps_id: u16) -> HwpParagraph {
    HwpParagraph {
        text: text.to_string(),
        char_shape_ids: Vec::new(),
        para_shape_id: ps_id,
        style_id: 0,
        controls: Vec::new(),
        raw_para_text: None,
    }
}

fn doc_info_empty() -> DocInfo {
    DocInfo::default()
}

/// `DocInfo` with `ParaShape[0].numbering_id = Some(1)`.
fn doc_info_with_numbering() -> DocInfo {
    let mut di = DocInfo::default();
    di.para_shapes.push(ParaShape {
        numbering_id: Some(1),
        ..Default::default()
    });
    di
}

fn make_hwp_doc(paras: Vec<HwpParagraph>) -> HwpDocument {
    HwpDocument {
        header: FileHeader::default(),
        doc_info: DocInfo::default(),
        sections: vec![HwpSection { paragraphs: paras }],
        bin_data: HashMap::new(),
        summary_title: None,
        summary_author: None,
        summary_subject: None,
        summary_keywords: Vec::new(),
    }
}

fn make_hwp_doc_with_di(doc_info: DocInfo, paras: Vec<HwpParagraph>) -> HwpDocument {
    HwpDocument {
        header: FileHeader::default(),
        doc_info,
        sections: vec![HwpSection { paragraphs: paras }],
        bin_data: HashMap::new(),
        summary_title: None,
        summary_author: None,
        summary_subject: None,
        summary_keywords: Vec::new(),
    }
}

// -- detect_list_kind: bullet characters (Tier 2 heuristic) ------------------

#[test]
fn detect_list_kind_bullet_circle_is_unordered() {
    let para = make_para("● First item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Unordered)
    );
}

#[test]
fn detect_list_kind_bullet_square_is_unordered() {
    let para = make_para("■ Second item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Unordered)
    );
}

#[test]
fn detect_list_kind_bullet_triangle_is_unordered() {
    let para = make_para("▶ Arrow item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Unordered)
    );
}

#[test]
fn detect_list_kind_bullet_dot_is_unordered() {
    let para = make_para("• Dot item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Unordered)
    );
}

#[test]
fn detect_list_kind_bullet_dash_is_unordered() {
    let para = make_para("- Dash item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Unordered)
    );
}

#[test]
fn detect_list_kind_bullet_star_is_unordered() {
    let para = make_para("* Star item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Unordered)
    );
}

// -- detect_list_kind: numbered prefixes (Tier 2 heuristic) ------------------

#[test]
fn detect_list_kind_number_dot_space_is_ordered() {
    let para = make_para("1. First item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Ordered)
    );
}

#[test]
fn detect_list_kind_number_paren_space_is_ordered() {
    let para = make_para("2) Second item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Ordered)
    );
}

#[test]
fn detect_list_kind_alpha_dot_space_is_ordered() {
    let para = make_para("a. Alpha item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Ordered)
    );
}

#[test]
fn detect_list_kind_alpha_paren_space_is_ordered() {
    let para = make_para("b) Beta item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Ordered)
    );
}

#[test]
fn detect_list_kind_three_digit_number_is_ordered() {
    let para = make_para("100. Centesimal item");
    assert_eq!(
        detect_list_kind(&para, &doc_info_empty()),
        Some(ListKind::Ordered)
    );
}

// -- detect_list_kind: plain paragraph -- no detection -----------------------

#[test]
fn detect_list_kind_plain_paragraph_returns_none() {
    let para = make_para("This is a normal paragraph.");
    assert_eq!(detect_list_kind(&para, &doc_info_empty()), None);
}

#[test]
fn detect_list_kind_empty_paragraph_returns_none() {
    let para = make_para("");
    assert_eq!(detect_list_kind(&para, &doc_info_empty()), None);
}

#[test]
fn detect_list_kind_whitespace_only_returns_none() {
    let para = make_para("   ");
    assert_eq!(detect_list_kind(&para, &doc_info_empty()), None);
}

// -- edge cases: year-like patterns must NOT match ----------------------------

#[test]
fn detect_list_kind_year_number_is_not_ordered() {
    // 4-digit number followed by a non-separator character must NOT match.
    let para = make_para("2026 business plan");
    assert_eq!(detect_list_kind(&para, &doc_info_empty()), None);
}

#[test]
fn detect_list_kind_four_digit_no_separator_is_not_ordered() {
    let para = make_para("1234 abcd");
    assert_eq!(detect_list_kind(&para, &doc_info_empty()), None);
}

#[test]
fn detect_list_kind_number_without_space_after_dot_is_not_ordered() {
    // "1.Second" -- no space after dot.
    let para = make_para("1.Second");
    assert_eq!(detect_list_kind(&para, &doc_info_empty()), None);
}

#[test]
fn detect_list_kind_bullet_without_trailing_space_is_not_list() {
    // Bullet directly followed by text, no separator space.
    let para = make_para("●text without space");
    assert_eq!(detect_list_kind(&para, &doc_info_empty()), None);
}

// -- Tier 1: binary numbering_id ---------------------------------------------

#[test]
fn detect_list_kind_numbering_id_with_bullet_text_is_unordered() {
    let di = doc_info_with_numbering();
    let para = make_para_with_ps("● Formal bullet", 0);
    assert_eq!(detect_list_kind(&para, &di), Some(ListKind::Unordered));
}

#[test]
fn detect_list_kind_numbering_id_with_ordered_text_is_ordered() {
    let di = doc_info_with_numbering();
    let para = make_para_with_ps("1. Formal numbered", 0);
    assert_eq!(detect_list_kind(&para, &di), Some(ListKind::Ordered));
}

#[test]
fn detect_list_kind_numbering_id_with_plain_text_defaults_to_unordered() {
    // numbering_id set but text has no recognisable prefix.
    let di = doc_info_with_numbering();
    let para = make_para_with_ps("Plain text in a formal list", 0);
    assert_eq!(detect_list_kind(&para, &di), Some(ListKind::Unordered));
}

#[test]
fn detect_list_kind_no_numbering_id_plain_text_returns_none() {
    let di = doc_info_with_numbering();
    // para_shape_id = 1 but doc_info only has one shape at index 0.
    let para = make_para_with_ps("Regular paragraph", 1);
    assert_eq!(detect_list_kind(&para, &di), None);
}

// -- hwp_to_ir: grouping consecutive list paragraphs -------------------------

#[test]
fn hwp_to_ir_bullet_paragraph_produces_list_block() {
    let hwp = make_hwp_doc(vec![make_para("- Single bullet")]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    assert!(
        matches!(
            doc.sections[0].blocks[0],
            ir::Block::List { ordered: false, .. }
        ),
        "expected unordered Block::List, got {:?}",
        doc.sections[0].blocks[0]
    );
}

#[test]
fn hwp_to_ir_numbered_paragraph_produces_ordered_list_block() {
    let hwp = make_hwp_doc(vec![make_para("1. Single item")]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    assert!(
        matches!(
            doc.sections[0].blocks[0],
            ir::Block::List { ordered: true, .. }
        ),
        "expected ordered Block::List, got {:?}",
        doc.sections[0].blocks[0]
    );
}

#[test]
fn hwp_to_ir_consecutive_bullets_produce_single_list() {
    let hwp = make_hwp_doc(vec![
        make_para("- Item one"),
        make_para("- Item two"),
        make_para("- Item three"),
    ]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(
        doc.sections[0].blocks.len(),
        1,
        "three consecutive bullet paragraphs must collapse into one Block::List"
    );
    if let ir::Block::List { ordered, items, .. } = &doc.sections[0].blocks[0] {
        assert!(!ordered, "must be unordered");
        assert_eq!(items.len(), 3);
    } else {
        panic!("expected Block::List");
    }
}

#[test]
fn hwp_to_ir_consecutive_numbered_produce_single_list() {
    let hwp = make_hwp_doc(vec![
        make_para("1. First"),
        make_para("2. Second"),
        make_para("3. Third"),
    ]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    if let ir::Block::List { ordered, items, .. } = &doc.sections[0].blocks[0] {
        assert!(*ordered, "must be ordered");
        assert_eq!(items.len(), 3);
    } else {
        panic!("expected Block::List");
    }
}

#[test]
fn hwp_to_ir_mixed_bullet_numbered_produces_two_lists() {
    let hwp = make_hwp_doc(vec![
        make_para("- Bullet A"),
        make_para("- Bullet B"),
        make_para("1. Number one"),
        make_para("2. Number two"),
    ]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(
        doc.sections[0].blocks.len(),
        2,
        "bullet list then numbered list must produce 2 Block::List values"
    );
    assert!(matches!(
        doc.sections[0].blocks[0],
        ir::Block::List { ordered: false, .. }
    ));
    assert!(matches!(
        doc.sections[0].blocks[1],
        ir::Block::List { ordered: true, .. }
    ));
}

#[test]
fn hwp_to_ir_plain_paragraph_between_lists_produces_three_blocks() {
    let hwp = make_hwp_doc(vec![
        make_para("- Bullet A"),
        make_para("- Bullet B"),
        make_para("Regular paragraph here."),
        make_para("- Bullet C"),
        make_para("- Bullet D"),
    ]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 3);
    assert!(matches!(doc.sections[0].blocks[0], ir::Block::List { .. }));
    assert!(matches!(
        doc.sections[0].blocks[1],
        ir::Block::Paragraph { .. }
    ));
    assert!(matches!(doc.sections[0].blocks[2], ir::Block::List { .. }));
}

#[test]
fn hwp_to_ir_regular_paragraph_not_detected_as_list() {
    let hwp = make_hwp_doc(vec![make_para("This is a normal sentence.")]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    assert!(
        matches!(doc.sections[0].blocks[0], ir::Block::Paragraph { .. }),
        "plain text must remain a Paragraph"
    );
}

#[test]
fn hwp_to_ir_year_paragraph_not_detected_as_list() {
    // 4-digit number followed by non-separator text must NOT become a list.
    let hwp = make_hwp_doc(vec![make_para("2026 business plan")]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    assert!(
        matches!(doc.sections[0].blocks[0], ir::Block::Paragraph { .. }),
        "year-like number prefix must remain a Paragraph, not become a list"
    );
}

#[test]
fn hwp_to_ir_heading_not_treated_as_list_item() {
    // Paragraph with heading_type = Some(0) must become a Heading, not a list.
    let mut di = DocInfo::default();
    di.para_shapes.push(ParaShape {
        heading_type: Some(0),
        ..Default::default()
    });
    // Even if text starts with "- " it should remain a Heading.
    let hwp = make_hwp_doc_with_di(di, vec![make_para_with_ps("- Heading text", 0)]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    assert!(
        matches!(doc.sections[0].blocks[0], ir::Block::Heading { .. }),
        "heading paragraph must remain a Heading even if text starts with bullet"
    );
}

#[test]
fn hwp_to_ir_binary_numbering_id_groups_plain_text_into_list() {
    // Paragraphs with numbering_id set (formal binary list) must become a
    // Block::List even when the text has no bullet/number prefix.
    let di = doc_info_with_numbering();
    let hwp = make_hwp_doc_with_di(
        di,
        vec![
            make_para_with_ps("First formal item", 0),
            make_para_with_ps("Second formal item", 0),
        ],
    );
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    if let ir::Block::List { items, .. } = &doc.sections[0].blocks[0] {
        assert_eq!(items.len(), 2);
    } else {
        panic!("expected Block::List for formal binary list paragraphs");
    }
}
// -- leading_ordinal: literal start-number capture (G002) --------------------

#[test]
fn leading_ordinal_parses_numeric_dot_and_paren() {
    assert_eq!(leading_ordinal("8. eight"), Some(8));
    assert_eq!(leading_ordinal("12) twelve"), Some(12));
    assert_eq!(leading_ordinal("  10. indented"), Some(10));
    assert_eq!(leading_ordinal("1."), Some(1));
}

#[test]
fn leading_ordinal_rejects_non_ordinal_markers() {
    assert_eq!(leading_ordinal("a. alpha"), None); // alphabetic → default start
    assert_eq!(leading_ordinal("- bullet"), None);
    assert_eq!(leading_ordinal("7-1. sub"), None); // '-' is not a separator
    assert_eq!(leading_ordinal("2026년"), None); // >3 digits / year-like
    assert_eq!(leading_ordinal("8.no space"), None); // no whitespace after sep
    assert_eq!(leading_ordinal("plain text"), None);
}

// -- hwp_to_ir: ordered-list start preservation (G002) -----------------------

#[test]
fn hwp_to_ir_ordered_list_preserves_nonone_start() {
    // A list whose first item is literally "8." must render starting at 8,
    // not be silently renumbered from 1.
    let hwp = make_hwp_doc(vec![
        make_para("8. Eight"),
        make_para("9. Nine"),
        make_para("10. Ten"),
    ]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    let ir::Block::List {
        ordered,
        start,
        items,
    } = &doc.sections[0].blocks[0]
    else {
        panic!("expected Block::List");
    };
    assert!(*ordered);
    assert_eq!(*start, 8, "list must keep the author's start ordinal 8");
    assert_eq!(items.len(), 3);
}

#[test]
fn hwp_to_ir_ordered_list_starting_at_one_is_unchanged() {
    // Regression guard: a normal 1-based list keeps start = 1.
    let hwp = make_hwp_doc(vec![make_para("1. First"), make_para("2. Second")]);
    let doc = hwp_to_ir(&hwp);
    let ir::Block::List { start, .. } = &doc.sections[0].blocks[0] else {
        panic!("expected Block::List");
    };
    assert_eq!(*start, 1);
}

#[test]
fn hwp_to_ir_list_resuming_after_interruption_keeps_source_numbering() {
    // The exact G002 bug: items 1-2, an interrupting free-typed "7-1." paragraph,
    // then items 8-9. The resumed list must start at 8, not restart at 1.
    let hwp = make_hwp_doc(vec![
        make_para("1. One"),
        make_para("2. Two"),
        make_para("  7-1. Free-typed sub-item paragraph"),
        make_para("8. Eight"),
        make_para("9. Nine"),
    ]);
    let doc = hwp_to_ir(&hwp);
    let blocks = &doc.sections[0].blocks;
    assert_eq!(blocks.len(), 3, "list / interruption / resumed list");
    let ir::Block::List {
        start: first_start,
        items: first_items,
        ..
    } = &blocks[0]
    else {
        panic!("expected first Block::List");
    };
    assert_eq!(*first_start, 1);
    assert_eq!(first_items.len(), 2);
    assert!(
        matches!(blocks[1], ir::Block::Paragraph { .. }),
        "7-1. is a plain paragraph"
    );
    let ir::Block::List {
        start: resumed_start,
        items: resumed_items,
        ..
    } = &blocks[2]
    else {
        panic!("expected resumed Block::List");
    };
    assert_eq!(
        *resumed_start, 8,
        "resumed list must continue at 8, not restart at 1"
    );
    assert_eq!(resumed_items.len(), 2);
}

// -- RED-TEAM: adversarial hostile inputs against the G002 fix ----------------

#[test]
fn leading_ordinal_boundary_hostile_inputs() {
    // Largest 3-digit ordinal is accepted.
    assert_eq!(leading_ordinal("999. x"), Some(999));
    // 4-digit run exceeds the 1-3 digit cap → rejected (year-like guard).
    assert_eq!(leading_ordinal("1000. x"), None);
    // Zero is a legal literal ordinal; parses cleanly, no special-casing.
    assert_eq!(leading_ordinal("0. x"), Some(0));
    // Separator present but no whitespace after it → not a list marker.
    assert_eq!(leading_ordinal("12)x"), None);
    // Separator with no leading digit → first char is not a digit → rejected.
    assert_eq!(leading_ordinal(") x"), None);
    // Empty string → `?` on the first char short-circuits to None (no panic).
    assert_eq!(leading_ordinal(""), None);
}

#[test]
fn hwp_to_ir_large_start_ordinal_no_overflow() {
    // A single item literally numbered 999 must yield start = 999 with no
    // panic, overflow, or truncation. Writer renders start + i (999, 1000, ...).
    let hwp = make_hwp_doc(vec![make_para("999. Item")]);
    let doc = hwp_to_ir(&hwp);
    let ir::Block::List { ordered, start, .. } = &doc.sections[0].blocks[0] else {
        panic!("expected Block::List");
    };
    assert!(*ordered);
    assert_eq!(
        *start, 999,
        "large start ordinal must be preserved verbatim"
    );
}

#[test]
fn hwp_to_ir_nonconsecutive_ordinals_use_first_as_start() {
    // Hostile: source ordinals jump (5 then 9). Within a single detected group
    // only the FIRST item's literal ordinal seeds `start`; the writer then
    // auto-increments (5, 6, ...). This is the accepted, documented behavior —
    // it must not crash and must not renumber from 1.
    let hwp = make_hwp_doc(vec![make_para("5. a"), make_para("9. b")]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(
        doc.sections[0].blocks.len(),
        1,
        "both items are one ordered group"
    );
    let ir::Block::List {
        ordered,
        start,
        items,
    } = &doc.sections[0].blocks[0]
    else {
        panic!("expected Block::List");
    };
    assert!(*ordered);
    assert_eq!(*start, 5, "group start is the first item's literal ordinal");
    assert_eq!(items.len(), 2);
}

#[test]
fn hwp_to_ir_mixed_sequence_starts_are_independent() {
    // Bullet list → ordered list starting at 3 → plain paragraph → ordered
    // list starting at 1. Each ordered Block::List must carry its own start
    // (3, then 1); the plain paragraph must flush the pending list.
    let hwp = make_hwp_doc(vec![
        make_para("- Bullet A"),
        make_para("- Bullet B"),
        make_para("3. Three"),
        make_para("4. Four"),
        make_para("Plain interruption."),
        make_para("1. One"),
        make_para("2. Two"),
    ]);
    let doc = hwp_to_ir(&hwp);
    let blocks = &doc.sections[0].blocks;
    assert_eq!(blocks.len(), 4, "unordered / ordered / paragraph / ordered");

    let ir::Block::List {
        ordered: b0_ordered,
        items: b0_items,
        ..
    } = &blocks[0]
    else {
        panic!("expected leading unordered Block::List");
    };
    assert!(!b0_ordered, "first block is the bullet list");
    assert_eq!(b0_items.len(), 2);

    let ir::Block::List {
        ordered: b1_ordered,
        start: b1_start,
        items: b1_items,
    } = &blocks[1]
    else {
        panic!("expected second ordered Block::List");
    };
    assert!(*b1_ordered);
    assert_eq!(*b1_start, 3, "ordered list must keep its author start of 3");
    assert_eq!(b1_items.len(), 2);

    assert!(
        matches!(blocks[2], ir::Block::Paragraph { .. }),
        "plain paragraph must flush the pending list"
    );

    let ir::Block::List {
        ordered: b3_ordered,
        start: b3_start,
        items: b3_items,
    } = &blocks[3]
    else {
        panic!("expected final ordered Block::List");
    };
    assert!(*b3_ordered);
    assert_eq!(
        *b3_start, 1,
        "second ordered list restarts at its own literal 1"
    );
    assert_eq!(b3_items.len(), 2);
}

#[test]
fn hwp_to_ir_alpha_marker_list_keeps_default_start() {
    // Alphabetic markers (`a.`, `b.`) are ordered lists but carry no numeric
    // ordinal, so `leading_ordinal` returns None and start defaults to 1.
    let hwp = make_hwp_doc(vec![make_para("a. x"), make_para("b. y")]);
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    let ir::Block::List {
        ordered,
        start,
        items,
    } = &doc.sections[0].blocks[0]
    else {
        panic!("expected Block::List");
    };
    assert!(*ordered, "alpha markers still form an ordered list");
    assert_eq!(*start, 1, "alpha markers keep the default start of 1");
    assert_eq!(items.len(), 2);
}
