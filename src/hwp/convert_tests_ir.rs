use super::*;
use crate::hwp::model::{
    DocInfo, FileHeader, HwpControl, HwpDocument, HwpParagraph, HwpSection, ParaShape,
};

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn make_para(text: &str, para_shape_id: u16) -> HwpParagraph {
    HwpParagraph {
        text: text.to_string(),
        char_shape_ids: Vec::new(),
        para_shape_id,
        style_id: 0,
        controls: Vec::new(),
        raw_para_text: None,
    }
}

fn make_hwp_document() -> HwpDocument {
    HwpDocument {
        header: FileHeader::default(),
        doc_info: DocInfo::default(),
        sections: Vec::new(),
        bin_data: std::collections::HashMap::new(),
        summary_title: None,
        summary_author: None,
        summary_subject: None,
        summary_keywords: Vec::new(),
    }
}

// -----------------------------------------------------------------------
// hwp_to_ir — basic document conversion
// -----------------------------------------------------------------------

#[test]
fn hwp_to_ir_empty_document_produces_empty_ir() {
    let hwp = make_hwp_document();
    let doc = hwp_to_ir(&hwp);
    assert!(doc.sections.is_empty());
    assert!(doc.assets.is_empty());
    assert!(doc.metadata.title.is_none());
}

#[test]
fn hwp_to_ir_copies_metadata() {
    let mut hwp = make_hwp_document();
    hwp.summary_title = Some("My Title".into());
    hwp.summary_author = Some("The Author".into());
    hwp.summary_subject = Some("Subject matter".into());
    hwp.summary_keywords = vec!["key1".into(), "key2".into()];

    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.metadata.title.as_deref(), Some("My Title"));
    assert_eq!(doc.metadata.author.as_deref(), Some("The Author"));
    assert_eq!(doc.metadata.subject.as_deref(), Some("Subject matter"));
    assert_eq!(doc.metadata.keywords, vec!["key1", "key2"]);
}

#[test]
fn hwp_to_ir_paragraph_text_becomes_ir_paragraph() {
    let mut hwp = make_hwp_document();
    hwp.sections.push(HwpSection {
        paragraphs: vec![HwpParagraph {
            text: "Hello world".into(),
            char_shape_ids: Vec::new(),
            para_shape_id: 0,
            style_id: 0,
            controls: Vec::new(),
            raw_para_text: None,
        }],
    });
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections.len(), 1);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    if let ir::Block::Paragraph { inlines } = &doc.sections[0].blocks[0] {
        assert_eq!(inlines[0].text, "Hello world");
    } else {
        panic!("Expected Paragraph block");
    }
}

#[test]
fn hwp_to_ir_bin_data_becomes_asset() {
    let mut hwp = make_hwp_document();
    let png_header = vec![0x89u8, b'P', b'N', b'G', 0x00, 0x00, 0x00, 0x00];
    hwp.bin_data.insert(1, png_header.clone());

    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.assets.len(), 1);
    assert_eq!(doc.assets[0].mime_type, "image/png");
    assert_eq!(
        std::path::Path::new(&doc.assets[0].name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("png")),
        Some(true)
    );
    assert_eq!(doc.assets[0].data, png_header);
}

#[test]
fn hwp_to_ir_image_src_matches_asset_filename() {
    // Regression (2026-06-05): an embedded PNG must produce an Image block whose
    // `src` carries the real extension (`.png`) and exactly matches the extracted
    // asset filename — not the bin-unaware `.bin` placeholder. Previously the
    // Markdown linked `image_5.bin` while the asset was saved as `image_5.png`,
    // so every extracted image was a broken link.
    let mut hwp = make_hwp_document();
    let png = vec![0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    hwp.bin_data.insert(5, png);
    hwp.sections.push(HwpSection {
        paragraphs: vec![HwpParagraph {
            text: String::new(),
            char_shape_ids: Vec::new(),
            para_shape_id: 0,
            style_id: 0,
            controls: vec![HwpControl::Image {
                bin_data_id: 5,
                width: 10,
                height: 10,
            }],
            raw_para_text: None,
        }],
    });

    let doc = hwp_to_ir(&hwp);

    let asset_name = doc.assets[0].name.clone();
    assert_eq!(asset_name, "image_5.png", "asset filename");

    let img_src = doc
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| match b {
            ir::Block::Image { src, .. } => Some(src.clone()),
            _ => None,
        })
        .expect("an Image block");
    assert_eq!(
        img_src, "image_5.png",
        "image src must carry the real extension"
    );
    assert_eq!(
        img_src, asset_name,
        "image src must match the extracted asset filename"
    );
}

#[test]
fn hwp_to_ir_image_in_table_cell_src_resolved() {
    // The src-resolution pass must reach images nested inside table cells,
    // not just top-level paragraphs.
    use crate::hwp::model::HwpTableCell;
    let mut hwp = make_hwp_document();
    let jpg = vec![0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    hwp.bin_data.insert(2, jpg);
    let cell = HwpTableCell {
        row: 0,
        col: 0,
        row_span: 1,
        col_span: 1,
        vertical_align: 0,
        is_header: false,
        paragraphs: vec![HwpParagraph {
            text: String::new(),
            char_shape_ids: Vec::new(),
            para_shape_id: 0,
            style_id: 0,
            controls: vec![HwpControl::Image {
                bin_data_id: 2,
                width: 5,
                height: 5,
            }],
            raw_para_text: None,
        }],
    };
    hwp.sections.push(HwpSection {
        paragraphs: vec![HwpParagraph {
            text: String::new(),
            char_shape_ids: Vec::new(),
            para_shape_id: 0,
            style_id: 0,
            controls: vec![HwpControl::Table {
                row_count: 1,
                col_count: 1,
                cells: vec![cell],
            }],
            raw_para_text: None,
        }],
    });

    let doc = hwp_to_ir(&hwp);

    fn find_img(b: &ir::Block) -> Option<String> {
        match b {
            ir::Block::Image { src, .. } => Some(src.clone()),
            ir::Block::Table { rows, .. } => rows
                .iter()
                .flat_map(|r| &r.cells)
                .flat_map(|c| &c.blocks)
                .find_map(find_img),
            _ => None,
        }
    }
    let src = doc
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(find_img)
        .expect("nested Image block");
    assert_eq!(
        src, "image_2.jpg",
        "nested image src must resolve to the jpg asset filename"
    );
}

#[test]
fn hwp_to_ir_empty_paragraph_text_not_emitted() {
    let mut hwp = make_hwp_document();
    hwp.sections.push(HwpSection {
        paragraphs: vec![HwpParagraph {
            text: "   ".into(), // whitespace only → trimmed to empty
            char_shape_ids: Vec::new(),
            para_shape_id: 0,
            style_id: 0,
            controls: Vec::new(),
            raw_para_text: None,
        }],
    });
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 0);
}

// -----------------------------------------------------------------------
// hwp_to_ir — sequential footnote / endnote IDs (L1 fix)
// -----------------------------------------------------------------------

#[test]
fn hwp_to_ir_two_footnotes_get_sequential_ids() {
    let make_fn_ctrl = || HwpControl::FootnoteEndnote {
        is_endnote: false,
        paragraphs: Vec::new(),
    };
    let mut hwp = make_hwp_document();
    hwp.sections.push(HwpSection {
        paragraphs: vec![
            HwpParagraph {
                text: String::new(),
                char_shape_ids: Vec::new(),
                para_shape_id: 0,
                style_id: 0,
                controls: vec![make_fn_ctrl()],
                raw_para_text: None,
            },
            HwpParagraph {
                text: String::new(),
                char_shape_ids: Vec::new(),
                para_shape_id: 0,
                style_id: 0,
                controls: vec![make_fn_ctrl()],
                raw_para_text: None,
            },
        ],
    });
    let doc = hwp_to_ir(&hwp);
    let footnote_ids: Vec<&str> = doc.sections[0]
        .blocks
        .iter()
        .filter_map(|b| {
            if let ir::Block::Footnote { id, .. } = b {
                Some(id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(footnote_ids, vec!["footnote-1", "footnote-2"]);
}

#[test]
fn hwp_to_ir_footnote_emits_inline_ref_matching_definition() {
    // R2: a paragraph carrying a footnote control must emit a `footnote_ref`
    // inline (linking the call-out) whose id matches the Footnote definition
    // block. Previously the definition floated with no in-body reference.
    let mut hwp = make_hwp_document();
    hwp.sections.push(HwpSection {
        paragraphs: vec![HwpParagraph {
            text: "본문 문장".into(),
            char_shape_ids: Vec::new(),
            para_shape_id: 0,
            style_id: 0,
            controls: vec![HwpControl::FootnoteEndnote {
                is_endnote: false,
                paragraphs: vec![HwpParagraph {
                    text: "각주 내용".into(),
                    char_shape_ids: Vec::new(),
                    para_shape_id: 0,
                    style_id: 0,
                    controls: Vec::new(),
                    raw_para_text: None,
                }],
            }],
            raw_para_text: None,
        }],
    });

    let doc = hwp_to_ir(&hwp);
    let blocks = &doc.sections[0].blocks;

    let ref_id = blocks
        .iter()
        .find_map(|b| match b {
            ir::Block::Paragraph { inlines } => inlines.iter().find_map(|i| i.footnote_ref.clone()),
            _ => None,
        })
        .expect("a footnote_ref inline in the body paragraph");

    let def_id = blocks
        .iter()
        .find_map(|b| match b {
            ir::Block::Footnote { id, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("a Footnote definition block");

    assert_eq!(ref_id, def_id, "footnote ref id must match definition id");
    assert_eq!(ref_id, "footnote-1");

    let md = crate::md::write_markdown(&doc, false);
    assert!(
        md.contains("[^footnote-1]"),
        "body must reference the footnote; got: {md}"
    );
    assert!(
        md.contains("[^footnote-1]:"),
        "definition must be present; got: {md}"
    );
}

#[test]
fn hwp_to_ir_ordered_list_does_not_double_number() {
    // F-D3: HWP docs embed the literal "1." that the author typed; the list
    // detector wraps the paragraph as an ordered item and the MD writer adds
    // its own "1." → "1. \1. ..." (the writer escapes the embedded marker).
    // The embedded marker must be stripped from the list-item content so the
    // writer's marker is the only one.
    let mut hwp = make_hwp_document();
    hwp.sections.push(HwpSection {
        paragraphs: vec![make_para("1. 첫 항목", 0), make_para("2. 둘째 항목", 0)],
    });

    let doc = hwp_to_ir(&hwp);

    // IR: the list-item content must have the ordinal marker stripped.
    let ir::Block::List { items, .. } = &doc.sections[0].blocks[0] else {
        panic!("expected a List block, got {:?}", doc.sections[0].blocks);
    };
    let item_text = |i: usize| {
        let ir::Block::Paragraph { inlines } = &items[i].blocks[0] else {
            panic!("expected Paragraph in list item {i}");
        };
        inlines[0].text.clone()
    };
    assert_eq!(item_text(0), "첫 항목", "leading '1.' must be stripped");
    assert_eq!(item_text(1), "둘째 항목", "leading '2.' must be stripped");

    // MD: rendered once, no embedded/escaped second marker.
    let md = crate::md::write_markdown(&doc, false);
    assert!(
        !md.contains(r"\1.") && !md.contains(r"\2."),
        "no escaped embedded marker; got:\n{md}"
    );
    assert!(md.contains("1. 첫 항목"), "got:\n{md}");
    assert!(md.contains("2. 둘째 항목"), "got:\n{md}");
}

#[test]
fn hwp_to_ir_article_heading_splits_marker_from_body() {
    // F-D1/D2: "제2조(정의) 본문" → Heading "제2조(정의)" + Paragraph "본문",
    // even when the body is long (no ceiling). Fixes both the inconsistency
    // (some 조 were body) and the heading-includes-body defect.
    let mut hwp = make_hwp_document();
    let long_body = "이 고시에서 사용하는 용어의 뜻은 다음과 같다. ".repeat(6); // > 100 chars
    hwp.sections.push(HwpSection {
        paragraphs: vec![make_para(&format!("제2조(정의) {long_body}"), 0)],
    });

    let doc = hwp_to_ir(&hwp);
    let blocks = &doc.sections[0].blocks;

    let ir::Block::Heading { level, inlines } = &blocks[0] else {
        panic!("first block must be a Heading, got {:?}", blocks[0]);
    };
    assert_eq!(*level, 2);
    let head_text: String = inlines.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(
        head_text, "제2조(정의)",
        "heading must be only the marker+title"
    );

    let ir::Block::Paragraph { inlines: body } = &blocks[1] else {
        panic!(
            "second block must be the body Paragraph, got {:?}",
            blocks[1]
        );
    };
    let body_text: String = body.iter().map(|i| i.text.as_str()).collect();
    assert!(
        body_text.starts_with("이 고시에서"),
        "body must carry the article text; got {body_text:?}"
    );
}

#[test]
fn hwp_to_ir_generic_heading_trims_marker_whitespace() {
    // F-D4: a non-regulation heading (detected via para_shape heading_type)
    // must not retain HWP indentation whitespace in the rendered marker
    // (would render as "#   들여쓴 제목").
    let mut hwp = make_hwp_document();
    hwp.doc_info.para_shapes = vec![ParaShape {
        heading_type: Some(0), // → level 1
        ..Default::default()
    }];
    hwp.sections.push(HwpSection {
        paragraphs: vec![make_para("   들여쓴 제목", 0)],
    });
    let doc = hwp_to_ir(&hwp);
    let ir::Block::Heading { level, inlines } = &doc.sections[0].blocks[0] else {
        panic!("expected Heading, got {:?}", doc.sections[0].blocks);
    };
    assert_eq!(*level, 1);
    let t: String = inlines.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(
        t, "들여쓴 제목",
        "heading marker whitespace must be trimmed"
    );
}

#[test]
fn hwp_to_ir_chapter_divider_stays_single_heading() {
    // 장/편/절/관 dividers have no body — they remain a single heading block.
    let mut hwp = make_hwp_document();
    hwp.sections.push(HwpSection {
        paragraphs: vec![make_para("제1장 총칙", 0)],
    });
    let doc = hwp_to_ir(&hwp);
    assert_eq!(
        doc.sections[0].blocks.len(),
        1,
        "divider = one heading, no body"
    );
    let ir::Block::Heading { level, inlines } = &doc.sections[0].blocks[0] else {
        panic!("expected Heading");
    };
    assert_eq!(*level, 1);
    let t: String = inlines.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(t, "제1장 총칙");
}

#[test]
fn hwp_to_ir_header_footer_controls_hoisted_to_section() {
    // R3: HWP binary header/footer controls must populate Section.header/footer
    // (previously the HWP path set them to None via ..Default).
    let mut hwp = make_hwp_document();
    let head_para = HwpParagraph {
        text: "머리말 텍스트".into(),
        char_shape_ids: Vec::new(),
        para_shape_id: 0,
        style_id: 0,
        controls: Vec::new(),
        raw_para_text: None,
    };
    let foot_para = HwpParagraph {
        text: "꼬리말 텍스트".into(),
        char_shape_ids: Vec::new(),
        para_shape_id: 0,
        style_id: 0,
        controls: Vec::new(),
        raw_para_text: None,
    };
    hwp.sections.push(HwpSection {
        paragraphs: vec![HwpParagraph {
            text: "본문".into(),
            char_shape_ids: Vec::new(),
            para_shape_id: 0,
            style_id: 0,
            controls: vec![
                HwpControl::HeaderFooter {
                    is_footer: false,
                    paragraphs: vec![head_para],
                },
                HwpControl::HeaderFooter {
                    is_footer: true,
                    paragraphs: vec![foot_para],
                },
            ],
            raw_para_text: None,
        }],
    });

    let doc = hwp_to_ir(&hwp);
    let section = &doc.sections[0];

    let header = section
        .header
        .as_ref()
        .expect("Section.header must be Some");
    assert!(
        header
            .iter()
            .any(|b| matches!(b, ir::Block::Paragraph { inlines }
            if inlines.iter().any(|i| i.text.contains("머리말")))),
        "header must carry the header text; got {header:?}"
    );

    let footer = section
        .footer
        .as_ref()
        .expect("Section.footer must be Some");
    assert!(
        footer
            .iter()
            .any(|b| matches!(b, ir::Block::Paragraph { inlines }
            if inlines.iter().any(|i| i.text.contains("꼬리말")))),
        "footer must carry the footer text; got {footer:?}"
    );

    // The header/footer controls must NOT leak into the body blocks.
    assert!(
        !section
            .blocks
            .iter()
            .any(|b| matches!(b, ir::Block::Paragraph { inlines }
            if inlines.iter().any(|i| i.text.contains("머리말") || i.text.contains("꼬리말")))),
        "header/footer text must not appear in body blocks"
    );
}

#[test]
fn hwp_to_ir_two_endnotes_get_sequential_ids() {
    let make_en_ctrl = || HwpControl::FootnoteEndnote {
        is_endnote: true,
        paragraphs: Vec::new(),
    };
    let mut hwp = make_hwp_document();
    hwp.sections.push(HwpSection {
        paragraphs: vec![
            HwpParagraph {
                text: String::new(),
                char_shape_ids: Vec::new(),
                para_shape_id: 0,
                style_id: 0,
                controls: vec![make_en_ctrl()],
                raw_para_text: None,
            },
            HwpParagraph {
                text: String::new(),
                char_shape_ids: Vec::new(),
                para_shape_id: 0,
                style_id: 0,
                controls: vec![make_en_ctrl()],
                raw_para_text: None,
            },
        ],
    });
    let doc = hwp_to_ir(&hwp);
    let endnote_ids: Vec<&str> = doc.sections[0]
        .blocks
        .iter()
        .filter_map(|b| {
            if let ir::Block::Footnote { id, .. } = b {
                Some(id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(endnote_ids, vec!["endnote-1", "endnote-2"]);
}

#[test]
fn hwp_to_ir_footnote_and_endnote_counters_are_independent() {
    let mut hwp = make_hwp_document();
    hwp.sections.push(HwpSection {
        paragraphs: vec![
            HwpParagraph {
                text: String::new(),
                char_shape_ids: Vec::new(),
                para_shape_id: 0,
                style_id: 0,
                controls: vec![HwpControl::FootnoteEndnote {
                    is_endnote: false,
                    paragraphs: Vec::new(),
                }],
                raw_para_text: None,
            },
            HwpParagraph {
                text: String::new(),
                char_shape_ids: Vec::new(),
                para_shape_id: 0,
                style_id: 0,
                controls: vec![HwpControl::FootnoteEndnote {
                    is_endnote: true,
                    paragraphs: Vec::new(),
                }],
                raw_para_text: None,
            },
            HwpParagraph {
                text: String::new(),
                char_shape_ids: Vec::new(),
                para_shape_id: 0,
                style_id: 0,
                controls: vec![HwpControl::FootnoteEndnote {
                    is_endnote: false,
                    paragraphs: Vec::new(),
                }],
                raw_para_text: None,
            },
        ],
    });
    let doc = hwp_to_ir(&hwp);
    let ids: Vec<&str> = doc.sections[0]
        .blocks
        .iter()
        .filter_map(|b| {
            if let ir::Block::Footnote { id, .. } = b {
                Some(id.as_str())
            } else {
                None
            }
        })
        .collect();
    // footnote counter and endnote counter are independent sequences.
    assert_eq!(ids, vec!["footnote-1", "endnote-1", "footnote-2"]);
}

// -----------------------------------------------------------------------
// hwp_to_ir — heading detection uses make_para helper
// -----------------------------------------------------------------------

#[test]
fn hwp_to_ir_heading_para_shape_becomes_heading_block() {
    let mut hwp = make_hwp_document();
    // Insert a para_shape with heading_type = Some(0) at index 0.
    hwp.doc_info.para_shapes.push(ParaShape {
        heading_type: Some(0),
        ..Default::default()
    });
    hwp.sections.push(HwpSection {
        paragraphs: vec![make_para("Top Heading", 0)],
    });
    let doc = hwp_to_ir(&hwp);
    assert_eq!(doc.sections[0].blocks.len(), 1);
    if let ir::Block::Heading { level, inlines } = &doc.sections[0].blocks[0] {
        assert_eq!(*level, 1);
        assert_eq!(inlines[0].text, "Top Heading");
    } else {
        panic!(
            "expected Heading block, got {:?}",
            doc.sections[0].blocks[0]
        );
    }
}
