//! Random and mutated input: the sanitiser must never panic, and whatever
//! it accepts must be stable and not much larger than the input.

mod common;

use common::*;
use proptest::prelude::*;
use spliceosome::{Limits, sanitize};

/// The most the rebuilt file can grow: a fresh EXIF orientation block for a
/// JPEG, or a fresh `VP8X` header for a WebP.
const MAX_GROWTH: usize = 64;

fn check(bytes: &[u8]) {
    let limits = Limits::default();
    if let Ok(clean) = sanitize(bytes, &limits) {
        assert!(clean.bytes().len() <= bytes.len() + MAX_GROWTH);
        let again = sanitize(clean.bytes(), &limits).expect("a sanitised image is valid");
        assert_eq!(again, clean, "sanitising is idempotent");
    }
}

fn seeds() -> Vec<Vec<u8>> {
    vec![
        PHOTO_JPG.to_vec(),
        PROGRESSIVE_JPG.to_vec(),
        jpeg_with(
            PHOTO_JPG,
            &[
                exif_segment("Merta", Some(6)),
                segment(0xE2, b"ICC_PROFILE\0\x01\x01icc"),
            ],
        ),
        PHOTO_PNG.to_vec(),
        png(
            4,
            3,
            &[(b"tEXt", b"a\0b"), (b"gAMA", &[0, 0, 177, 143])],
            None,
        ),
        PHOTO_WEBP.to_vec(),
        SMALL_WEBP.to_vec(),
        LOSSLESS_WEBP.to_vec(),
        webp(16, 16, false, &[(b"ICCP", b"icc"), (b"EXIF", b"MM\0*")]),
    ]
}

#[derive(Debug, Clone)]
enum Mutation {
    Set(usize, u8),
    Insert(usize, Vec<u8>),
    Remove(usize, usize),
    Truncate(usize),
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        4 => (any::<usize>(), any::<u8>()).prop_map(|(at, byte)| Mutation::Set(at, byte)),
        2 => (any::<usize>(), prop::collection::vec(any::<u8>(), 1..16)).prop_map(|(at, bytes)| Mutation::Insert(at, bytes)),
        1 => (any::<usize>(), 1..32_usize).prop_map(|(at, length)| Mutation::Remove(at, length)),
        1 => any::<usize>().prop_map(Mutation::Truncate),
    ]
}

fn apply(bytes: &mut Vec<u8>, mutation: &Mutation) {
    let len = bytes.len().max(1);
    match mutation {
        Mutation::Set(at, byte) => {
            if let Some(slot) = bytes.get_mut(at % len) {
                *slot = *byte;
            }
        }
        Mutation::Insert(at, inserted) => {
            let at = at % (bytes.len() + 1);
            bytes.splice(at..at, inserted.iter().copied());
        }
        Mutation::Remove(at, length) => {
            let at = at % len;
            let end = (at + length).min(bytes.len());
            bytes.drain(at.min(end)..end);
        }
        Mutation::Truncate(at) => bytes.truncate(at % len),
    }
}

proptest! {
    // 4096 cases per test; set PROPTEST_CASES for a longer run.
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(4096)
    ))]

    #[test]
    fn random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        check(&bytes);
    }

    #[test]
    fn random_bytes_behind_a_signature(
        signature in prop::sample::select(vec![
            &b"\xFF\xD8\xFF"[..],
            b"\x89PNG\r\n\x1A\n",
            b"\x89PNG\r\n\x1A\n\0\0\0\x0DIHDR",
            b"RIFF\xFF\xFF\xFF\xFFWEBP",
            b"RIFF\x40\0\0\0WEBPVP8X",
        ]),
        rest in prop::collection::vec(any::<u8>(), 0..512),
    ) {
        check(&[signature, &rest].concat());
    }

    #[test]
    fn mutated_images(
        seed in prop::sample::select(seeds()),
        mutations in prop::collection::vec(mutation(), 1..8),
    ) {
        let mut bytes = seed;
        for mutation in &mutations {
            apply(&mut bytes, mutation);
        }
        check(&bytes);
    }
}
