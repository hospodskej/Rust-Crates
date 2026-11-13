//! Shrinks photos in the browser before they are uploaded.
//!
//! An attenuator is the part that reduces a signal's strength; this one
//! reduces images. When someone picks a photo in a file input, [`Shrink`]
//! decodes it, scales it down to at most 1600 px on its longer side, re-encodes
//! it as WebP through a canvas and puts the smaller file back into the input,
//! so the form uploads a few hundred KB instead of a 5-10 MB phone photo. The
//! server needs no image library.
//!
//! The rules:
//!
//! - Only JPEG, PNG and WebP files are processed (`image/jpeg`, `image/png`,
//!   `image/webp`). Anything else (GIF, SVG, HEIC, PDF, ...) is left alone.
//! - The picture is drawn at [`target_size`]: the longer side at most
//!   [`max_side`](Shrink::max_side) (default 1600 px), aspect ratio kept,
//!   rounded to whole pixels, never enlarged.
//! - EXIF orientation is applied (`imageOrientation: "from-image"`), so a
//!   portrait phone photo stays upright after its metadata is dropped.
//! - It is encoded as WebP at [`quality`](Shrink::quality) 0.82. A browser that
//!   cannot write WebP (Safari hands back a PNG) gets JPEG at the
//!   same quality instead, named `.jpg`.
//! - The original is kept when it already fits and is at most 500 000 bytes
//!   ([`small_enough`](Shrink::small_enough)), or when the re-encoded file is
//!   not smaller than the original.
//! - The new file keeps the original's name with the extension swapped
//!   (`IMG_1234.JPG` becomes `IMG_1234.webp`) and its `lastModified`.
//! - A picture the browser cannot read is an [`Error`]; the input keeps the
//!   original file, so it is uploaded as it is.
//!
//! Decoding uses `createImageBitmap` where the browser has it and an
//! `<img>` with an object URL otherwise; drawing uses an `OffscreenCanvas`
//! where available and a `<canvas>` element otherwise. Object URLs are revoked
//! and bitmaps closed even when the future is dropped half way, and no Rust
//! closures are handed to JavaScript, so nothing leaks.
//!
//! # Example
//!
//! ```no_run
//! use attenuator::{Outcome, Shrink};
//! use web_sys::HtmlInputElement;
//!
//! async fn on_change(input: HtmlInputElement) {
//!     match Shrink::new().shrink_input(&input).await {
//!         Ok(Some(Outcome::Shrunk { width, height, .. })) => {
//!             input.set_title(&format!("Shrunk to {width}x{height} px"));
//!         }
//!         Ok(_) => {}       // nothing picked, or the original is kept
//!         Err(_error) => {} // the original is uploaded as it is
//!     }
//! }
//! ```
//!
//! The crate compiles on every target, so code using it builds and tests
//! natively, but the browser calls only work on `wasm32-unknown-unknown` in a
//! browser. [`target_size`] and the [`Shrink`] rules ([`Shrink::accepts`],
//! [`Shrink::plan`]) are plain Rust and work everywhere.
#![forbid(unsafe_code)]

mod browser;
mod size;

use std::fmt;

pub use size::target_size;
use wasm_bindgen::JsValue;
use web_sys::{File, HtmlInputElement};

/// The file types that are shrunk; every other file is left alone.
pub const PROCESSED_TYPES: [&str; 3] = ["image/jpeg", "image/png", "image/webp"];

/// What the picture is re-encoded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Format {
    /// `image/webp`. Browsers that cannot write WebP get JPEG instead.
    #[default]
    WebP,
    /// `image/jpeg`.
    Jpeg,
}

impl Format {
    /// The MIME type, e.g. `image/webp`.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::WebP => "image/webp",
            Self::Jpeg => "image/jpeg",
        }
    }

    /// The file name extension without the dot, e.g. `webp`.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::WebP => "webp",
            Self::Jpeg => "jpg",
        }
    }

    fn from_mime(mime: &str) -> Option<Self> {
        match mime {
            "image/webp" => Some(Self::WebP),
            "image/jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }
}

/// Settings for shrinking a picture. The defaults are 1600 px, quality 0.82,
/// WebP, and keeping already small files up to 500 000 bytes.
///
/// ```
/// use attenuator::{Format, Shrink};
///
/// let shrink = Shrink::new().max_side(1200).quality(0.8).format(Format::WebP);
/// assert_eq!(shrink.plan(4000, 3000, 6_000_000), Some((1200, 900)));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shrink {
    max_side: u32,
    quality: f64,
    format: Format,
    small_enough: u64,
}

impl Default for Shrink {
    fn default() -> Self {
        Self::new()
    }
}

impl Shrink {
    /// The default longer side in pixels.
    pub const DEFAULT_MAX_SIDE: u32 = 1600;
    /// The default encoder quality.
    pub const DEFAULT_QUALITY: f64 = 0.82;
    /// The default size in bytes up to which a picture that already fits is
    /// kept as it is.
    pub const DEFAULT_SMALL_ENOUGH: u64 = 500_000;

    /// The default settings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_side: Self::DEFAULT_MAX_SIDE,
            quality: Self::DEFAULT_QUALITY,
            format: Format::WebP,
            small_enough: Self::DEFAULT_SMALL_ENOUGH,
        }
    }

    /// The longest the longer side may be, in pixels (default 1600; 0 counts
    /// as 1).
    #[must_use]
    pub const fn max_side(mut self, pixels: u32) -> Self {
        self.max_side = if pixels == 0 { 1 } else { pixels };
        self
    }

    /// The encoder quality from 0.0 to 1.0 (default 0.82). Values outside are
    /// clamped; NaN means the default.
    #[must_use]
    pub const fn quality(mut self, quality: f64) -> Self {
        self.quality = if quality.is_nan() {
            Self::DEFAULT_QUALITY
        } else {
            quality.clamp(0.0, 1.0)
        };
        self
    }

    /// The format to encode as (default [`Format::WebP`]).
    #[must_use]
    pub const fn format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// A picture that already fits within [`max_side`](Self::max_side) and
    /// is at most this many bytes is kept as it is (default 500 000). With 0,
    /// every fitting picture is re-encoded (and still kept if that is not
    /// smaller).
    #[must_use]
    pub const fn small_enough(mut self, bytes: u64) -> Self {
        self.small_enough = bytes;
        self
    }

    /// Whether a file of this MIME type is processed (JPEG, PNG and WebP; see
    /// [`PROCESSED_TYPES`]).
    ///
    /// ```
    /// let shrink = attenuator::Shrink::new();
    /// assert!(shrink.accepts("image/jpeg"));
    /// assert!(!shrink.accepts("image/gif"));
    /// ```
    #[must_use]
    pub fn accepts(&self, mime: &str) -> bool {
        PROCESSED_TYPES.contains(&mime)
    }

    /// The dimensions a decoded `width` x `height` picture of `bytes` bytes is
    /// re-encoded at, or `None` when it is kept as it is because it already
    /// fits and is small enough.
    ///
    /// ```
    /// let shrink = attenuator::Shrink::new();
    /// assert_eq!(shrink.plan(4032, 3024, 3_000_000), Some((1600, 1200)));
    /// assert_eq!(shrink.plan(1200, 900, 2_000_000), Some((1200, 900)));
    /// assert_eq!(shrink.plan(1200, 900, 400_000), None);
    /// ```
    #[must_use]
    pub fn plan(&self, width: u32, height: u32, bytes: u64) -> Option<(u32, u32)> {
        let target = target_size(width, height, self.max_side);
        if target == (width, height) && bytes <= self.small_enough {
            None
        } else {
            Some(target)
        }
    }

    /// Shrinks one file.
    ///
    /// Returns [`Outcome::Shrunk`] with the new, smaller file, or
    /// [`Outcome::Unchanged`] with the original and the reason it was kept.
    ///
    /// # Errors
    ///
    /// [`Error::Decode`] when the browser cannot read the picture,
    /// [`Error::Encode`] when it cannot encode the canvas, and
    /// [`Error::Browser`] when a browser API is missing or fails (for example
    /// outside a browser). In every case the original is fine to upload.
    ///
    /// ```no_run
    /// # async fn run(file: web_sys::File) -> Result<(), attenuator::Error> {
    /// let small = attenuator::Shrink::new().shrink(file).await?.into_file();
    /// # Ok(()) }
    /// ```
    pub async fn shrink(&self, file: File) -> Result<Outcome, Error> {
        if !self.accepts(&file.type_()) {
            return Ok(Outcome::Unchanged {
                file,
                reason: Reason::UnsupportedType,
            });
        }
        let image = browser::Image::decode(&file).await?;
        let (width, height) = image.dimensions();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bytes = file.size() as u64;
        let Some((new_width, new_height)) = self.plan(width, height, bytes) else {
            return Ok(Outcome::Unchanged {
                file,
                reason: Reason::SmallEnough { width, height },
            });
        };

        let canvas = browser::Canvas::new(new_width, new_height)?;
        canvas.draw(&image)?;
        drop(image);

        let mut blob = canvas.encode(self.format.mime(), self.quality).await;
        if self.format == Format::WebP
            && blob
                .as_ref()
                .is_none_or(|blob| blob.type_() != "image/webp")
        {
            // Safari cannot write WebP and hands back a PNG.
            blob = canvas.encode(Format::Jpeg.mime(), self.quality).await;
        }
        let Some(blob) = blob else {
            return Err(Error::Encode);
        };
        let Some(format) = Format::from_mime(&blob.type_()) else {
            return Err(Error::Encode);
        };
        if blob.size() >= file.size() {
            return Ok(Outcome::Unchanged {
                file,
                reason: Reason::NotSmaller {
                    width: new_width,
                    height: new_height,
                },
            });
        }

        let name = size::renamed(&file.name(), format.extension());
        let smaller = browser::file_from_blob(&blob, &name, format.mime(), file.last_modified())?;
        Ok(Outcome::Shrunk {
            file: smaller,
            width: new_width,
            height: new_height,
            format,
        })
    }

    /// Shrinks the file picked in `input` and puts the smaller one back into
    /// the input (through a `DataTransfer`), so submitting the form uploads
    /// it. Call it from the input's `change` event.
    ///
    /// Only the first file is shrunk; any further files of a `multiple` input
    /// stay as they are. Returns `Ok(None)` when no file is picked, or when
    /// the input's file changed while shrinking (the newer pick is left
    /// alone). Setting the files does not fire another `change` event.
    ///
    /// Shrinking takes a moment; a form submitted meanwhile uploads the
    /// original. To avoid that, disable the submit button until the returned
    /// future is done, or prevent the submit and call `requestSubmit()` after.
    ///
    /// # Errors
    ///
    /// The errors of [`shrink`](Self::shrink), and [`Error::Browser`] when the
    /// files cannot be replaced. The input then keeps the original file.
    ///
    /// ```no_run
    /// # async fn run(input: web_sys::HtmlInputElement) {
    /// // Whatever happens, the input holds a file that is fine to upload.
    /// let _ = attenuator::Shrink::new().shrink_input(&input).await;
    /// # }
    /// ```
    pub async fn shrink_input(&self, input: &HtmlInputElement) -> Result<Option<Outcome>, Error> {
        let Some(files) = input.files() else {
            return Ok(None);
        };
        let Some(original) = files.get(0) else {
            return Ok(None);
        };
        let outcome = self.shrink(original.clone()).await?;
        if let Outcome::Shrunk { file, .. } = &outcome {
            let current = input.files().and_then(|files| files.get(0));
            if !current.is_some_and(|current| js_sys::Object::is(&current, &original)) {
                return Ok(None);
            }
            let others = (1..files.length()).filter_map(|index| files.get(index));
            browser::replace_files(input, std::iter::once(file.clone()).chain(others))?;
        }
        Ok(Some(outcome))
    }
}

/// What [`Shrink::shrink`] did.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// The picture was re-encoded and the new file is smaller.
    Shrunk {
        /// The new file, e.g. `IMG_1234.webp`.
        file: File,
        /// Its width in pixels.
        width: u32,
        /// Its height in pixels.
        height: u32,
        /// What it was encoded as (JPEG when WebP was asked for but the
        /// browser cannot write it).
        format: Format,
    },
    /// The original is kept.
    Unchanged {
        /// The original file.
        file: File,
        /// Why it is kept.
        reason: Reason,
    },
}

impl Outcome {
    /// The file to upload: the new one or the original.
    #[must_use]
    pub fn file(&self) -> &File {
        match self {
            Self::Shrunk { file, .. } | Self::Unchanged { file, .. } => file,
        }
    }

    /// The file to upload, by value.
    #[must_use]
    pub fn into_file(self) -> File {
        match self {
            Self::Shrunk { file, .. } | Self::Unchanged { file, .. } => file,
        }
    }

    /// Whether a new, smaller file was made.
    #[must_use]
    pub const fn is_shrunk(&self) -> bool {
        matches!(self, Self::Shrunk { .. })
    }

    /// The size in bytes of [`file`](Self::file).
    #[must_use]
    pub fn size(&self) -> f64 {
        self.file().size()
    }

    /// The width and height in pixels of the shrunk file, or of the decoded
    /// original when it was kept (`None` when it was not decoded).
    #[must_use]
    pub const fn dimensions(&self) -> Option<(u32, u32)> {
        match *self {
            Self::Shrunk { width, height, .. }
            | Self::Unchanged {
                reason: Reason::SmallEnough { width, height },
                ..
            } => Some((width, height)),
            Self::Unchanged { .. } => None,
        }
    }
}

/// Why [`Outcome::Unchanged`] kept the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Reason {
    /// Not a JPEG, PNG or WebP file, so it was not touched.
    UnsupportedType,
    /// It already fits and is at most [`Shrink::small_enough`] bytes.
    SmallEnough {
        /// The original's width in pixels.
        width: u32,
        /// The original's height in pixels.
        height: u32,
    },
    /// The re-encoded file at this size was not smaller than the original.
    NotSmaller {
        /// The width it was re-encoded at.
        width: u32,
        /// The height it was re-encoded at.
        height: u32,
    },
}

/// Why shrinking failed. The original file is fine to upload in every case.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The browser cannot read the picture (damaged, or a format it cannot
    /// decode). Holds the browser's message.
    Decode(String),
    /// The browser cannot encode the canvas (for example, it is too large).
    Encode,
    /// A browser API is missing or failed, e.g. when not running in a
    /// browser. Holds the browser's message.
    Browser(String),
}

impl Error {
    fn browser(value: &JsValue) -> Self {
        Self::Browser(browser::message(value))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(message) => write!(f, "the browser cannot read the picture: {message}"),
            Self::Encode => f.write_str("the browser cannot encode the picture"),
            Self::Browser(message) => write!(f, "browser API failed: {message}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_javascript_values() {
        let shrink = Shrink::default();
        assert_eq!(shrink.max_side, 1600);
        assert!((shrink.quality - 0.82).abs() < f64::EPSILON);
        assert_eq!(shrink.format, Format::WebP);
        assert_eq!(shrink.small_enough, 500_000);
    }

    #[test]
    fn processes_jpeg_png_and_webp_only() {
        let shrink = Shrink::new();
        for mime in ["image/jpeg", "image/png", "image/webp"] {
            assert!(shrink.accepts(mime), "{mime}");
        }
        for mime in [
            "image/gif",
            "image/svg+xml",
            "image/heic",
            "image/avif",
            "application/pdf",
            "",
        ] {
            assert!(!shrink.accepts(mime), "{mime}");
        }
    }

    #[test]
    fn keeps_a_fitting_picture_up_to_the_small_enough_size() {
        let shrink = Shrink::new();
        assert_eq!(shrink.plan(1600, 1200, 500_000), None);
        assert_eq!(shrink.plan(1600, 1200, 500_001), Some((1600, 1200)));
        assert_eq!(shrink.plan(1601, 1200, 1000), Some((1600, 1199)));
        assert_eq!(
            Shrink::new().small_enough(0).plan(10, 10, 1),
            Some((10, 10))
        );
    }

    #[test]
    fn builder_clamps_nonsense() {
        let shrink = Shrink::new().max_side(0).quality(3.0);
        assert_eq!(shrink.max_side, 1);
        assert!((shrink.quality - 1.0).abs() < f64::EPSILON);
        assert!((Shrink::new().quality(f64::NAN).quality - 0.82).abs() < f64::EPSILON);
        assert!(Shrink::new().quality(-1.0).quality.abs() < f64::EPSILON);
    }

    #[test]
    fn formats() {
        assert_eq!(Format::WebP.mime(), "image/webp");
        assert_eq!(Format::Jpeg.extension(), "jpg");
        assert_eq!(Format::from_mime("image/png"), None);
    }

    #[test]
    fn errors_read_well() {
        assert_eq!(
            Error::Decode("bad".into()).to_string(),
            "the browser cannot read the picture: bad"
        );
    }
}
