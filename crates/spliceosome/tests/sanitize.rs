//! Behaviour of the sanitiser on real and hand-built images.

mod common;

use common::*;
use spliceosome::{Clean, Error, Format, Limits, Sanitizer, sanitize};

fn clean(bytes: &[u8]) -> Clean {
    sanitize(bytes, &Limits::default()).expect("a valid image")
}

fn refuse(bytes: &[u8]) -> Error {
    sanitize(bytes, &Limits::default()).expect_err("an invalid image")
}

fn assert_unreadable(bytes: &[u8]) {
    let error = refuse(bytes);
    assert!(
        matches!(error, Error::Truncated { .. } | Error::Malformed { .. }),
        "expected a truncated or malformed image, got {error:?}"
    );
}

// ------------------------------------------------------------ accepted

#[test]
fn accepts_each_format_and_keeps_its_type() {
    for (bytes, format, mime, extension, dimensions) in [
        (
            jpeg(640, 360),
            Format::Jpeg,
            "image/jpeg",
            "jpg",
            (640, 360),
        ),
        (
            png(640, 360, &[], None),
            Format::Png,
            "image/png",
            "png",
            (640, 360),
        ),
        (
            webp(640, 360, false, &[]),
            Format::WebP,
            "image/webp",
            "webp",
            (640, 360),
        ),
        (
            webp(640, 360, true, &[]),
            Format::WebP,
            "image/webp",
            "webp",
            (640, 360),
        ),
    ] {
        let clean = clean(&bytes);
        assert_eq!(clean.format(), format);
        assert_eq!(clean.mime(), mime);
        assert_eq!(clean.extension(), extension);
        assert_eq!(clean.dimensions(), dimensions);
        assert_eq!(Format::detect(clean.bytes()), Some(format));
    }
}

#[test]
fn real_images_without_metadata_come_back_unchanged() {
    for (bytes, dimensions) in [
        (PHOTO_JPG, (64, 48)),
        (PROGRESSIVE_JPG, (64, 48)),
        (PHOTO_PNG, (32, 24)),
        (PHOTO_WEBP, (32, 24)),
        (SMALL_WEBP, (16, 16)),
        (LOSSLESS_WEBP, (30, 20)),
    ] {
        let clean = clean(bytes);
        assert_eq!(clean.bytes(), bytes, "{} unchanged", clean.format());
        assert_eq!(clean.dimensions(), dimensions);
        assert_eq!(clean.orientation(), None);
    }
}

#[test]
fn a_clean_built_picture_comes_back_unchanged() {
    let png = png(40, 40, &[], None);
    assert_eq!(clean(&png).bytes(), png);
    let lossy = webp(40, 40, false, &[]);
    assert_eq!(clean(&lossy).bytes(), lossy);
    let lossless = webp(30, 20, true, &[]);
    assert_eq!(clean(&lossless).bytes(), lossless);
}

#[test]
fn sanitising_twice_changes_nothing() {
    for bytes in [
        jpeg_with(PHOTO_JPG, &[exif_segment("Merta-Sawmill", Some(6))]),
        png_with(PHOTO_PNG, &[(b"tEXt", b"Author\0Merta")]),
        webp(
            20,
            20,
            false,
            &[(b"ICCP", b"profile-data"), (b"EXIF", b"MM\0*gps")],
        ),
    ] {
        let once = clean(&bytes);
        assert_eq!(clean(once.bytes()), once);
    }
}

#[test]
fn the_api_hands_out_the_bytes() {
    let clean = clean(PHOTO_PNG);
    assert_eq!(clean.as_ref(), PHOTO_PNG);
    assert_eq!(Vec::from(clean.clone()), PHOTO_PNG);
    assert_eq!(clean.into_bytes(), PHOTO_PNG);
    let sanitizer = Sanitizer::default();
    assert_eq!(sanitizer.limits(), &Limits::DEFAULT);
    assert_eq!(sanitizer.sanitize(PHOTO_PNG).unwrap().bytes(), PHOTO_PNG);
}

// ------------------------------------------------------------ JPEG

#[test]
fn a_jpeg_loses_its_exif_but_keeps_the_orientation() {
    let upload = jpeg_with(PHOTO_JPG, &[exif_segment("Merta-Sawmill", Some(6))]);
    let clean = clean(&upload);
    let output = clean.bytes();

    assert!(!contains(output, b"Merta-Sawmill"));
    assert_eq!(count(output, b"Exif"), 1);
    assert_eq!(clean.orientation(), Some(6));
    // The orientation block is a valid EXIF block of its own, first in the
    // file, followed by the untouched picture.
    let expected_exif =
        b"\xFF\xE1\x00\x22Exif\0\0MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
    assert_eq!(&output[2..2 + expected_exif.len()], expected_exif);
    assert_eq!(&output[2 + expected_exif.len()..], &PHOTO_JPG[2..]);
    assert_eq!(jpeg_markers(output)[0], 0xE1);
}

#[test]
fn a_jpeg_standing_upright_needs_no_exif_at_all() {
    for orientation in [None, Some(1), Some(0), Some(9)] {
        let output = clean(&jpeg_with(
            PHOTO_JPG,
            &[exif_segment("Merta-Sawmill", orientation)],
        ))
        .into_bytes();
        assert!(!contains(&output, b"Exif"));
        assert_eq!(output, PHOTO_JPG);
    }
}

#[test]
fn a_jpeg_takes_the_first_orientation_it_finds() {
    let xmp = segment(
        0xE1,
        b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>Merta</x:xmpmeta>",
    );
    let upload = jpeg_with(
        PHOTO_JPG,
        &[
            xmp,
            exif_segment("first", None),
            exif_segment("a", Some(8)),
            exif_segment("b", Some(3)),
        ],
    );
    assert_eq!(clean(&upload).orientation(), Some(8));
}

#[test]
fn a_jpeg_loses_every_kind_of_metadata_and_keeps_its_colours() {
    let icc = segment(0xE2, b"ICC_PROFILE\0\x01\x01colour-profile");
    let adobe = segment(0xEE, b"Adobe\0\x64\0\0\0\0\x01");
    let upload = jpeg_with(
        PROGRESSIVE_JPG,
        &[
            exif_segment("Merta-Sawmill", None),
            segment(
                0xE1,
                b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>xmp-secret</x:xmpmeta>",
            ),
            segment(0xED, b"Photoshop 3.0\08BIM\x04\x04iptc-secret"),
            segment(0xFE, b"comment-secret"),
            segment(0xE3, b"app3-secret"),
            segment(0xEF, b"app15-secret"),
            icc.clone(),
            adobe.clone(),
        ],
    );
    let mut upload_with_junk = upload.clone();
    upload_with_junk.extend_from_slice(PAYLOAD);

    let output = clean(&upload_with_junk).into_bytes();
    for secret in [
        &b"Merta"[..],
        b"xmp-secret",
        b"iptc-secret",
        b"comment-secret",
        b"app3-secret",
        b"app15-secret",
        b"<script",
        b"<?php",
    ] {
        assert!(
            !contains(&output, secret),
            "{}",
            String::from_utf8_lossy(secret)
        );
    }
    // The colour segments stay, in front of the original picture (whose
    // JFIF segment is kept too).
    assert_eq!(
        output,
        [&b"\xFF\xD8"[..], &icc, &adobe, &PROGRESSIVE_JPG[2..]].concat()
    );
    let markers = jpeg_markers(&output);
    assert_eq!(count(&markers, &[0xDA]), 10, "all progressive scans kept");
}

#[test]
fn a_jpeg_keeps_restart_markers_and_stuffed_bytes_in_its_scan() {
    let scan_data = b"\x12\xFF\x00\x34\xFF\xD0\x56\xFF\xFF\xFF\xD1\x78";
    let upload = [
        &b"\xFF\xD8"[..],
        &segment(0xDD, &[0, 1]),
        &segment(0xC0, &[8, 0, 16, 0, 16, 1, 1, 0x11, 0]),
        &segment(0xDA, &[1, 1, 0, 0, 63, 0]),
        scan_data,
        b"\xFF\xD9trailing",
    ]
    .concat();
    let output = clean(&upload).into_bytes();
    assert_eq!(output, upload[..upload.len() - 8]);
    assert!(contains(&output, scan_data));
}

#[test]
fn a_jpeg_may_have_fill_bytes_before_a_marker() {
    let mut upload = PHOTO_JPG.to_vec();
    upload.splice(2..2, *b"\xFF\xFF");
    assert_eq!(clean(&upload).bytes(), PHOTO_JPG);
}

#[test]
fn a_jpegs_comments_are_dropped() {
    let output = clean(&jpeg_with(
        &jpeg(16, 16),
        &[segment(0xFE, b"secret-note!!")],
    ))
    .into_bytes();
    assert!(!contains(&output, b"secret-note"));
}

#[test]
fn broken_jpegs_are_refused() {
    let frame = segment(0xC0, &[8, 0, 16, 0, 16, 1, 1, 0x11, 0]);
    let scan = [
        segment(0xDA, &[1, 1, 0, 0, 63, 0]),
        b"\x12\x34\xFF\xD9".to_vec(),
    ]
    .concat();
    for (name, upload) in [
        (
            "no image data",
            [&b"\xFF\xD8"[..], &frame, b"\xFF\xD9"].concat(),
        ),
        ("scan before frame", [&b"\xFF\xD8"[..], &scan].concat()),
        (
            "restart outside a scan",
            [&b"\xFF\xD8"[..], &frame, b"\xFF\xD0", &scan].concat(),
        ),
        (
            "second start of image",
            [&b"\xFF\xD8"[..], b"\xFF\xD8", &frame, &scan].concat(),
        ),
        (
            "TEM marker",
            [&b"\xFF\xD8"[..], b"\xFF\x01", &frame, &scan].concat(),
        ),
        (
            "length below 2",
            [&b"\xFF\xD8"[..], b"\xFF\xE0\x00\x01", &frame, &scan].concat(),
        ),
        (
            "short frame",
            [&b"\xFF\xD8"[..], &segment(0xC0, &[8, 0, 16, 0]), &scan].concat(),
        ),
        (
            "zero height",
            [
                &b"\xFF\xD8"[..],
                &segment(0xC0, &[8, 0, 0, 0, 16, 1, 1, 0x11, 0]),
                &scan,
            ]
            .concat(),
        ),
        (
            "junk between segments",
            [&b"\xFF\xD8"[..], &frame, b"junk", &scan].concat(),
        ),
        ("no end of image", PHOTO_JPG[..PHOTO_JPG.len() - 2].to_vec()),
        (
            "scan never ends",
            [
                &b"\xFF\xD8"[..],
                &frame,
                &segment(0xDA, &[1, 1, 0, 0, 63, 0]),
                b"\x12\x34",
            ]
            .concat(),
        ),
    ] {
        let error = refuse(&upload);
        assert!(
            matches!(
                error,
                Error::Truncated {
                    format: Format::Jpeg
                } | Error::Malformed {
                    format: Format::Jpeg,
                    ..
                }
            ),
            "{name}: {error:?}"
        );
    }
    assert_eq!(
        refuse(&[&b"\xFF\xD8"[..], &frame, b"\xFF\xD9"].concat()),
        Error::Malformed {
            format: Format::Jpeg,
            reason: "no image data"
        }
    );
}

// ------------------------------------------------------------ PNG

#[test]
fn a_png_loses_its_text_and_exif_chunks_and_keeps_its_colour_information() {
    let upload = png(
        20,
        20,
        &[
            (b"tEXt", b"Author\0Merta"),
            (b"eXIf", b"MM\0*secret-gps"),
            (b"gAMA", &45_455_u32.to_be_bytes()),
        ],
        None,
    );
    let output = clean(&upload).into_bytes();
    assert!(!contains(&output, b"Merta"));
    assert!(!contains(&output, b"secret-gps"));
    assert!(contains(&output, b"gAMA"));
    assert_eq!(
        png_chunks(&output),
        [*b"IHDR", *b"gAMA", *b"IDAT", *b"IEND"]
    );
}

#[test]
fn a_real_png_loses_every_kind_of_metadata() {
    let kept: [(&[u8; 4], &[u8]); 4] = [
        (b"sRGB", &[0]),
        (b"cHRM", &[0; 32]),
        (b"iCCP", b"profile\0\0compressed-profile"),
        (b"sBIT", &[8, 8, 8]),
    ];
    let removed: [(&[u8; 4], &[u8]); 9] = [
        (b"tEXt", b"Comment\0text-secret"),
        (b"zTXt", b"Comment\0\0ztxt-secret"),
        (
            b"iTXt",
            b"XML:com.adobe.xmp\0\0\0\0\0<x:xmpmeta>itxt-secret</x:xmpmeta>",
        ),
        (b"eXIf", b"MM\0*exif-secret"),
        (b"tIME", &[7, 234, 11, 13, 12, 0, 0]),
        (b"pHYs", &[0, 0, 11, 19, 0, 0, 11, 19, 1]),
        (b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]),
        (b"fcTL", &[0; 26]),
        (b"prVt", b"private-secret"),
    ];
    let mut upload = png_with(PHOTO_PNG, &[&kept[..], &removed[..]].concat());
    upload.extend_from_slice(PAYLOAD);

    let output = clean(&upload).into_bytes();
    for secret in [
        &b"text-secret"[..],
        b"ztxt-secret",
        b"itxt-secret",
        b"exif-secret",
        b"private-secret",
        b"tIME",
        b"acTL",
        b"<script",
    ] {
        assert!(
            !contains(&output, secret),
            "{}",
            String::from_utf8_lossy(secret)
        );
    }
    assert_eq!(output, png_with(PHOTO_PNG, &kept));
    assert_eq!(
        png_chunks(&output),
        [
            *b"IHDR", *b"sRGB", *b"cHRM", *b"iCCP", *b"sBIT", *b"PLTE", *b"tRNS", *b"IDAT",
            *b"IEND"
        ]
    );
}

#[test]
fn an_apng_keeps_only_its_default_image() {
    let upload = png(
        8,
        8,
        &[(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]), (b"fcTL", &[0; 26])],
        None,
    );
    let mut with_frame = upload[..upload.len() - 12].to_vec();
    with_frame.extend_from_slice(&png_chunk(b"fdAT", &[0, 0, 0, 1, 1, 2, 3]));
    with_frame.extend_from_slice(&png_chunk(b"IEND", b""));
    assert_eq!(
        png_chunks(clean(&with_frame).bytes()),
        [*b"IHDR", *b"IDAT", *b"IEND"]
    );
}

#[test]
fn a_png_whose_checksums_do_not_match_is_refused() {
    let mut upload = png(20, 20, &[], None);
    let at = upload.len() - 20; // a byte of the picture data
    upload[at] ^= 0xFF;
    assert_eq!(
        refuse(&upload),
        Error::Malformed {
            format: Format::Png,
            reason: "chunk checksum mismatch"
        }
    );
}

#[test]
fn broken_pngs_are_refused() {
    let signature = b"\x89PNG\r\n\x1A\n";
    let ihdr = |width: u32, height: u32| {
        png_chunk(
            b"IHDR",
            &[
                &width.to_be_bytes()[..],
                &height.to_be_bytes(),
                &[8, 2, 0, 0, 0],
            ]
            .concat(),
        )
    };
    let idat = png_chunk(b"IDAT", &zlib_stored(&[0, 1, 2, 3]));
    let iend = png_chunk(b"IEND", b"");
    for (name, upload) in [
        (
            "bad signature",
            [&b"\x89PNG\r\n\x1A\0"[..], &ihdr(1, 1), &idat, &iend].concat(),
        ),
        (
            "not IHDR first",
            [
                &signature[..],
                &png_chunk(b"tEXt", b"a\0b"),
                &ihdr(1, 1),
                &idat,
                &iend,
            ]
            .concat(),
        ),
        (
            "short IHDR",
            [
                &signature[..],
                &png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1]),
                &idat,
                &iend,
            ]
            .concat(),
        ),
        (
            "zero width",
            [&signature[..], &ihdr(0, 1), &idat, &iend].concat(),
        ),
        ("no IDAT", [&signature[..], &ihdr(1, 1), &iend].concat()),
        ("no IEND", [&signature[..], &ihdr(1, 1), &idat].concat()),
        (
            "huge chunk length",
            [&signature[..], &ihdr(1, 1), b"\xFF\xFF\xFF\xFFIDAT"].concat(),
        ),
        (
            "a GIF behind a PNG signature",
            [&b"\x89PNG"[..], TINY_GIF].concat(),
        ),
    ] {
        let error = refuse(&upload);
        assert!(
            matches!(
                error,
                Error::Truncated {
                    format: Format::Png
                } | Error::Malformed {
                    format: Format::Png,
                    ..
                }
            ),
            "{name}: {error:?}"
        );
    }
}

// ------------------------------------------------------------ WebP

#[test]
fn a_webp_loses_its_exif_and_xmp_and_keeps_its_colour_profile() {
    let upload = webp(
        20,
        20,
        false,
        &[
            (b"ICCP", b"profile-data"),
            (b"EXIF", b"MM\0*secret-gps"),
            (b"XMP ", b"<x:xmpmeta>Merta</x:xmpmeta>"),
        ],
    );
    let output = clean(&upload).into_bytes();
    assert!(!contains(&output, b"secret-gps"));
    assert!(!contains(&output, b"xmpmeta"));
    assert!(contains(&output, b"profile-data"));
    assert_eq!(
        output[20], 0x20,
        "the header says: a colour profile, nothing else"
    );
    assert_eq!(webp_kinds(&output), [*b"VP8X", *b"ICCP", *b"VP8 "]);
}

#[test]
fn a_webp_with_alpha_loses_its_metadata() {
    // The real WebP with an alpha channel, extended with EXIF, XMP and an
    // unknown chunk, its header flags announcing them, and junk after it.
    let chunks = webp_chunks(PHOTO_WEBP);
    let mut header = chunks[0].1.clone();
    header[0] |= 0x08 | 0x04; // EXIF and XMP
    let upload = riff(&[
        (b"VP8X", &header[..]),
        (b"EXIF", b"Exif\0\0MM\0*exif-secret"),
        (&chunks[1].0, &chunks[1].1[..]),
        (b"XMP ", b"<x:xmpmeta>xmp-secret</x:xmpmeta>"),
        (b"ZZZZ", b"odd"),
        (&chunks[2].0, &chunks[2].1[..]),
    ]);
    let upload = [upload, PAYLOAD.to_vec()].concat();

    let output = clean(&upload).into_bytes();
    assert_eq!(output, PHOTO_WEBP);
    assert_eq!(webp_kinds(&output), [*b"VP8X", *b"ALPH", *b"VP8 "]);
}

#[test]
fn lossless_webps_work_too() {
    let upload = webp(30, 20, true, &[]);
    let clean = clean(&upload);
    assert_eq!(clean.bytes(), upload);
    assert_eq!(clean.dimensions(), (30, 20));
}

#[test]
fn a_webp_without_extras_drops_its_vp8x_header() {
    let output = clean(&webp(16, 16, false, &[(b"EXIF", b"MM\0*gps")])).into_bytes();
    assert_eq!(output, SMALL_WEBP);
}

#[test]
fn animated_webps_are_refused() {
    let upload = webp(20, 20, false, &[(b"ANIM", &[0; 6])]);
    assert_eq!(
        refuse(&upload),
        Error::Animated {
            format: Format::WebP
        }
    );
    let upload = webp(20, 20, false, &[(b"ANMF", &[0; 16])]);
    assert_eq!(
        refuse(&upload),
        Error::Animated {
            format: Format::WebP
        }
    );
}

#[test]
fn broken_webps_are_refused() {
    let picture = webp_chunks(SMALL_WEBP).remove(0).1;
    let mut overlong = SMALL_WEBP.to_vec();
    overlong[16] = 0xFF; // the VP8 chunk claims more than the container holds
    let mut container_too_long = SMALL_WEBP.to_vec();
    container_too_long[4] += 2;
    for (name, upload) in [
        ("no picture", riff(&[(b"EXIF", b"data")])),
        ("bad VP8 signature", riff(&[(b"VP8 ", &picture[..3])])),
        ("short VP8", riff(&[(b"VP8 ", &picture[..8])])),
        (
            "bad VP8L signature",
            riff(&[(b"VP8L", &[0x2E, 0, 0, 0, 0])]),
        ),
        ("short VP8L", riff(&[(b"VP8L", &[0x2F, 0, 0])])),
        (
            "zero width VP8",
            riff(&[(b"VP8 ", &[&picture[..6], &[0, 0, 16, 0]].concat())]),
        ),
        ("bad VP8X", riff(&[(b"VP8X", &[0; 9]), (b"VP8 ", &picture)])),
        ("chunk past the container", overlong),
        ("container past the file", container_too_long),
        (
            "chunk header cut off",
            [&SMALL_WEBP[..12], b"VP8 "].concat(),
        ),
    ] {
        let error = refuse(&upload);
        assert!(
            matches!(
                error,
                Error::Truncated {
                    format: Format::WebP
                } | Error::Malformed {
                    format: Format::WebP,
                    ..
                }
            ),
            "{name}: {error:?}"
        );
    }
}

// ------------------------------------------------------------ all formats

#[test]
fn a_payload_appended_to_any_format_is_dropped() {
    for image in [
        jpeg(400, 300),
        png(40, 30, &[], None),
        webp(400, 300, false, &[]),
        webp(400, 300, true, &[]),
    ] {
        let upload = [&image[..], PAYLOAD].concat();
        let output = clean(&upload).into_bytes();
        assert!(!contains(&output, b"<script"));
        assert!(!contains(&output, b"<?php"));
        assert_eq!(output, image);
    }
}

#[test]
fn scripts_svgs_and_other_formats_are_refused() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"><script>alert(1)</script></svg>"#;
    for upload in [
        &b"<?php system($_GET['c']); ?>"[..],
        b"<html><script>alert(1)</script></html>",
        svg,
        TINY_GIF,
        b"",
        b"\xFF\xD8",
        b"RIFF\0\0\0\0WAVE",
        b"RIFF\0\0\0\0WEB",
        b"\x89PN",
    ] {
        assert_eq!(
            refuse(upload),
            Error::UnsupportedFormat,
            "{}",
            String::from_utf8_lossy(upload)
        );
    }
}

#[test]
fn a_file_over_the_size_limit_is_refused() {
    let big = png(2000, 2000, &[], Some(vec![7; Limits::DEFAULT.max_bytes]));
    assert_eq!(
        refuse(&big),
        Error::TooLarge {
            size: big.len(),
            max: 2 * 1024 * 1024
        }
    );

    // The size is checked first, whatever the bytes are.
    let limits = Limits {
        max_bytes: 10,
        ..Limits::default()
    };
    assert_eq!(
        sanitize(&[0; 11], &limits),
        Err(Error::TooLarge { size: 11, max: 10 })
    );

    // Exactly at the limit is fine.
    let limits = Limits {
        max_bytes: PHOTO_PNG.len(),
        ..Limits::default()
    };
    assert!(sanitize(PHOTO_PNG, &limits).is_ok());
}

#[test]
fn a_decompression_bomb_is_refused_in_every_format() {
    for upload in [
        jpeg(8000, 8000),
        png(8000, 8000, &[], Some(zlib_stored(&[0; 64]))),
        webp(8000, 8000, false, &[]),
        webp(8000, 8000, true, &[]),
    ] {
        assert!(upload.len() < Limits::DEFAULT.max_bytes);
        assert_eq!(
            refuse(&upload),
            Error::TooManyPixels {
                width: 8000,
                height: 8000,
                max: 25_000_000
            },
            "{:?}",
            Format::detect(&upload)
        );
    }
}

#[test]
fn the_pixel_limit_is_inclusive_and_configurable() {
    assert!(sanitize(&jpeg(5000, 5000), &Limits::default()).is_ok());
    assert!(sanitize(&jpeg(5000, 5001), &Limits::default()).is_err());
    let limits = Limits {
        max_pixels: 100,
        ..Limits::default()
    };
    assert!(sanitize(&png(10, 10, &[], None), &limits).is_ok());
    assert_eq!(
        sanitize(&png(10, 11, &[], None), &limits),
        Err(Error::TooManyPixels {
            width: 10,
            height: 11,
            max: 100
        })
    );
}

#[test]
fn a_webp_canvas_larger_than_its_picture_counts_too() {
    let mut upload = webp(16, 16, false, &[(b"ICCP", b"profile")]);
    // VP8X canvas: 16384 x 16384.
    upload[24..30].copy_from_slice(&[0xFF, 0x3F, 0, 0xFF, 0x3F, 0]);
    assert_eq!(
        refuse(&upload),
        Error::TooManyPixels {
            width: 16384,
            height: 16384,
            max: 25_000_000
        }
    );
}

#[test]
fn a_truncated_or_corrupt_image_is_refused() {
    for image in [
        jpeg(300, 300),
        png(300, 300, &[], None),
        webp(300, 300, false, &[]),
        PHOTO_WEBP.to_vec(),
        PROGRESSIVE_JPG.to_vec(),
    ] {
        assert_unreadable(&image[..image.len() / 2]);
    }
    assert_unreadable(&[&b"\xFF\xD8\xFF\xE0"[..], &b"garbage".repeat(50)].concat());
}

#[test]
fn every_prefix_of_a_valid_image_is_refused_without_panicking() {
    for image in [
        PHOTO_JPG,
        PROGRESSIVE_JPG,
        PHOTO_PNG,
        PHOTO_WEBP,
        LOSSLESS_WEBP,
    ] {
        for length in 0..image.len() {
            assert!(
                sanitize(&image[..length], &Limits::default()).is_err(),
                "prefix of {length} bytes"
            );
        }
    }
}

#[test]
fn errors_read_well_in_logs() {
    assert_eq!(
        Error::TooLarge {
            size: 3_000_000,
            max: 2_097_152
        }
        .to_string(),
        "image is 3000000 bytes, over the limit of 2097152 bytes"
    );
    assert_eq!(
        Error::TooManyPixels {
            width: 8000,
            height: 8000,
            max: 25_000_000
        }
        .to_string(),
        "image is 8000x8000 pixels, over the limit of 25000000 pixels"
    );
    assert_eq!(
        Error::UnsupportedFormat.to_string(),
        "not a JPEG, PNG or WebP image"
    );
    assert_eq!(
        Error::Animated {
            format: Format::WebP
        }
        .to_string(),
        "animated WebP images are not accepted"
    );
    assert_eq!(
        Error::Truncated {
            format: Format::Png
        }
        .to_string(),
        "truncated PNG image"
    );
    assert_eq!(
        refuse(&[&PHOTO_JPG[..PHOTO_JPG.len() - 2], b"\xFF\xFE\0\x01"].concat()).to_string(),
        "malformed JPEG image: segment length below 2"
    );
}
