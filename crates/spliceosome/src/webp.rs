//! WebP: rebuilt from its RIFF chunks.
//!
//! Kept: the picture (`VP8 ` lossy or `VP8L` lossless), its alpha channel
//! (`ALPH`) and its colour profile (`ICCP`), behind a fresh `VP8X` header
//! when either of the last two is present. Left out: EXIF and XMP, anything
//! unknown, and anything after the RIFF container. Animated files are
//! refused.

use alloc::vec::Vec;

use crate::reader::{Reader, array_at};
use crate::{Error, Format, Limits, Parsed};

const LOSSY: [u8; 4] = *b"VP8 ";
const LOSSLESS: [u8; 4] = *b"VP8L";
const EXTENDED: [u8; 4] = *b"VP8X";
const ICC: [u8; 4] = *b"ICCP";
const ALPHA: [u8; 4] = *b"ALPH";
const ANIMATION: [[u8; 4]; 2] = [*b"ANIM", *b"ANMF"];
const ALPHA_FLAG: u8 = 0x10;
const ICC_FLAG: u8 = 0x20;
/// The RIFF header: `RIFF`, the container size, `WEBP`.
const HEADER: usize = 12;

/// The first chunk of each kind the sanitised file can use.
#[derive(Default)]
struct Chunks<'a> {
    picture: Option<([u8; 4], &'a [u8])>,
    extended: Option<&'a [u8]>,
    icc: Option<&'a [u8]>,
    alpha: Option<&'a [u8]>,
}

pub(crate) fn clean(bytes: &[u8], limits: &Limits) -> Result<Parsed, Error> {
    let file = Reader::new(bytes, Format::WebP);
    if file.slice(0, 4)? != b"RIFF" || file.slice(8, 4)? != b"WEBP" {
        return Err(file.malformed("not a RIFF WEBP container"));
    }

    let chunks = read_chunks(&file)?;
    let Some((kind, picture)) = chunks.picture else {
        return Err(file.malformed("no VP8 or VP8L image data"));
    };
    let (width, height) = dimensions(&file, kind, picture)?;
    limits.check_dimensions(&file, width, height)?;
    if let Some(header) = chunks.extended {
        let (canvas_width, canvas_height) = canvas_size(&file, header)?;
        limits.check_dimensions(&file, canvas_width, canvas_height)?;
    }

    let mut body = Vec::with_capacity(bytes.len());
    if chunks.icc.is_some() || chunks.alpha.is_some() {
        let mut flags = 0;
        if chunks.icc.is_some() {
            flags |= ICC_FLAG;
        }
        if chunks.alpha.is_some() {
            flags |= ALPHA_FLAG;
        }
        // Both are at least 1 (checked above) and at most 16384.
        let [w0, w1, w2] = le24(width - 1);
        let [h0, h1, h2] = le24(height - 1);
        write_chunk(
            &mut body,
            EXTENDED,
            &[flags, 0, 0, 0, w0, w1, w2, h0, h1, h2],
        );
    }
    if let Some(icc) = chunks.icc {
        write_chunk(&mut body, ICC, icc);
    }
    if let Some(alpha) = chunks.alpha {
        write_chunk(&mut body, ALPHA, alpha);
    }
    write_chunk(&mut body, kind, picture);

    let mut output = Vec::with_capacity(HEADER + body.len());
    output.extend_from_slice(b"RIFF");
    output.extend_from_slice(&riff_size(&file, body.len() + 4)?.to_le_bytes());
    output.extend_from_slice(b"WEBP");
    output.extend_from_slice(&body);
    Ok(Parsed {
        bytes: output,
        width,
        height,
        orientation: None,
    })
}

/// The chunks inside the RIFF container, which must lie within the file.
fn read_chunks<'a>(file: &Reader<'a>) -> Result<Chunks<'a>, Error> {
    let last = 8_usize.saturating_add(file.length_le(4)?);
    if last > file.len() {
        return Err(file.truncated());
    }
    let mut chunks = Chunks::default();
    let mut position = HEADER;
    while position < last {
        let kind: [u8; 4] = file
            .slice(position, 4)?
            .try_into()
            .map_err(|_| file.truncated())?;
        let length = file.length_le(position + 4)?;
        let end = (position + 8).saturating_add(length);
        if end > last {
            return Err(file.malformed("chunk runs past the end of the RIFF container"));
        }
        if ANIMATION.contains(&kind) {
            return Err(Error::Animated {
                format: Format::WebP,
            });
        }

        let data = file.slice(position + 8, length)?;
        match kind {
            LOSSY | LOSSLESS => {
                chunks.picture.get_or_insert((kind, data));
            }
            EXTENDED => {
                chunks.extended.get_or_insert(data);
            }
            ICC => {
                chunks.icc.get_or_insert(data);
            }
            ALPHA => {
                chunks.alpha.get_or_insert(data);
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        position = end + (length & 1);
    }
    Ok(chunks)
}

/// The picture's width and height, from its bitstream header.
fn dimensions(file: &Reader<'_>, kind: [u8; 4], data: &[u8]) -> Result<(u32, u32), Error> {
    if kind == LOSSLESS {
        let (Some(0x2F), Some(bits)) = (data.first(), array_at(data, 1)) else {
            return Err(file.malformed("bad VP8L header"));
        };
        let bits = u32::from_le_bytes(bits);
        Ok(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
    } else {
        let (Some(b"\x9D\x01\x2A"), Some(width), Some(height)) =
            (data.get(3..6), array_at(data, 6), array_at(data, 8))
        else {
            return Err(file.malformed("bad VP8 header"));
        };
        Ok((
            u32::from(u16::from_le_bytes(width) & 0x3FFF),
            u32::from(u16::from_le_bytes(height) & 0x3FFF),
        ))
    }
}

/// The canvas width and height in a `VP8X` header.
fn canvas_size(file: &Reader<'_>, header: &[u8]) -> Result<(u32, u32), Error> {
    let (10, Some([w0, w1, w2]), Some([h0, h1, h2])) =
        (header.len(), array_at(header, 4), array_at(header, 7))
    else {
        return Err(file.malformed("bad VP8X header"));
    };
    Ok((
        u32::from_le_bytes([w0, w1, w2, 0]) + 1,
        u32::from_le_bytes([h0, h1, h2, 0]) + 1,
    ))
}

fn le24(number: u32) -> [u8; 3] {
    let [a, b, c, _] = number.to_le_bytes();
    [a, b, c]
}

/// The RIFF size field. The rebuilt file is never larger than the original
/// plus a header, so this only fails on inputs beyond 4 GiB.
fn riff_size(file: &Reader<'_>, size: usize) -> Result<u32, Error> {
    u32::try_from(size).map_err(|_| file.malformed("file too large for RIFF"))
}

fn write_chunk(output: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    output.extend_from_slice(&kind);
    // Every chunk comes from a container whose size fit in 32 bits.
    output.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes());
    output.extend_from_slice(data);
    if data.len() % 2 == 1 {
        output.push(0);
    }
}
