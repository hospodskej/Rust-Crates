//! Test images, built from small real files (made with `ImageMagick` and
//! `libwebp`, in `tests/fixtures`) or from scratch, plus readers that check
//! the structure of a sanitised file.

#![allow(
    dead_code,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::trivially_copy_pass_by_ref
)]

pub const PHOTO_JPG: &[u8] = include_bytes!("../fixtures/photo.jpg");
pub const PROGRESSIVE_JPG: &[u8] = include_bytes!("../fixtures/progressive.jpg");
pub const PHOTO_PNG: &[u8] = include_bytes!("../fixtures/photo.png");
/// Lossy with an alpha channel: `VP8X`, `ALPH`, `VP8 `.
pub const PHOTO_WEBP: &[u8] = include_bytes!("../fixtures/photo.webp");
/// A plain lossy 16x16 WebP: a single `VP8 ` chunk.
pub const SMALL_WEBP: &[u8] = include_bytes!("../fixtures/small.webp");
pub const LOSSLESS_WEBP: &[u8] = include_bytes!("../fixtures/lossless.webp");
pub const TINY_GIF: &[u8] = include_bytes!("../fixtures/tiny.gif");

pub const PAYLOAD: &[u8] = b"<script>alert(document.cookie)</script><?php system($_GET['c']); ?>";

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

pub fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

// ---------------------------------------------------------------- JPEG

/// The real 64x48 JPEG with its frame header claiming `width` x `height`.
pub fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = PHOTO_JPG.to_vec();
    let frame = find(&bytes, b"\xFF\xC0");
    bytes[frame + 5..frame + 9]
        .copy_from_slice(&[height.to_be_bytes(), width.to_be_bytes()].concat());
    bytes
}

/// `segments` inserted right after the start-of-image marker.
pub fn jpeg_with(base: &[u8], segments: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = base[..2].to_vec();
    for segment in segments {
        bytes.extend_from_slice(segment);
    }
    bytes.extend_from_slice(&base[2..]);
    bytes
}

/// A JPEG segment with `marker` and `data`.
pub fn segment(marker: u8, data: &[u8]) -> Vec<u8> {
    let mut segment = vec![0xFF, marker];
    segment.extend_from_slice(&((data.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(data);
    segment
}

/// An APP1 EXIF block (big-endian TIFF): orientation, copyright and a GPS
/// position in IFD0, as a phone would write it.
pub fn exif_segment(copyright: &str, orientation: Option<u16>) -> Vec<u8> {
    let copyright = format!("{copyright}\0");
    let copyright_at = 8 + 2 + 3 * 12 + 4;
    let gps_at = copyright_at + copyright.len() as u32;
    let mut entries: Vec<(u16, u16, u32, [u8; 4])> = Vec::new();
    if let Some(orientation) = orientation {
        let [a, b] = orientation.to_be_bytes();
        entries.push((0x0112, 3, 1, [a, b, 0, 0]));
    }
    entries.push((
        0x8298,
        2,
        copyright.len() as u32,
        copyright_at.to_be_bytes(),
    ));
    entries.push((0x8825, 4, 1, gps_at.to_be_bytes()));

    let mut ifd = (entries.len() as u16).to_be_bytes().to_vec();
    for (tag, kind, count, value) in entries {
        ifd.extend_from_slice(&tag.to_be_bytes());
        ifd.extend_from_slice(&kind.to_be_bytes());
        ifd.extend_from_slice(&count.to_be_bytes());
        ifd.extend_from_slice(&value);
    }
    ifd.resize(2 + 3 * 12, 0);
    ifd.extend_from_slice(&0_u32.to_be_bytes());
    // GPS IFD: latitude reference "N".
    let mut gps = 1_u16.to_be_bytes().to_vec();
    gps.extend_from_slice(&[0, 1, 0, 2, 0, 0, 0, 2, b'N', 0, 0, 0, 0, 0, 0, 0]);

    let mut data = b"Exif\0\0MM\0*".to_vec();
    data.extend_from_slice(&8_u32.to_be_bytes());
    data.extend_from_slice(&ifd);
    data.extend_from_slice(copyright.as_bytes());
    data.extend_from_slice(&gps);
    segment(0xE1, &data)
}

/// The markers of a JPEG, checking that every segment fits, the scans end
/// at a marker and nothing follows the end-of-image marker.
pub fn jpeg_markers(bytes: &[u8]) -> Vec<u8> {
    assert_eq!(&bytes[..2], b"\xFF\xD8", "starts with SOI");
    let mut markers = Vec::new();
    let mut position = 2;
    loop {
        assert_eq!(bytes[position], 0xFF, "marker at {position}");
        let marker = bytes[position + 1];
        markers.push(marker);
        if marker == 0xD9 {
            assert_eq!(position + 2, bytes.len(), "nothing after EOI");
            return markers;
        }
        let length = usize::from(u16::from_be_bytes([
            bytes[position + 2],
            bytes[position + 3],
        ]));
        position += 2 + length;
        assert!(position <= bytes.len(), "segment fits");
        if marker == 0xDA {
            while !(bytes[position] == 0xFF
                && bytes[position + 1] != 0
                && bytes[position + 1] != 0xFF
                && !(0xD0..=0xD7).contains(&bytes[position + 1]))
            {
                position += 1;
            }
        }
    }
}

// ---------------------------------------------------------------- PNG

/// A real (uncompressed) orange PNG of `width` x `height`, with
/// `extra_chunks` between `IHDR` and `IDAT`. `idat` replaces the picture
/// data, so huge sizes need not be drawn.
pub fn png(
    width: u32,
    height: u32,
    extra_chunks: &[(&[u8; 4], &[u8])],
    idat: Option<Vec<u8>>,
) -> Vec<u8> {
    let idat = idat.unwrap_or_else(|| {
        let mut row = vec![0_u8];
        for _ in 0..width {
            row.extend_from_slice(&[200, 120, 40]);
        }
        zlib_stored(&row.repeat(height as usize))
    });
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);

    let mut bytes = b"\x89PNG\r\n\x1A\n".to_vec();
    bytes.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    for (kind, data) in extra_chunks {
        bytes.extend_from_slice(&png_chunk(kind, data));
    }
    bytes.extend_from_slice(&png_chunk(b"IDAT", &idat));
    bytes.extend_from_slice(&png_chunk(b"IEND", b""));
    bytes
}

/// `chunks` inserted right after `IHDR` of a PNG.
pub fn png_with(base: &[u8], chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let after_ihdr = 8 + 12 + 13;
    let mut bytes = base[..after_ihdr].to_vec();
    for (kind, data) in chunks {
        bytes.extend_from_slice(&png_chunk(kind, data));
    }
    bytes.extend_from_slice(&base[after_ihdr..]);
    bytes
}

pub fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(data);
    chunk.extend_from_slice(&crc32(&chunk[4..]).to_be_bytes());
    chunk
}

/// The chunk types of a PNG, checking every checksum and that nothing
/// follows `IEND`.
pub fn png_chunks(bytes: &[u8]) -> Vec<[u8; 4]> {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1A\n", "signature");
    let mut chunks = Vec::new();
    let mut position = 8;
    while position < bytes.len() {
        let length = u32::from_be_bytes(bytes[position..position + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = bytes[position + 4..position + 8].try_into().unwrap();
        let checked = &bytes[position + 4..position + 8 + length];
        let checksum = u32::from_be_bytes(
            bytes[position + 8 + length..position + 12 + length]
                .try_into()
                .unwrap(),
        );
        assert_eq!(
            crc32(checked),
            checksum,
            "checksum of {}",
            String::from_utf8_lossy(&kind)
        );
        chunks.push(kind);
        position += 12 + length;
    }
    assert_eq!(position, bytes.len(), "chunks fill the file");
    assert_eq!(chunks.last(), Some(b"IEND"), "ends with IEND");
    chunks
}

/// A zlib stream of stored (uncompressed) deflate blocks.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut stream = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(65_535).collect()
    };
    for (index, block) in blocks.iter().enumerate() {
        stream.push(u8::from(index + 1 == blocks.len()));
        let length = block.len() as u16;
        stream.extend_from_slice(&length.to_le_bytes());
        stream.extend_from_slice(&(!length).to_le_bytes());
        stream.extend_from_slice(block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    stream.extend_from_slice(&((b << 16) | a).to_be_bytes());
    stream
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

// ---------------------------------------------------------------- WebP

/// The real 16x16 lossy (or 30x20 lossless) WebP with its bitstream header
/// claiming `width` x `height`, and `extra_chunks` before the picture in an
/// extended (`VP8X`) file.
pub fn webp(
    width: u32,
    height: u32,
    lossless: bool,
    extra_chunks: &[(&[u8; 4], &[u8])],
) -> Vec<u8> {
    let source = if lossless { LOSSLESS_WEBP } else { SMALL_WEBP };
    let kind: [u8; 4] = source[12..16].try_into().unwrap();
    let length = u32::from_le_bytes(source[16..20].try_into().unwrap()) as usize;
    let mut picture = source[20..20 + length].to_vec();
    if lossless {
        let bits = u32::from_le_bytes(picture[1..5].try_into().unwrap()) & !((1 << 28) - 1);
        picture[1..5].copy_from_slice(&(bits | (width - 1) | ((height - 1) << 14)).to_le_bytes());
    } else {
        picture[6..8].copy_from_slice(&(width as u16).to_le_bytes());
        picture[8..10].copy_from_slice(&(height as u16).to_le_bytes());
    }
    let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    if !extra_chunks.is_empty() {
        let mut header = vec![0x2C, 0, 0, 0];
        header.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        header.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        chunks.push((b"VP8X", header));
        chunks.extend(
            extra_chunks
                .iter()
                .map(|(kind, data)| (*kind, data.to_vec())),
        );
    }
    chunks.push((&kind, picture));
    riff(
        &chunks
            .iter()
            .map(|(kind, data)| (*kind, data.as_slice()))
            .collect::<Vec<_>>(),
    )
}

/// A RIFF WEBP file of `chunks`.
pub fn riff(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (kind, data) in chunks {
        body.extend_from_slice(*kind);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        if data.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
    bytes.extend_from_slice(b"WEBP");
    bytes.extend_from_slice(&body);
    bytes
}

/// The chunks of a WebP, checking that the RIFF size matches the file.
pub fn webp_chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");
    let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    assert_eq!(size + 8, bytes.len(), "RIFF size covers exactly the file");
    let mut chunks = Vec::new();
    let mut position = 12;
    while position < bytes.len() {
        let kind: [u8; 4] = bytes[position..position + 4].try_into().unwrap();
        let length =
            u32::from_le_bytes(bytes[position + 4..position + 8].try_into().unwrap()) as usize;
        chunks.push((kind, bytes[position + 8..position + 8 + length].to_vec()));
        position += 8 + length + (length & 1);
    }
    assert_eq!(position, bytes.len(), "chunks fill the container");
    chunks
}

pub fn webp_kinds(bytes: &[u8]) -> Vec<[u8; 4]> {
    webp_chunks(bytes)
        .into_iter()
        .map(|(kind, _)| kind)
        .collect()
}

fn find(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("needle present")
}
