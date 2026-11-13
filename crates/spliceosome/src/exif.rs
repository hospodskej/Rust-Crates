//! The one piece of EXIF a sanitised JPEG keeps: its orientation (how a
//! phone photo has to be turned to stand upright). It is read from an APP1
//! segment and written back as a minimal EXIF block of its own.

use alloc::vec::Vec;

use crate::reader::array_at;

const HEADER: &[u8] = b"Exif\0\0";
const ORIENTATION_TAG: u16 = 0x0112;
/// The TIFF field type of a 16-bit unsigned integer.
const SHORT: u16 = 3;
/// The size of one IFD entry: tag, type, count and value.
const ENTRY: usize = 12;

/// The orientation (2 to 8) in an APP1 segment's data.
///
/// `None` when the segment is not EXIF, has no orientation, says 1 ("as
/// stored", so nothing needs to be kept) or cannot be read. Only the first
/// orientation entry of the first IFD counts.
pub(crate) fn orientation(data: &[u8]) -> Option<u8> {
    let tiff = data.strip_prefix(HEADER)?;
    let big_endian = match tiff.get(..2)? {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };
    let u16_at = |offset| {
        array_at(tiff, offset).map(|bytes| {
            if big_endian {
                u16::from_be_bytes(bytes)
            } else {
                u16::from_le_bytes(bytes)
            }
        })
    };
    let u32_at = |offset| {
        array_at(tiff, offset).map(|bytes| {
            if big_endian {
                u32::from_be_bytes(bytes)
            } else {
                u32::from_le_bytes(bytes)
            }
        })
    };

    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = u16_at(ifd)?;
    let entries = ifd.checked_add(2)?;
    for index in 0..usize::from(count) {
        // At most 65535 entries of 12 bytes: no overflow before checked_add.
        let entry = entries.checked_add(index * ENTRY)?;
        if tiff.len() < entry.checked_add(ENTRY)? {
            return None;
        }
        if u16_at(entry)? != ORIENTATION_TAG || u16_at(entry + 2)? != SHORT {
            continue;
        }
        let value = u16_at(entry + 8)?;
        return u8::try_from(value)
            .ok()
            .filter(|value| (2..=8).contains(value));
    }
    None
}

/// An APP1 segment (marker included) holding only `orientation`, or nothing
/// without one.
pub(crate) fn orientation_segment(orientation: Option<u8>) -> Vec<u8> {
    let Some(orientation) = orientation else {
        return Vec::new();
    };
    let mut tiff = Vec::with_capacity(26);
    tiff.extend_from_slice(b"MM\0*");
    tiff.extend_from_slice(&8_u32.to_be_bytes()); // the first IFD follows
    tiff.extend_from_slice(&1_u16.to_be_bytes()); // one entry
    tiff.extend_from_slice(&ORIENTATION_TAG.to_be_bytes());
    tiff.extend_from_slice(&SHORT.to_be_bytes());
    tiff.extend_from_slice(&1_u32.to_be_bytes()); // one value
    tiff.extend_from_slice(&u16::from(orientation).to_be_bytes());
    tiff.extend_from_slice(&[0, 0]); // padding of the 4-byte value field
    tiff.extend_from_slice(&0_u32.to_be_bytes()); // no further IFD

    let length = 2 + HEADER.len() + tiff.len();
    let mut segment = Vec::with_capacity(2 + length);
    segment.extend_from_slice(&[0xFF, 0xE1]);
    #[allow(clippy::cast_possible_truncation)] // 34 bytes
    segment.extend_from_slice(&(length as u16).to_be_bytes());
    segment.extend_from_slice(HEADER);
    segment.extend_from_slice(&tiff);
    segment
}

#[cfg(test)]
mod tests {
    use super::{orientation, orientation_segment};

    #[test]
    fn reads_back_what_it_writes() {
        for value in 2..=8 {
            let segment = orientation_segment(Some(value));
            assert_eq!(orientation(&segment[4..]), Some(value));
        }
        assert_eq!(orientation_segment(None), [] as [u8; 0]);
    }

    #[test]
    fn reads_little_endian_blocks() {
        let mut data = b"Exif\0\0II*\0\x08\0\0\0\x01\0".to_vec();
        data.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(orientation(&data), Some(6));
    }

    #[test]
    fn ignores_upright_unknown_and_broken_values() {
        for value in [0_u8, 1, 9] {
            let mut segment = orientation_segment(Some(2));
            segment[4 + 6 + 8 + 2 + 8 + 1] = value;
            assert_eq!(orientation(&segment[4..]), None, "value {value}");
        }
        assert_eq!(
            orientation(b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>"),
            None
        );
        assert_eq!(orientation(b"Exif\0\0XX*\0"), None);
        assert_eq!(orientation(b"Exif\0\0MM\0*\xFF\xFF\xFF\xFF"), None);
        assert_eq!(
            orientation(b"Exif\0\0MM\0*\0\0\0\x08\xFF\xFF\x01\x12"),
            None
        );
    }
}
