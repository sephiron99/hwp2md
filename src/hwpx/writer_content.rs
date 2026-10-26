use crate::error::Hwp2MdError;
use crate::ir;
use std::collections::HashMap;
use std::fmt::Write as _;

use super::ImageAssetMap;

// ---------------------------------------------------------------------------
// Resolved asset type
// ---------------------------------------------------------------------------

/// A resolved image asset ready to be written into the HWPX ZIP.
#[derive(Debug)]
pub(super) struct ResolvedAsset {
    /// The bare filename used as the `BinData` entry (e.g. `"photo.png"`).
    pub(super) entry_name: String,
    /// Raw bytes of the image.
    pub(super) data: Vec<u8>,
    /// MIME type (e.g. `"image/png"`), written to the `content.hpf` manifest.
    pub(super) mime_type: String,
}

// ---------------------------------------------------------------------------
// Image source classification
// ---------------------------------------------------------------------------

/// Source classification for an image `src` string.
#[derive(Debug)]
pub(super) enum ImageSource<'a> {
    /// A local file path (not a URL, not a data URI).
    FilePath(&'a str),
    /// A `data:image/<subtype>;base64,<data>` URI.
    DataUri {
        /// The MIME sub-type, e.g. `"png"`.
        subtype: &'a str,
        /// The raw base64 payload (without the header prefix).
        payload: &'a str,
    },
    /// An HTTP or HTTPS URL — not embedded.
    RemoteUrl,
}

/// Classify a raw `src` string into one of the three [`ImageSource`] variants.
pub(super) fn classify_image_src(src: &str) -> ImageSource<'_> {
    if src.starts_with("http://") || src.starts_with("https://") {
        return ImageSource::RemoteUrl;
    }
    if let Some(rest) = src.strip_prefix("data:image/") {
        // Expected format: `data:image/<subtype>;base64,<payload>`
        if let Some((subtype_and_enc, payload)) = rest.split_once(',') {
            if let Some(subtype) = subtype_and_enc.strip_suffix(";base64") {
                return ImageSource::DataUri { subtype, payload };
            }
        }
    }
    ImageSource::FilePath(src)
}

/// Infer the MIME type for a file path based solely on its extension.
///
/// Falls back to `"application/octet-stream"` for unknown extensions.
pub(super) fn mime_from_extension(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// Image asset collection
// ---------------------------------------------------------------------------

/// Scan every `Block::Image` in `doc`, resolve file-path and data-URI sources,
/// and return:
///
/// 1. An [`ImageAssetMap`] mapping original `src` → bare `entry_name`.
/// 2. A `Vec<ResolvedAsset>` of all assets that must be written to `BinData/`.
///
/// HTTP/HTTPS URLs are skipped silently (they stay as-is in the `binaryItemIDRef`).
/// File paths that cannot be read are logged as warnings and omitted from the map
/// so that the caller can still produce a valid (if incomplete) HWPX.
///
/// Assets already present in `doc.assets` are also included so that documents
/// produced by the HWPX reader (which pre-populates `doc.assets`) continue to
/// roundtrip correctly.
pub(super) fn collect_image_assets(doc: &ir::Document) -> (ImageAssetMap, Vec<ResolvedAsset>) {
    let mut asset_map: ImageAssetMap = HashMap::new();
    let mut resolved: Vec<ResolvedAsset> = Vec::new();
    let mut counter: u32 = 0;

    // Pre-populate from doc.assets (already-resolved assets from an HWPX reader
    // roundtrip).  Use the asset's bare filename as the entry name.
    for asset in &doc.assets {
        let entry_name = std::path::Path::new(&asset.name).file_name().map_or_else(
            || {
                counter += 1;
                format!("asset_{counter}")
            },
            |n| n.to_string_lossy().into_owned(),
        );
        // Only register if not already mapped (src key == asset.name for
        // pre-existing assets coming from the reader).
        asset_map
            .entry(asset.name.clone())
            .or_insert_with(|| entry_name.clone());
        // Avoid duplicate entries in resolved.
        if !resolved.iter().any(|r| r.entry_name == entry_name) {
            resolved.push(ResolvedAsset {
                entry_name,
                data: asset.data.clone(),
                mime_type: asset.mime_type.clone(),
            });
        }
    }

    // Walk all blocks to find Image sources that were not already covered.
    for section in &doc.sections {
        collect_images_from_blocks(&section.blocks, &mut asset_map, &mut resolved, &mut counter);
    }

    (asset_map, resolved)
}

/// Return a unique entry name derived from `preferred` that is not already
/// present in `resolved`.
///
/// If `preferred` is already taken, inserts a numeric suffix before the
/// extension: `"photo.png"` → `"photo_2.png"` → `"photo_3.png"` etc.
fn unique_entry_name(preferred: &str, resolved: &[ResolvedAsset]) -> String {
    if !resolved.iter().any(|r| r.entry_name == preferred) {
        return preferred.to_owned();
    }
    // Split at the last `.` to get stem and extension.
    let (stem, ext) = match preferred.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (preferred, String::new()),
    };
    let mut n: u32 = 2;
    loop {
        let candidate = format!("{stem}_{n}{ext}");
        if !resolved.iter().any(|r| r.entry_name == candidate) {
            return candidate;
        }
        n += 1;
        // Safety valve: prevent unbounded iteration in pathological cases.
        // In normal usage this limit is never reached.
        if n > 10_000 {
            return candidate;
        }
    }
}

/// Recursively walk `blocks` and resolve any `Block::Image` whose `src` has
/// not already been added to `asset_map`.
pub(super) fn collect_images_from_blocks(
    blocks: &[ir::Block],
    asset_map: &mut ImageAssetMap,
    resolved: &mut Vec<ResolvedAsset>,
    counter: &mut u32,
) {
    for block in blocks {
        match block {
            ir::Block::Image { src, .. } => {
                if asset_map.contains_key(src.as_str()) {
                    continue;
                }
                match classify_image_src(src) {
                    ImageSource::RemoteUrl => {
                        // Leave remote URLs as-is; no embedding.
                    }
                    ImageSource::DataUri { subtype, payload } => match base64_decode(payload) {
                        Ok(bytes) => {
                            *counter += 1;
                            let entry_name = format!("image_{counter}.{subtype}");
                            let mime = format!("image/{subtype}");
                            asset_map.insert(src.clone(), entry_name.clone());
                            resolved.push(ResolvedAsset {
                                entry_name,
                                data: bytes,
                                mime_type: mime,
                            });
                        }
                        Err(e) => {
                            tracing::warn!("Failed to decode data URI for image: {e}");
                        }
                    },
                    ImageSource::FilePath(path) => {
                        match std::fs::read(path) {
                            Ok(bytes) => {
                                let bare = std::path::Path::new(path).file_name().map_or_else(
                                    || {
                                        *counter += 1;
                                        format!("image_{counter}")
                                    },
                                    |n| n.to_string_lossy().into_owned(),
                                );
                                let mime = mime_from_extension(path).to_owned();
                                // Deduplicate: if the bare filename is already
                                // occupied by a different asset, append a
                                // counter suffix (e.g. "photo_2.png").
                                let entry_name = unique_entry_name(&bare, resolved);
                                if entry_name != bare {
                                    tracing::warn!(
                                        "Image filename collision: {path:?} \
                                         renamed to {entry_name:?} to avoid \
                                         overwriting an earlier BinData entry"
                                    );
                                }
                                asset_map.insert(src.clone(), entry_name.clone());
                                resolved.push(ResolvedAsset {
                                    entry_name,
                                    data: bytes,
                                    mime_type: mime,
                                });
                            }
                            Err(e) => {
                                tracing::warn!("Cannot read image file {path:?}: {e}");
                            }
                        }
                    }
                }
            }
            ir::Block::Table { rows, .. } => {
                for row in rows {
                    for cell in &row.cells {
                        collect_images_from_blocks(&cell.blocks, asset_map, resolved, counter);
                    }
                }
            }
            ir::Block::BlockQuote { blocks: inner }
            | ir::Block::Footnote { content: inner, .. } => {
                collect_images_from_blocks(inner, asset_map, resolved, counter);
            }
            ir::Block::List { items, .. } => {
                for item in items {
                    collect_images_from_blocks(&item.blocks, asset_map, resolved, counter);
                }
            }
            ir::Block::Heading { .. }
            | ir::Block::Paragraph { .. }
            | ir::Block::CodeBlock { .. }
            | ir::Block::HorizontalRule
            | ir::Block::PageBreak
            | ir::Block::Math { .. } => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Base64 decoder
// ---------------------------------------------------------------------------

/// Minimal base64 decoder (standard alphabet, ignores whitespace).
///
/// No external crate is required — the `base64` crate is not in the dependency
/// tree.  The implementation handles the standard RFC 4648 alphabet and padding.
// Casting from 6-bit base64 indices to u8 always fits.
#[allow(clippy::cast_possible_truncation)]
// Sign loss is inherent: i8 indices cast to u32 for bitwise operations, lossy but safe after validation.
#[allow(clippy::cast_sign_loss)]
// Single-char names (a, b, c, d) are standard base64 nomenclature for quad processing.
#[allow(clippy::many_single_char_names)]
pub(super) fn base64_decode(input: &str) -> Result<Vec<u8>, Hwp2MdError> {
    const TABLE: [i8; 256] = {
        let mut t = [-1i8; 256];
        let mut i = 0usize;
        // A-Z
        while i < 26 {
            t[b'A' as usize + i] = i as i8;
            i += 1;
        }
        // a-z
        i = 0;
        while i < 26 {
            t[b'a' as usize + i] = (i + 26) as i8;
            i += 1;
        }
        // 0-9
        i = 0;
        while i < 10 {
            t[b'0' as usize + i] = (i + 52) as i8;
            i += 1;
        }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };

    let input = input.trim();
    // Strip trailing '=' padding for length calculation.
    let pad = input.bytes().rev().take(2).filter(|&b| b == b'=').count();
    let clean: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();

    if !clean.len().is_multiple_of(4) && !(clean.len() + pad).is_multiple_of(4) {
        // Allow length not divisible by 4 if padding was stripped.
    }

    let mut out = Vec::with_capacity(clean.len() * 3 / 4 + 2);
    let mut i = 0;
    while i + 3 < clean.len() {
        let [a, b, c, d] = [
            TABLE[clean[i] as usize],
            TABLE[clean[i + 1] as usize],
            TABLE[clean[i + 2] as usize],
            TABLE[clean[i + 3] as usize],
        ];
        if a < 0 || b < 0 || c < 0 || d < 0 {
            return Err(Hwp2MdError::HwpxWrite(
                "invalid base64 character in data URI".into(),
            ));
        }
        let n = (a as u32) << 18 | (b as u32) << 12 | (c as u32) << 6 | (d as u32);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
        i += 4;
    }
    // Handle remaining 2- or 3-byte groups (with stripped padding).
    let rem = clean.len() - i;
    if rem == 2 {
        let [a, b] = [TABLE[clean[i] as usize], TABLE[clean[i + 1] as usize]];
        if a < 0 || b < 0 {
            return Err(Hwp2MdError::HwpxWrite(
                "invalid base64 character in data URI".into(),
            ));
        }
        let n = (a as u32) << 18 | (b as u32) << 12;
        out.push((n >> 16) as u8);
    } else if rem == 3 {
        let [a, b, c] = [
            TABLE[clean[i] as usize],
            TABLE[clean[i + 1] as usize],
            TABLE[clean[i + 2] as usize],
        ];
        if a < 0 || b < 0 || c < 0 {
            return Err(Hwp2MdError::HwpxWrite(
                "invalid base64 character in data URI".into(),
            ));
        }
        let n = (a as u32) << 18 | (b as u32) << 12 | (c as u32) << 6;
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Static file generators
// ---------------------------------------------------------------------------

/// `META-INF/container.xml`: points at the package file, as Hancom writes it.
pub(super) fn generate_container_xml() -> String {
    concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>"#,
        r#"<ocf:container xmlns:ocf="urn:oasis:names:tc:opendocument:xmlns:container" "#,
        r#"xmlns:hpf="http://www.hancom.co.kr/schema/2011/hpf"><ocf:rootfiles>"#,
        r#"<ocf:rootfile full-path="Contents/content.hpf" media-type="application/hwpml-package+xml"/>"#,
        r#"</ocf:rootfiles></ocf:container>"#,
    )
    .to_string()
}

/// `version.xml`: the OWPML version the package follows (`HCFVersion`).
pub(super) fn generate_version_xml() -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>"#,
            r#"<hv:HCFVersion xmlns:hv="http://www.hancom.co.kr/hwpml/2011/version" "#,
            // `tagetApplication` is Hancom's spelling.
            r#"tagetApplication="WORDPROCESSOR" major="5" minor="1" micro="0" buildNumber="1" "#,
            r#"os="1" xmlVersion="1.4" application="hwp2md" appVersion="{}"/>"#,
        ),
        env!("CARGO_PKG_VERSION")
    )
}

/// `Contents/content.hpf`: the OPF package Hancom reads to find the header,
/// the sections and the embedded binaries, plus the title and author.
pub(super) fn generate_content_hpf(
    doc: &ir::Document,
    resolved_assets: &[ResolvedAsset],
) -> String {
    let non_empty = |v: &Option<String>| {
        v.as_deref()
            .filter(|t| !t.is_empty())
            .map(xml_escape_content)
    };
    let title = non_empty(&doc.metadata.title).map_or_else(
        || "<opf:title/>".to_string(),
        |t| format!("<opf:title>{t}</opf:title>"),
    );
    let creator = non_empty(&doc.metadata.author)
        .map(|a| format!(r#"<opf:meta name="creator" content="text">{a}</opf:meta>"#))
        .unwrap_or_default();

    let section_count = doc.sections.len().max(1);
    let mut manifest = String::from(
        r#"<opf:item id="header" href="Contents/header.xml" media-type="application/xml"/>"#,
    );
    // Each embedded binary: its id is the name section XML refers to.
    for asset in resolved_assets {
        let item_id = std::path::Path::new(&asset.entry_name)
            .file_stem()
            .map_or_else(
                || asset.entry_name.clone(),
                |s| s.to_string_lossy().into_owned(),
            );
        let _ = write!(
            manifest,
            r#"<opf:item id="{}" href="BinData/{}" media-type="{}" isEmbeded="1"/>"#,
            xml_escape_content(&item_id),
            xml_escape_content(&asset.entry_name),
            asset.mime_type,
        );
    }
    let mut spine = String::from(r#"<opf:itemref idref="header" linear="no"/>"#);
    for i in 0..section_count {
        let _ = write!(
            manifest,
            r#"<opf:item id="section{i}" href="Contents/section{i}.xml" media-type="application/xml"/>"#
        );
        let _ = write!(spine, r#"<opf:itemref idref="section{i}" linear="yes"/>"#);
    }

    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>"#,
            "\n",
            r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/" "#,
            r#"xmlns:hpf="http://www.hancom.co.kr/schema/2011/hpf" "#,
            r#"xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" version="" unique-identifier="" id="">"#,
            "<opf:metadata>{title}<opf:language>ko</opf:language>{creator}</opf:metadata>",
            "<opf:manifest>{manifest}</opf:manifest>",
            "<opf:spine>{spine}</opf:spine>",
            "</opf:package>",
        ),
        title = title,
        creator = creator,
        manifest = manifest,
        spine = spine,
    )
}

/// Minimal XML escaping for text nodes.
pub(super) fn xml_escape_content(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
