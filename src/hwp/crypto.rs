//! HWP distribution document decryption.
//!
//! HWP "배포용" (distribution/read-only) documents store body text in
//! `ViewText/Section{N}` streams instead of `BodyText/Section{N}`.  Each such
//! stream is laid out as:
//!
//! ```text
//! [record header: tag 0x1C (DISTRIBUTE_DOC_DATA), size 256]
//! [256-byte seed payload]
//! [AES-128 ECB encrypted section data ...]
//! ```
//!
//! The decryption pipeline is:
//!
//! 1. Read the `DISTRIBUTE_DOC_DATA` record at the start of the stream.
//! 2. Seed the MSVC LCG (`srand`) with the first 4 payload bytes and XOR the
//!    payload with run-length repeated `rand()` bytes.
//! 3. Take the 16-byte AES key at offset `4 + (seed & 0x0F)` of the result.
//! 4. AES-128 ECB decrypt the remainder of the stream.
//! 5. Deflate-decompress if the document is compressed (same as `BodyText`).
//!
//! # Algorithm sources
//! - HWP 5.0 file format specification (배포용 문서 데이터)
//! - pyhwp `hwp5.distdoc`

use crate::error::Hwp2MdError;
use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
use aes::Aes128;

/// Tag ID for the `DISTRIBUTE_DOC_DATA` record (`HWPTAG_BEGIN + 12`).
pub(crate) const HWPTAG_DISTRIBUTE_DOC_DATA: u16 = 0x001C;

/// Size of the `DISTRIBUTE_DOC_DATA` seed payload in bytes.
const SEED_LEN: usize = 256;

/// AES block size in bytes.
const AES_BLOCK_SIZE: usize = 16;

/// MSVC `rand()` Linear Congruential Generator.
///
/// ```text
/// seed = seed * 214013 + 2531011;
/// return (seed >> 16) & 0x7FFF;
/// ```
struct MsvcLcg {
    seed: u32,
}

impl MsvcLcg {
    fn new(seed: u32) -> Self {
        Self { seed }
    }

    fn rand(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.seed >> 16) & 0x7FFF
    }
}

/// Decode the 256-byte `DISTRIBUTE_DOC_DATA` payload.
///
/// Each XOR byte is `rand() & 0xFF` and is repeated for `(rand() & 0x0F) + 1`
/// consecutive payload bytes before a new pair is drawn.
///
/// # Errors
/// Returns an error if `payload` is not exactly 256 bytes.
pub(crate) fn decrypt_seed(payload: &[u8]) -> Result<Vec<u8>, Hwp2MdError> {
    if payload.len() != SEED_LEN {
        return Err(Hwp2MdError::HwpParse(format!(
            "DISTRIBUTE_DOC_DATA payload must be {SEED_LEN} bytes, got {}",
            payload.len(),
        )));
    }

    let seed = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let mut lcg = MsvcLcg::new(seed);

    let mut decoded = payload.to_vec();
    let mut key = 0u8;
    let mut run = 0u32;
    for byte in &mut decoded {
        if run == 0 {
            key = (lcg.rand() & 0xFF) as u8;
            run = (lcg.rand() & 0x0F) + 1;
        }
        *byte ^= key;
        run -= 1;
    }
    Ok(decoded)
}

/// Extract the 16-byte AES-128 key from a decoded seed.
///
/// The key starts at `4 + (seed & 0x0F)`, where `seed` is the little-endian
/// u32 formed by the first 4 bytes of the *original* (encoded) payload.
///
/// # Errors
/// Returns an error if the key would extend beyond the slice.
pub(crate) fn extract_aes_key(decoded: &[u8], seed: u32) -> Result<[u8; 16], Hwp2MdError> {
    let offset = 4 + (seed & 0x0F) as usize;
    let end = offset + AES_BLOCK_SIZE;
    if end > decoded.len() {
        return Err(Hwp2MdError::HwpParse(format!(
            "AES key would extend past seed: offset={offset}, seed_len={}",
            decoded.len(),
        )));
    }
    let mut key = [0u8; 16];
    key.copy_from_slice(&decoded[offset..end]);
    Ok(key)
}

/// Decrypt a raw `ViewText/Section{N}` stream.
///
/// Parses the leading `DISTRIBUTE_DOC_DATA` record, derives the AES key and
/// returns the AES-decrypted remainder (still compressed if the document is).
///
/// # Errors
/// Returns an error if the stream does not start with a 256-byte
/// `DISTRIBUTE_DOC_DATA` record.
pub(crate) fn decrypt_view_text_stream(raw: &[u8]) -> Result<Vec<u8>, Hwp2MdError> {
    const HEADER_LEN: usize = 4;
    if raw.len() < HEADER_LEN + SEED_LEN {
        return Err(Hwp2MdError::HwpParse(format!(
            "ViewText stream too short: {} bytes",
            raw.len()
        )));
    }
    let header = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let tag = (header & 0x3FF) as u16;
    let size = (header >> 20) as usize;
    if tag != HWPTAG_DISTRIBUTE_DOC_DATA || size != SEED_LEN {
        return Err(Hwp2MdError::HwpParse(format!(
            "ViewText stream does not start with DISTRIBUTE_DOC_DATA \
             (tag=0x{tag:X}, size={size})"
        )));
    }

    let payload = &raw[HEADER_LEN..HEADER_LEN + SEED_LEN];
    let seed = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let decoded = decrypt_seed(payload)?;
    let key = extract_aes_key(&decoded, seed)?;
    Ok(decrypt_viewtext(&raw[HEADER_LEN + SEED_LEN..], &key))
}

/// Decrypt data with AES-128 ECB.
///
/// Trailing bytes that do not form a complete 16-byte block are left as-is.
pub(crate) fn decrypt_viewtext(data: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut out = data.to_vec();
    for chunk in out.as_chunks_mut::<AES_BLOCK_SIZE>().0 {
        cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncrypt;

    #[test]
    fn msvc_lcg_seed_zero_known_sequence() {
        let mut lcg = MsvcLcg::new(0);
        assert_eq!(lcg.rand(), 38);
        assert_eq!(lcg.rand(), 7_719);
        assert_eq!(lcg.rand(), 21_238);
        assert_eq!(lcg.rand(), 2_437);
    }

    #[test]
    fn msvc_lcg_wraps_without_panic() {
        let mut lcg = MsvcLcg::new(u32::MAX);
        let _ = lcg.rand();
    }

    #[test]
    fn decrypt_seed_rejects_wrong_length() {
        assert!(decrypt_seed(&[0u8; 255]).is_err());
        assert!(decrypt_seed(&[0u8; 257]).is_err());
    }

    /// Seed 0: rand() = 38, 7719 → key byte 0x26, run = (7719 & 0xF) + 1 = 8.
    /// The first 8 bytes are XORed with 0x26, then a new pair is drawn.
    #[test]
    fn decrypt_seed_run_length_known_vector() {
        let out = decrypt_seed(&[0u8; SEED_LEN]).unwrap();
        assert!(out[..8].iter().all(|&b| b == 0x26), "{:x?}", &out[..9]);
        // Next pair: rand() = 21238 (0x52F6) → 0xF6, rand() = 2437 → run 6.
        assert!(out[8..14].iter().all(|&b| b == 0xF6), "{:x?}", &out[8..15]);
    }

    #[test]
    fn extract_aes_key_uses_original_seed_low_nibble() {
        let mut decoded = vec![0u8; SEED_LEN];
        decoded[9..25].copy_from_slice(&[0x55; 16]);
        // seed & 0xF = 5 → offset 9
        assert_eq!(extract_aes_key(&decoded, 0xABCD_EF05).unwrap(), [0x55; 16]);
    }

    #[test]
    fn decrypt_viewtext_partial_trailing_block_left_unmodified() {
        let key = [0u8; 16];
        let cipher = Aes128::new(GenericArray::from_slice(&key));
        let mut block = GenericArray::from([0x55u8; 16]);
        cipher.encrypt_block(&mut block);
        let mut input = block.to_vec();
        input.extend_from_slice(&[0xAA; 5]);

        let out = decrypt_viewtext(&input, &key);
        assert_eq!(&out[..16], &[0x55u8; 16]);
        assert_eq!(&out[16..], &[0xAAu8; 5]);
    }

    /// Build a synthetic ViewText stream and check the full pipeline.
    #[test]
    fn decrypt_view_text_stream_roundtrip() {
        // Arbitrary encoded payload; derive the key the same way a reader would.
        let payload: Vec<u8> = (0..SEED_LEN).map(|i| (i * 7 + 3) as u8).collect();
        let seed = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let key = extract_aes_key(&decrypt_seed(&payload).unwrap(), seed).unwrap();

        let plaintext = [0x42u8; 48];
        let cipher = Aes128::new(GenericArray::from_slice(&key));
        let mut body = plaintext.to_vec();
        for chunk in body.as_chunks_mut::<16>().0 {
            cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        }

        let header: u32 = u32::from(HWPTAG_DISTRIBUTE_DOC_DATA) | ((SEED_LEN as u32) << 20);
        let mut raw = header.to_le_bytes().to_vec();
        raw.extend_from_slice(&payload);
        raw.extend_from_slice(&body);

        assert_eq!(decrypt_view_text_stream(&raw).unwrap(), plaintext);
    }

    #[test]
    fn decrypt_view_text_stream_rejects_wrong_tag() {
        let header: u32 = 0x0042 | ((SEED_LEN as u32) << 20);
        let mut raw = header.to_le_bytes().to_vec();
        raw.extend_from_slice(&[0u8; SEED_LEN]);
        assert!(decrypt_view_text_stream(&raw).is_err());
    }
}
