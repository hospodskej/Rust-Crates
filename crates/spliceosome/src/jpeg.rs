//! JPEG: rebuilt from its segments.
//!
//! Kept: the tables and frame that make up the picture, the compressed scan
//! data, and APP0 (JFIF), APP2 (ICC colour profile) and APP14 (Adobe colour
//! transform), which change how it looks. Left out: APP1 (EXIF, XMP), APP13
//! (IPTC), comments, every other application segment, and anything after
//! the end-of-image marker. The EXIF orientation comes back as a minimal
//! EXIF block of its own.

use alloc::vec::Vec;

use crate::reader::{Reader, array_at};
use crate::{Error, Format, Limits, Parsed, exif};

const SOI: [u8; 2] = [0xFF, 0xD8];
const EOI: [u8; 2] = [0xFF, 0xD9];
const MARKER: u8 = 0xFF;
const END_OF_IMAGE: u8 = 0xD9;
const START_OF_IMAGE: u8 = 0xD8;
const START_OF_SCAN: u8 = 0xDA;
const TEMPORARY: u8 = 0x01;
const APP1: u8 = 0xE1;

/// Start of frame, in all its variants (C4, C8 and CC are other markers).
const fn is_frame(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

const fn is_restart(marker: u8) -> bool {
    matches!(marker, 0xD0..=0xD7)
}

/// Quantization and Huffman tables, arithmetic coding conditioning, restart
/// interval, and the APP0, APP2 and APP14 segments that change the colours.
const fn is_kept(marker: u8) -> bool {
    matches!(marker, 0xDB | 0xC4 | 0xCC | 0xDD | 0xE0 | 0xE2 | 0xEE)
}

pub(crate) fn clean(bytes: &[u8], limits: &Limits) -> Result<Parsed, Error> {
    let file = Reader::new(bytes, Format::Jpeg);
    if !bytes.starts_with(&SOI) {
        return Err(file.malformed("missing start-of-image marker"));
    }

    let mut kept = Vec::with_capacity(bytes.len());
    let mut orientation = None;
    let mut dimensions = None;
    let mut scanned = false;
    let mut position = SOI.len();
    loop {
        let marker;
        (marker, position) = next_marker(&file, position)?;
        if marker == END_OF_IMAGE {
            break;
        }
        if is_restart(marker) || marker == TEMPORARY || marker == START_OF_IMAGE {
            return Err(file.malformed("unexpected marker between segments"));
        }

        let length = usize::from(file.u16_be(position)?);
        if length < 2 {
            return Err(file.malformed("segment length below 2"));
        }
        // `position` is past a marker byte and its 0xFF, so at least 3.
        let segment = file.slice(position - 2, length + 2)?;
        let data = segment.get(4..).unwrap_or_default();
        position += length;

        if is_frame(marker) {
            // Sample precision, then height and width.
            let (Some(height), Some(width)) = (array_at(data, 1), array_at(data, 3)) else {
                return Err(file.malformed("frame header too short"));
            };
            let height = u32::from(u16::from_be_bytes(height));
            let width = u32::from(u16::from_be_bytes(width));
            limits.check_dimensions(&file, width, height)?;
            dimensions.get_or_insert((width, height));
            kept.extend_from_slice(segment);
        } else if marker == START_OF_SCAN {
            if dimensions.is_none() {
                return Err(file.malformed("scan before the frame header"));
            }
            let scan_end = end_of_scan(&file, position)?;
            kept.extend_from_slice(segment);
            kept.extend_from_slice(file.slice(position, scan_end - position)?);
            position = scan_end;
            scanned = true;
        } else if marker == APP1 {
            if orientation.is_none() {
                orientation = exif::orientation(data);
            }
        } else if is_kept(marker) {
            kept.extend_from_slice(segment);
        }
    }
    let (Some((width, height)), true) = (dimensions, scanned) else {
        return Err(file.malformed("no image data"));
    };

    let exif = exif::orientation_segment(orientation);
    let mut output = Vec::with_capacity(SOI.len() + exif.len() + kept.len() + EOI.len());
    output.extend_from_slice(&SOI);
    output.extend_from_slice(&exif);
    output.extend_from_slice(&kept);
    output.extend_from_slice(&EOI);
    Ok(Parsed {
        bytes: output,
        width,
        height,
        orientation,
    })
}

/// The marker at `position` (after any fill bytes) and where its segment
/// starts.
fn next_marker(file: &Reader<'_>, mut position: usize) -> Result<(u8, usize), Error> {
    if file.u8(position)? != MARKER {
        return Err(file.malformed("expected a marker"));
    }
    while file.u8(position)? == MARKER {
        position += 1;
    }
    Ok((file.u8(position)?, position + 1))
}

/// Where the compressed picture data after a start-of-scan ends: at the
/// first marker that is not a stuffed 0xFF byte, a fill byte or a restart
/// marker.
fn end_of_scan(file: &Reader<'_>, mut position: usize) -> Result<usize, Error> {
    loop {
        let found = file
            .rest(position)?
            .iter()
            .position(|&byte| byte == MARKER)
            .map(|offset| position + offset)
            .ok_or_else(|| file.truncated())?;
        let following = file.u8(found + 1)?;
        if following != 0 && following != MARKER && !is_restart(following) {
            return Ok(found);
        }
        position = found + if following == MARKER { 1 } else { 2 };
    }
}
