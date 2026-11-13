//! Cuts the junk out of uploaded images.
//!
//! `spliceosome` turns an untrusted upload (a profile picture, say) into a
//! safe JPEG, PNG or WebP file, in pure Rust and without decoding a single
//! pixel:
//!
//! - **The bytes decide the format**, not the file name or the content type
//!   the browser claims. Anything other than JPEG, PNG or WebP (GIF, SVG,
//!   HTML, scripts) is refused, and so are animated WebP files.
//! - **Limits are enforced before any work**: the file size, and the pixel
//!   count read from the image header, so a tiny file that would decode to
//!   gigabytes (a decompression bomb) is refused without being decoded.
//! - **The file is rebuilt from only the parts a viewer needs**: metadata
//!   (EXIF with the camera and GPS position, XMP, IPTC, comments, PNG text
//!   chunks, time stamps) and anything appended after the end of the image
//!   (a script hidden in a polyglot file, say) are left out. Colour
//!   information (ICC profiles, gamma, sRGB, transparency) is kept, and a
//!   JPEG keeps its EXIF orientation alone, so phone photos still stand the
//!   right way up.
//!
//! The compressed picture data is copied as it is, so the result looks
//! exactly like the upload. Malformed files are refused rather than
//! repaired, and no input makes the sanitiser panic or allocate more than a
//! small constant beyond the size of the input.
//!
//! # Example
//!
//! ```
//! use spliceosome::{Format, Limits};
//!
//! // A 1x1 PNG with a text chunk, followed by a hidden script.
//! let upload = [
//!     &b"\x89PNG\r\n\x1a\n"[..],
//!     b"\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01\x08\x02\0\0\0\x90wS\xde",
//!     b"\0\0\0\x0ctEXtAuthor\0Alice\x1et\xa6\xbf",
//!     b"\0\0\0\x0cIDATx\x9cc\xf8\xdf\xc0\0\0\x04\x01\x01\x80\xc5*\x18]",
//!     b"\0\0\0\0IEND\xaeB`\x82",
//!     b"<script>alert(1)</script>",
//! ]
//! .concat();
//!
//! let clean = spliceosome::sanitize(&upload, &Limits::default())?;
//!
//! assert_eq!(clean.format(), Format::Png);
//! assert_eq!(clean.mime(), "image/png");
//! assert_eq!(clean.dimensions(), (1, 1));
//! assert!(!clean.bytes().windows(5).any(|w| w == b"Alice"));
//! assert!(clean.bytes().ends_with(b"IEND\xaeB`\x82"));
//! # Ok::<(), spliceosome::Error>(())
//! ```
//!
//! # What is kept
//!
//! | Format | Kept | Left out |
//! |---|---|---|
//! | JPEG | quantization and Huffman tables, frame, scans, restart interval, APP0 (JFIF), APP2 (ICC profile), APP14 (Adobe); the EXIF orientation, rewritten as a minimal EXIF block | APP1 (EXIF, XMP), APP13 (IPTC), comments, every other segment, data after the end-of-image marker |
//! | PNG | `IHDR`, `PLTE`, `IDAT`, `IEND`, `tRNS`, `gAMA`, `cHRM`, `sRGB`, `iCCP`, `sBIT` | text and EXIF chunks, `tIME`, APNG animation (the default image stays), unknown chunks, data after `IEND` |
//! | WebP | `VP8 ` or `VP8L` picture, `ALPH`, `ICCP` (behind a fresh `VP8X` header) | `EXIF`, `XMP `, unknown chunks, data after the RIFF container; animated files are refused |
//!
//! # Serving the result
//!
//! A sanitised file is a much smaller risk than the upload, but serve it
//! with the content type from [`Clean::mime`], `X-Content-Type-Options:
//! nosniff` and, ideally, a restrictive `Content-Security-Policy`.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used
    )
)]

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;

mod crc32;
mod exif;
mod jpeg;
mod png;
mod reader;
mod webp;

use reader::Reader;

/// Sanitises `bytes` within `limits`.
///
/// This is a shorthand for [`Sanitizer::new(*limits).sanitize(bytes)`](Sanitizer::sanitize).
///
/// # Errors
///
/// See [`Sanitizer::sanitize`].
///
/// # Example
///
/// ```
/// let limits = spliceosome::Limits { max_bytes: 1024, max_pixels: 1_000_000 };
/// let error = spliceosome::sanitize(b"GIF89a...", &limits).unwrap_err();
/// assert_eq!(error, spliceosome::Error::UnsupportedFormat);
/// ```
pub fn sanitize(bytes: &[u8], limits: &Limits) -> Result<Clean, Error> {
    Sanitizer::new(*limits).sanitize(bytes)
}

/// The limits an upload must stay within.
///
/// The [default](Limits::DEFAULT) suits profile pictures: 2 MiB and 25
/// megapixels (about 5000 x 5000).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limits {
    /// The largest file accepted, in bytes.
    pub max_bytes: usize,
    /// The most pixels (width x height) accepted. The file itself may be
    /// tiny and still decode to hundreds of megabytes, so this protects
    /// whatever decodes the image later (a browser, a thumbnailer).
    pub max_pixels: u64,
}

impl Limits {
    /// 2 MiB and 25 megapixels.
    pub const DEFAULT: Self = Self {
        max_bytes: 2 * 1024 * 1024,
        max_pixels: 25_000_000,
    };

    /// Refuses an empty image or one with too many pixels.
    fn check_dimensions(&self, file: &Reader<'_>, width: u32, height: u32) -> Result<(), Error> {
        if width == 0 || height == 0 {
            return Err(file.malformed("zero width or height"));
        }
        if u64::from(width) * u64::from(height) > self.max_pixels {
            return Err(Error::TooManyPixels {
                width,
                height,
                max: self.max_pixels,
            });
        }
        Ok(())
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Sanitises uploads within a set of [`Limits`].
///
/// # Example
///
/// ```
/// use spliceosome::{Limits, Sanitizer};
///
/// let sanitizer = Sanitizer::new(Limits { max_bytes: 512 * 1024, ..Limits::default() });
/// assert_eq!(sanitizer.limits().max_pixels, 25_000_000);
/// assert!(sanitizer.sanitize(b"<svg onload=alert(1)>").is_err());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Sanitizer {
    limits: Limits,
}

impl Sanitizer {
    /// A sanitiser enforcing `limits`.
    #[must_use]
    pub const fn new(limits: Limits) -> Self {
        Self { limits }
    }

    /// The limits this sanitiser enforces.
    #[must_use]
    pub const fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Checks `bytes` and rebuilds them without metadata or trailing data.
    ///
    /// The checks run in this order: the file size, the format (from the
    /// leading magic bytes), then the format's own structure, during which
    /// the pixel count is checked as soon as the dimensions are known.
    ///
    /// # Errors
    ///
    /// - [`Error::TooLarge`] when `bytes` is longer than
    ///   [`Limits::max_bytes`],
    /// - [`Error::UnsupportedFormat`] when it is not a JPEG, PNG or WebP,
    /// - [`Error::TooManyPixels`] when the image has more than
    ///   [`Limits::max_pixels`],
    /// - [`Error::Animated`] for an animated WebP,
    /// - [`Error::Truncated`] or [`Error::Malformed`] when the file is
    ///   broken.
    pub fn sanitize(&self, bytes: &[u8]) -> Result<Clean, Error> {
        if bytes.len() > self.limits.max_bytes {
            return Err(Error::TooLarge {
                size: bytes.len(),
                max: self.limits.max_bytes,
            });
        }
        let format = Format::detect(bytes).ok_or(Error::UnsupportedFormat)?;
        let parsed = match format {
            Format::Jpeg => jpeg::clean(bytes, &self.limits),
            Format::Png => png::clean(bytes, &self.limits),
            Format::WebP => webp::clean(bytes, &self.limits),
        }?;
        Ok(Clean {
            format,
            width: parsed.width,
            height: parsed.height,
            orientation: parsed.orientation,
            bytes: parsed.bytes,
        })
    }
}

/// An image format the sanitiser accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// JPEG (JFIF or EXIF).
    Jpeg,
    /// PNG. An animated PNG is accepted as its still default image.
    Png,
    /// WebP, lossy or lossless, still images only.
    WebP,
}

impl Format {
    /// The format the leading bytes announce, if it is one of the accepted
    /// ones. Only the magic numbers are read; the rest of the file may still
    /// be broken.
    ///
    /// ```
    /// use spliceosome::Format;
    ///
    /// assert_eq!(Format::detect(b"\xFF\xD8\xFF\xE0"), Some(Format::Jpeg));
    /// assert_eq!(Format::detect(b"GIF89a"), None);
    /// ```
    #[must_use]
    pub fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\xFF\xD8\xFF") {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"\x89PNG") {
            Some(Self::Png)
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            Some(Self::WebP)
        } else {
            None
        }
    }

    /// The MIME type: `image/jpeg`, `image/png` or `image/webp`.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::WebP => "image/webp",
        }
    }

    /// The usual file extension, without a dot: `jpg`, `png` or `webp`.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::WebP => "webp",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::WebP => "WebP",
        })
    }
}

/// A sanitised image: its bytes, format and size.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Clean {
    format: Format,
    width: u32,
    height: u32,
    orientation: Option<u8>,
    bytes: Vec<u8>,
}

impl Clean {
    /// The image format, the same as the upload's.
    #[must_use]
    pub const fn format(&self) -> Format {
        self.format
    }

    /// The MIME type to serve the image with.
    #[must_use]
    pub const fn mime(&self) -> &'static str {
        self.format.mime()
    }

    /// The file extension to store the image with, without a dot.
    #[must_use]
    pub const fn extension(&self) -> &'static str {
        self.format.extension()
    }

    /// Width and height in pixels, as stored (before any EXIF rotation).
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The EXIF orientation (2 to 8) a JPEG kept, or `None` when the image
    /// is stored upright. Orientations 5 to 8 swap width and height on
    /// screen.
    #[must_use]
    pub const fn orientation(&self) -> Option<u8> {
        self.orientation
    }

    /// The sanitised file.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The sanitised file, without copying.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl AsRef<[u8]> for Clean {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl From<Clean> for Vec<u8> {
    fn from(clean: Clean) -> Self {
        clean.bytes
    }
}

/// Why an upload was refused.
///
/// The messages are meant for logs; map the variants to messages of your
/// own for the user. [`Error::Truncated`] and [`Error::Malformed`] both mean
/// "not a readable image".
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The file is larger than [`Limits::max_bytes`].
    #[error("image is {size} bytes, over the limit of {max} bytes")]
    TooLarge {
        /// The size of the upload in bytes.
        size: usize,
        /// The limit.
        max: usize,
    },
    /// The image has more pixels than [`Limits::max_pixels`].
    #[error("image is {width}x{height} pixels, over the limit of {max} pixels")]
    TooManyPixels {
        /// The width from the image header.
        width: u32,
        /// The height from the image header.
        height: u32,
        /// The limit.
        max: u64,
    },
    /// The file is not a JPEG, PNG or WebP (judged by its leading bytes).
    #[error("not a JPEG, PNG or WebP image")]
    UnsupportedFormat,
    /// The image is animated (an animated WebP).
    #[error("animated {format} images are not accepted")]
    Animated {
        /// The image format.
        format: Format,
    },
    /// The file ends before its structure does.
    #[error("truncated {format} image")]
    Truncated {
        /// The format the file announced.
        format: Format,
    },
    /// The file's structure is broken or not allowed.
    #[error("malformed {format} image: {reason}")]
    Malformed {
        /// The format the file announced.
        format: Format,
        /// What is wrong, for the logs.
        reason: &'static str,
    },
}

/// What a format module hands back.
struct Parsed {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    orientation: Option<u8>,
}

/// The README's examples, compiled and run as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
