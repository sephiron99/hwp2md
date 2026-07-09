use super::common::find_children_end;
use super::hyperlink::parse_hyperlink_url;
use super::image::parse_gshape_ctrl;
use super::ruby::{fixup_ruby_base_text, parse_ruby_ctrl};
use super::table::parse_table_ctrl;
use crate::hwp::model::{HwpControl, HwpParagraph};
use crate::hwp::reader::{extract_paragraph_text_with_raw, parse_char_shape_refs};
use crate::hwp::record::{
    read_utf16le_str, Record, CTRL_COL_BREAK, CTRL_ENDNOTE, CTRL_FOOTNOTE, CTRL_GSHAPE,
    CTRL_HYPERLINK, CTRL_PAGE_BREAK, CTRL_PAGE_FOOTER, CTRL_PAGE_HEADER, CTRL_RUBY, CTRL_TABLE,
    HWPTAG_CTRL_HEADER, HWPTAG_EQEDIT, HWPTAG_PARA_CHAR_SHAPE, HWPTAG_PARA_HEADER,
    HWPTAG_PARA_TEXT,
};

/// Extract `HwpParagraph`s from records in `[start, end)`, treating them as a
/// self-contained sub-stream (e.g. a table cell or footnote body).
pub(crate) fn extract_paragraphs_from_range(
    records: &[Record],
    start: usize,
    end: usize,
) -> Vec<HwpParagraph> {
    let mut paras: Vec<HwpParagraph> = Vec::new();
    let mut current: Option<HwpParagraph> = None;
    let mut idx = start;

    while idx < end {
        let rec = &records[idx];
        match rec.tag_id {
            HWPTAG_PARA_HEADER => {
                // HWP 5.0 PARA_HEADER layout (Hancom spec §3.2.1):
                //   bytes[0..4]  UINT32  nChar         paragraph char count
                //   bytes[4..6]  UINT16  nParaShapeID  PARA_SHAPE record index
                //   bytes[6]     UINT8   nStyleID      style-sheet index (0 = Normal)
                //   bytes[7]     UINT8   flags         heading/numbering flags
                if let Some(mut p) = current.take() {
                    fixup_ruby_base_text(&mut p);
                    p.raw_para_text = None;
                    paras.push(p);
                }
                let para_shape_id = if rec.data.len() >= 6 {
                    u16::from_le_bytes([rec.data[4], rec.data[5]])
                } else {
                    0
                };
                // nStyleID is UINT8 at byte[6]; reading two bytes would corrupt the
                // value with the flags byte when any flag bit is set.
                let style_id = if rec.data.len() >= 7 { rec.data[6] } else { 0 };
                current = Some(HwpParagraph {
                    text: String::new(),
                    char_shape_ids: Vec::new(),
                    para_shape_id,
                    style_id,
                    controls: Vec::new(),
                    raw_para_text: None,
                });
                idx += 1;
            }
            HWPTAG_PARA_TEXT => {
                if let Some(ref mut p) = current {
                    let (text, raw) = extract_paragraph_text_with_raw(&rec.data);
                    p.text = text;
                    p.raw_para_text = Some(raw);
                }
                idx += 1;
            }
            HWPTAG_PARA_CHAR_SHAPE => {
                if let Some(ref mut p) = current {
                    p.char_shape_ids = parse_char_shape_refs(&rec.data);
                }
                idx += 1;
            }
            HWPTAG_EQEDIT => {
                if let Some(ref mut p) = current {
                    // HWP 5.0 EQEDIT record layout (Hancom spec 표 118 / node-hwp
                    // `record.format`): a UINT32 attribute word (only the low bit,
                    // LineMode, is meaningful) precedes the equation script, which
                    // is stored as a length-prefixed WCHAR string.  The script
                    // therefore begins at byte offset 4, NOT 2.  Reading at offset
                    // 2 mistook the high 2 bytes of the attribute for the WCHAR
                    // length prefix, so an empty script (`len == 0`) was misread as
                    // a single NUL code unit and produced a `Block::Math { tex:
                    // "\0" }` that leaked a raw NUL byte into Markdown as `$<NUL>$`.
                    let (script, _) = read_utf16le_str(&rec.data, 4);
                    // Drop control-only/whitespace scripts (e.g. an empty anchor
                    // equation object): they carry no renderable math and must not
                    // emit a `$…$` span.
                    if script
                        .chars()
                        .any(|c| !c.is_control() && !c.is_whitespace())
                    {
                        p.controls.push(HwpControl::Equation { script });
                    }
                }
                idx += 1;
            }
            HWPTAG_CTRL_HEADER => {
                // Nested controls inside cells (e.g. images within a table cell).
                // Parse and attach to current paragraph, then skip the subtree.
                if let Some(ctrl) = parse_ctrl_header_at(records, idx) {
                    if let Some(ref mut p) = current {
                        p.controls.push(ctrl);
                    }
                }
                idx = find_children_end(records, idx);
            }
            _ => {
                idx += 1;
            }
        }
    }

    if let Some(mut p) = current {
        fixup_ruby_base_text(&mut p);
        p.raw_para_text = None;
        paras.push(p);
    }
    paras
}

/// Parse the `CTRL_HEADER` at `ctrl_idx` and return the corresponding
/// `HwpControl` variant, or `None` if the control type is unknown/malformed.
pub(crate) fn parse_ctrl_header_at(records: &[Record], ctrl_idx: usize) -> Option<HwpControl> {
    let rec = &records[ctrl_idx];
    if rec.data.len() < 4 {
        tracing::debug!(
            "CTRL_HEADER at index {ctrl_idx}: data too short ({} bytes)",
            rec.data.len()
        );
        return None;
    }

    let ctrl_id = u32::from_le_bytes([rec.data[0], rec.data[1], rec.data[2], rec.data[3]]);

    match ctrl_id {
        CTRL_TABLE => {
            let (row_count, col_count, cells) = parse_table_ctrl(records, ctrl_idx);
            tracing::trace!(
                "Parsed table: {row_count}x{col_count}, {} cells",
                cells.len()
            );
            Some(HwpControl::Table {
                row_count,
                col_count,
                cells,
            })
        }
        CTRL_GSHAPE => {
            let (bin_data_id, width, height) = parse_gshape_ctrl(records, ctrl_idx);
            Some(HwpControl::Image {
                bin_data_id,
                width,
                height,
            })
        }
        CTRL_FOOTNOTE => {
            let ctrl_end = find_children_end(records, ctrl_idx);
            let paragraphs = extract_paragraphs_from_range(records, ctrl_idx + 1, ctrl_end);
            Some(HwpControl::FootnoteEndnote {
                is_endnote: false,
                paragraphs,
            })
        }
        CTRL_ENDNOTE => {
            let ctrl_end = find_children_end(records, ctrl_idx);
            let paragraphs = extract_paragraphs_from_range(records, ctrl_idx + 1, ctrl_end);
            Some(HwpControl::FootnoteEndnote {
                is_endnote: true,
                paragraphs,
            })
        }
        CTRL_PAGE_HEADER => {
            let ctrl_end = find_children_end(records, ctrl_idx);
            let paragraphs = extract_paragraphs_from_range(records, ctrl_idx + 1, ctrl_end);
            Some(HwpControl::HeaderFooter {
                is_footer: false,
                paragraphs,
            })
        }
        CTRL_PAGE_FOOTER => {
            let ctrl_end = find_children_end(records, ctrl_idx);
            let paragraphs = extract_paragraphs_from_range(records, ctrl_idx + 1, ctrl_end);
            Some(HwpControl::HeaderFooter {
                is_footer: true,
                paragraphs,
            })
        }
        CTRL_HYPERLINK => {
            let url = parse_hyperlink_url(rec);
            if url.is_empty() {
                None
            } else {
                Some(HwpControl::Hyperlink { url })
            }
        }
        CTRL_RUBY => {
            if let Some(ruby_text) = parse_ruby_ctrl(rec) {
                Some(HwpControl::Ruby {
                    base_text: String::new(),
                    ruby_text,
                })
            } else {
                tracing::debug!("CTRL_RUBY at index {ctrl_idx}: data too short, skipping");
                None
            }
        }
        CTRL_PAGE_BREAK => Some(HwpControl::PageBreak),
        CTRL_COL_BREAK => Some(HwpControl::ColumnBreak),
        _ => {
            tracing::trace!("CTRL_HEADER at index {ctrl_idx}: unhandled ctrl_id=0x{ctrl_id:08X}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hwp::reader::encode_u16s_test;
    use crate::hwp::record::{
        CTRL_ENDNOTE, CTRL_FOOTNOTE, CTRL_HYPERLINK, CTRL_PAGE_BREAK, CTRL_PAGE_FOOTER,
        CTRL_PAGE_HEADER, CTRL_TABLE, HWPTAG_CTRL_HEADER, HWPTAG_EQEDIT, HWPTAG_PARA_HEADER,
        HWPTAG_PARA_TEXT, HWPTAG_TABLE,
    };

    fn make_record_with_data(tag_id: u16, level: u16, data: Vec<u8>) -> Record {
        Record {
            tag_id,
            level,
            data,
        }
    }

    fn make_table_record(level: u16, row_count: u16, col_count: u16) -> Record {
        let mut data = vec![0u8; 8];
        data[4..6].copy_from_slice(&row_count.to_le_bytes());
        data[6..8].copy_from_slice(&col_count.to_le_bytes());
        make_record_with_data(HWPTAG_TABLE, level, data)
    }

    fn make_ctrl_header_table(level: u16) -> Record {
        make_record_with_data(HWPTAG_CTRL_HEADER, level, CTRL_TABLE.to_le_bytes().to_vec())
    }

    // -----------------------------------------------------------------------
    // parse_ctrl_header_at
    // -----------------------------------------------------------------------

    #[test]
    fn parse_ctrl_header_at_table_returns_table_control() {
        let records = vec![make_ctrl_header_table(0), make_table_record(1, 3, 4)];
        let ctrl = parse_ctrl_header_at(&records, 0).expect("should return Some");
        assert!(
            matches!(
                ctrl,
                HwpControl::Table {
                    row_count: 3,
                    col_count: 4,
                    ..
                }
            ),
            "expected Table{{row_count:3, col_count:4}}, got {ctrl:?}"
        );
    }

    #[test]
    fn parse_ctrl_header_at_short_data_returns_none() {
        let records = vec![make_record_with_data(HWPTAG_CTRL_HEADER, 0, vec![0u8; 2])];
        assert!(parse_ctrl_header_at(&records, 0).is_none());
    }

    #[test]
    fn parse_ctrl_header_at_unknown_ctrl_id_returns_none() {
        let unknown_id: u32 = 0xDEAD_BEEF;
        let records = vec![make_record_with_data(
            HWPTAG_CTRL_HEADER,
            0,
            unknown_id.to_le_bytes().to_vec(),
        )];
        assert!(parse_ctrl_header_at(&records, 0).is_none());
    }

    #[test]
    fn parse_ctrl_header_at_footnote_returns_footnote_endnote() {
        let fn_id = CTRL_FOOTNOTE.to_le_bytes().to_vec();
        let records = vec![make_record_with_data(HWPTAG_CTRL_HEADER, 0, fn_id)];
        let ctrl = parse_ctrl_header_at(&records, 0).expect("Some");
        assert!(matches!(
            ctrl,
            HwpControl::FootnoteEndnote {
                is_endnote: false,
                ..
            }
        ));
    }

    #[test]
    fn parse_ctrl_header_at_endnote_returns_footnote_endnote() {
        let en_id = CTRL_ENDNOTE.to_le_bytes().to_vec();
        let records = vec![make_record_with_data(HWPTAG_CTRL_HEADER, 0, en_id)];
        let ctrl = parse_ctrl_header_at(&records, 0).expect("Some");
        assert!(matches!(
            ctrl,
            HwpControl::FootnoteEndnote {
                is_endnote: true,
                ..
            }
        ));
    }

    #[test]
    fn parse_ctrl_header_at_header_returns_header_footer() {
        let head_id = CTRL_PAGE_HEADER.to_le_bytes().to_vec();
        let records = vec![make_record_with_data(HWPTAG_CTRL_HEADER, 0, head_id)];
        let ctrl = parse_ctrl_header_at(&records, 0).expect("Some");
        assert!(matches!(
            ctrl,
            HwpControl::HeaderFooter {
                is_footer: false,
                ..
            }
        ));
    }

    #[test]
    fn parse_ctrl_header_at_footer_returns_header_footer() {
        let foot_id = CTRL_PAGE_FOOTER.to_le_bytes().to_vec();
        let records = vec![make_record_with_data(HWPTAG_CTRL_HEADER, 0, foot_id)];
        let ctrl = parse_ctrl_header_at(&records, 0).expect("Some");
        assert!(matches!(
            ctrl,
            HwpControl::HeaderFooter {
                is_footer: true,
                ..
            }
        ));
    }

    #[test]
    fn parse_ctrl_header_at_page_break_returns_page_break() {
        let pb_id = CTRL_PAGE_BREAK.to_le_bytes().to_vec();
        let records = vec![make_record_with_data(HWPTAG_CTRL_HEADER, 0, pb_id)];
        let ctrl = parse_ctrl_header_at(&records, 0).expect("Some");
        assert!(matches!(ctrl, HwpControl::PageBreak));
    }

    #[test]
    fn parse_ctrl_header_at_hyperlink_returns_control() {
        let url_chars: Vec<u16> = "https://test.org".encode_utf16().collect();
        let mut data = CTRL_HYPERLINK.to_le_bytes().to_vec();
        data.extend_from_slice(&(u16::try_from(url_chars.len()).unwrap()).to_le_bytes());
        for ch in &url_chars {
            data.extend_from_slice(&ch.to_le_bytes());
        }
        let records = vec![Record {
            tag_id: HWPTAG_CTRL_HEADER,
            level: 0,
            data,
        }];
        let ctrl = parse_ctrl_header_at(&records, 0).expect("Some");
        assert!(matches!(ctrl, HwpControl::Hyperlink { url } if url == "https://test.org"));
    }

    #[test]
    fn parse_ctrl_header_at_hyperlink_empty_url_returns_none() {
        let mut data = CTRL_HYPERLINK.to_le_bytes().to_vec();
        data.extend_from_slice(&0u16.to_le_bytes());
        let records = vec![Record {
            tag_id: HWPTAG_CTRL_HEADER,
            level: 0,
            data,
        }];
        assert!(parse_ctrl_header_at(&records, 0).is_none());
    }

    // -----------------------------------------------------------------------
    // extract_paragraphs_from_range
    // -----------------------------------------------------------------------

    fn make_record_plain(tag_id: u16, level: u16) -> Record {
        Record {
            tag_id,
            level,
            data: Vec::new(),
        }
    }

    fn make_record_with_data_plain(tag_id: u16, level: u16, data: Vec<u8>) -> Record {
        Record {
            tag_id,
            level,
            data,
        }
    }

    #[test]
    fn extract_paragraphs_from_range_empty_range() {
        let records: Vec<Record> = Vec::new();
        let paras = extract_paragraphs_from_range(&records, 0, 0);
        assert!(paras.is_empty());
    }

    #[test]
    fn extract_paragraphs_from_range_single_paragraph() {
        let text_data = encode_u16s_test(&[u16::from(b'H'), u16::from(b'i')]);
        let mut ph_data = vec![0u8; 6];
        ph_data[4] = 0;
        ph_data[5] = 0;

        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, ph_data),
            make_record_with_data_plain(HWPTAG_PARA_TEXT, 3, text_data),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].text, "Hi");
    }

    #[test]
    fn extract_paragraphs_from_range_multiple_paragraphs() {
        let text_a = encode_u16s_test(&[u16::from(b'A')]);
        let text_b = encode_u16s_test(&[u16::from(b'B')]);
        let ph_data = vec![0u8; 6];

        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, ph_data.clone()),
            make_record_with_data_plain(HWPTAG_PARA_TEXT, 3, text_a),
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, ph_data.clone()),
            make_record_with_data_plain(HWPTAG_PARA_TEXT, 3, text_b),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 2);
        assert_eq!(paras[0].text, "A");
        assert_eq!(paras[1].text, "B");
    }

    #[test]
    fn extract_paragraphs_from_range_ignores_unknown_tags() {
        let ph_data = vec![0u8; 6];
        let records = vec![
            make_record_plain(0xFFFF, 0), // unknown tag before any paragraph
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 0, ph_data),
            make_record_plain(0xFFFF, 1), // unknown tag inside paragraph
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].text, "");
    }

    #[test]
    fn para_header_style_id_is_single_byte_not_corrupted_by_flags() {
        // HWP spec §3.2.1: nStyleID is UINT8 at byte[6]; byte[7] is flags.
        // Reading 2 bytes (u16) would corrupt the value when flags != 0.
        let mut ph_data = vec![0u8; 8];
        ph_data[4] = 1; // para_shape_id low byte
        ph_data[5] = 0; // para_shape_id high byte
        ph_data[6] = 3; // nStyleID = 3
        ph_data[7] = 0xFF; // flags byte — must NOT bleed into style_id

        let records = vec![make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, ph_data)];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].para_shape_id, 1);
        assert_eq!(
            paras[0].style_id, 3,
            "flags byte must not contaminate style_id"
        );
    }

    #[test]
    fn para_header_style_id_zero_with_heading_flag_set() {
        // Motivating bug: nStyleID=0 (Normal), heading flag = 0x80.
        // Old code: u16 read → 0x80_00 = 32768 → styles[32768] OOB → tier-2 skipped.
        // New code: 0 → styles[0] = "Normal" → correct.
        let mut ph_data = vec![0u8; 8];
        ph_data[6] = 0; // Normal style
        ph_data[7] = 0x80; // heading/numbering flag bit

        let records = vec![make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, ph_data)];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(
            paras[0].style_id, 0,
            "heading flag must not corrupt style_id"
        );
    }

    #[test]
    fn para_header_short_data_does_not_panic() {
        // rec.data.len() == 6: para_shape_id present, no style byte → style_id = 0.
        let ph_data = vec![0u8; 6];
        let records = vec![make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, ph_data)];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].style_id, 0);
    }
    // -----------------------------------------------------------------------
    // HWPTAG_EQEDIT parsing (regression: `$<NUL>$` artifact, offset bug)
    // -----------------------------------------------------------------------

    /// Build an EQEDIT record body: UINT32 attribute word + length-prefixed
    /// UTF-16LE script, matching the real HWP 5.0 layout.
    fn make_eqedit_record(level: u16, script: &[u16]) -> Record {
        let mut data = vec![0u8; 4]; // UINT32 attribute
        data.extend_from_slice(&u16::try_from(script.len()).unwrap().to_le_bytes());
        for &u in script {
            data.extend_from_slice(&u.to_le_bytes());
        }
        make_record_with_data_plain(HWPTAG_EQEDIT, level, data)
    }

    #[test]
    fn eqedit_empty_script_produces_no_equation_control() {
        // Real fixture case: an empty anchor equation (len == 0 at offset 4).
        // Must NOT push any Equation control — the old offset-2 read misread the
        // empty script as a single NUL WCHAR and leaked `$<NUL>$` into Markdown.
        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, vec![0u8; 6]),
            make_eqedit_record(3, &[]),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert!(
            paras[0].controls.is_empty(),
            "empty EQEDIT script must yield no control, got {:?}",
            paras[0].controls
        );
    }

    #[test]
    fn eqedit_nul_only_script_is_dropped() {
        // A script that is a single NUL code unit (0x0000) carries no renderable
        // math and must be dropped by the control/whitespace guard.
        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, vec![0u8; 6]),
            make_eqedit_record(3, &[0x0000]),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert!(
            paras[0].controls.is_empty(),
            "NUL-only EQEDIT script must be dropped, got {:?}",
            paras[0].controls
        );
    }

    #[test]
    fn eqedit_real_script_read_at_offset_four() {
        // A genuine script ("a+b") stored after the 4-byte attribute must be read
        // verbatim — proving the offset-4 layout, not offset-2.
        let script: Vec<u16> = "a+b".encode_utf16().collect();
        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, vec![0u8; 6]),
            make_eqedit_record(3, &script),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].controls.len(), 1);
        assert!(
            matches!(&paras[0].controls[0], HwpControl::Equation { script } if script == "a+b"),
            "expected Equation{{script:\"a+b\"}}, got {:?}",
            paras[0].controls[0]
        );
    }
    // -----------------------------------------------------------------------
    // HWPTAG_EQEDIT adversarial / red-team cases (G001)
    // -----------------------------------------------------------------------

    /// (a) Record shorter than 6 bytes: 4-byte attribute only, no length
    /// prefix. The offset-4 read must yield an empty script → no control,
    /// and must not panic on the truncated buffer.
    #[test]
    fn eqedit_adversarial_record_shorter_than_six_bytes() {
        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, vec![0u8; 6]),
            make_record_with_data_plain(HWPTAG_EQEDIT, 3, vec![0u8; 4]),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert!(
            paras[0].controls.is_empty(),
            "attribute-only EQEDIT must yield no control, got {:?}",
            paras[0].controls
        );
    }

    /// (b) Length prefix far larger than the remaining data. `read_utf16le`
    /// must clamp at the buffer end (no over-read / panic); only the WCHARs
    /// actually present become script.
    #[test]
    fn eqedit_adversarial_len_prefix_larger_than_data() {
        // [UINT32 attr][UINT16 len = 0x7FFF][single WCHAR 'X']
        let mut data = vec![0u8; 4];
        data.extend_from_slice(&0x7FFFu16.to_le_bytes());
        data.extend_from_slice(&u16::from(b'X').to_le_bytes());
        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, vec![0u8; 6]),
            make_record_with_data_plain(HWPTAG_EQEDIT, 3, data),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].controls.len(), 1);
        assert!(
            matches!(&paras[0].controls[0], HwpControl::Equation { script } if script == "X"),
            "over-claimed len must clamp to present data, got {:?}",
            paras[0].controls[0]
        );
    }

    /// (c) Non-empty script mixing embedded control chars with real chars. The
    /// dispatcher keeps it (renderable chars present); the IR conversion must
    /// strip every control char so none survives into the Math tex.
    #[test]
    fn eqedit_adversarial_mixed_control_chars_stripped_in_tex() {
        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, vec![0u8; 6]),
            make_eqedit_record(3, &[0x0041, 0x0000, 0x0042]), // "A\0B"
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert_eq!(paras[0].controls.len(), 1);
        let doc_info = crate::hwp::model::DocInfo::default();
        let block = crate::hwp::convert::control_to_block(&paras[0].controls[0], &doc_info)
            .expect("mixed script should still produce a Math block");
        match block {
            crate::ir::Block::Math { tex, .. } => {
                assert!(
                    !tex.chars().any(|c| c.is_control()),
                    "no control char may survive into tex, got {tex:?}"
                );
                assert!(
                    tex.contains('A') && tex.contains('B'),
                    "real chars must be preserved, got {tex:?}"
                );
            }
            other => panic!("expected Math block, got {other:?}"),
        }
    }

    /// (d) A lone high-surrogate WCHAR (0xD800). `from_utf16_lossy` maps it to
    /// U+FFFD (not control, not whitespace), so a control may be produced, but
    /// no control byte reaches the tex and nothing panics.
    #[test]
    fn eqedit_adversarial_lone_high_surrogate() {
        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, vec![0u8; 6]),
            make_eqedit_record(3, &[0xD800]),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        let doc_info = crate::hwp::model::DocInfo::default();
        for ctrl in &paras[0].controls {
            if let Some(crate::ir::Block::Math { tex, .. }) =
                crate::hwp::convert::control_to_block(ctrl, &doc_info)
            {
                assert!(
                    !tex.chars().any(|c| c.is_control()),
                    "lone surrogate must not leak a control byte, got {tex:?}"
                );
            }
        }
    }

    /// (e) A DEL (0x7F) code unit alone. DEL is a control char, so the
    /// dispatcher guard drops the script entirely — no Equation control.
    #[test]
    fn eqedit_adversarial_del_only_script_dropped() {
        let records = vec![
            make_record_with_data_plain(HWPTAG_PARA_HEADER, 2, vec![0u8; 6]),
            make_eqedit_record(3, &[0x007F]),
        ];
        let paras = extract_paragraphs_from_range(&records, 0, records.len());
        assert_eq!(paras.len(), 1);
        assert!(
            paras[0].controls.is_empty(),
            "DEL-only EQEDIT script must be dropped, got {:?}",
            paras[0].controls
        );
    }
}
