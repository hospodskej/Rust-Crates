//! PNG: rebuilt from its chunks.
//!
//! Kept: the chunks that draw the picture (`IHDR`, `PLTE`, `IDAT`, `IEND`)
//! and those that change how it looks (`tRNS`, `gAMA`, `cHRM`, `sRGB`,
//! `iCCP`, `sBIT`). Left out: text and EXIF chunks, the time stamp, the
//! animation of an APNG (its default image stays), anything unknown, and
//! anything after `IEND`. Every chunk's checksum must match.

use alloc::vec::Vec;

use crate::crc32::crc32;
use crate::reader::{Reader, array_at};
use crate::{Error, Format, Limits, Parsed};

const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1A\n";
const KEEP: [&[u8; 4]; 10] = [
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"sBIT",
];
/// Length, type and checksum around a chunk's data.
const OVERHEAD: usize = 12;

pub(crate) fn clean(bytes: &[u8], limits: &Limits) -> Result<Parsed, Error> {
    let file = Reader::new(bytes, Format::Png);
    if !bytes.starts_with(SIGNATURE) {
        return Err(file.malformed("bad signature"));
    }

    let mut output = Vec::with_capacity(bytes.len());
    output.extend_from_slice(SIGNATURE);
    let mut position = SIGNATURE.len();
    let mut dimensions = None;
    let mut has_data = false;
    loop {
        let length = file.length_be(position)?;
        let chunk = file.slice(position, length.saturating_add(OVERHEAD))?;
        // The chunk is `length + 12` bytes long, so these cannot fail.
        let (Some(kind), Some(checked), Some(checksum)) = (
            array_at::<4>(chunk, 4),
            chunk.get(4..length + 8),
            array_at(chunk, length + 8),
        ) else {
            return Err(file.truncated());
        };
        if crc32(checked) != u32::from_be_bytes(checksum) {
            return Err(file.malformed("chunk checksum mismatch"));
        }

        if dimensions.is_none() {
            let (b"IHDR", 13, Some(width), Some(height)) =
                (&kind, length, array_at(chunk, 8), array_at(chunk, 12))
            else {
                return Err(file.malformed("first chunk is not a valid IHDR"));
            };
            let (width, height) = (u32::from_be_bytes(width), u32::from_be_bytes(height));
            limits.check_dimensions(&file, width, height)?;
            dimensions = Some((width, height));
        }
        if KEEP.contains(&&kind) {
            output.extend_from_slice(chunk);
        }
        has_data |= &kind == b"IDAT";
        position += chunk.len();
        if &kind == b"IEND" {
            break;
        }
    }
    let (Some((width, height)), true) = (dimensions, has_data) else {
        return Err(file.malformed("no image data"));
    };

    Ok(Parsed {
        bytes: output,
        width,
        height,
        orientation: None,
    })
}
