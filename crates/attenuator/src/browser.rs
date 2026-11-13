//! The browser side: decoding, drawing and encoding through web-sys.
//!
//! No Rust closure is handed to JavaScript: every callback is the `resolve` or
//! `reject` function of a `Promise`, so there is nothing to leak or forget.
//! Object URLs and bitmaps are released by `Drop`, which also runs when the
//! future is dropped before it finishes.

use js_sys::{Array, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, CanvasRenderingContext2d, DataTransfer, Document, File, FilePropertyBag,
    HtmlCanvasElement, HtmlImageElement, HtmlInputElement, ImageBitmap, ImageBitmapOptions,
    ImageEncodeOptions, ImageOrientation, OffscreenCanvas, OffscreenCanvasRenderingContext2d, Url,
    Window, WorkerGlobalScope,
};

use crate::Error;

/// The text of a JavaScript error or value.
pub(crate) fn message(value: &JsValue) -> String {
    if let Some(error) = value.dyn_ref::<js_sys::Error>() {
        return String::from(error.message());
    }
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

fn has_global(name: &str) -> bool {
    Reflect::has(&js_sys::global(), &JsValue::from_str(name)).unwrap_or(false)
}

fn document() -> Result<Document, Error> {
    js_sys::global()
        .dyn_into::<Window>()
        .ok()
        .and_then(|window| window.document())
        .ok_or_else(|| Error::Browser("no document (not running in a page)".into()))
}

/// A decoded picture, upright (EXIF orientation applied).
pub(crate) enum Image {
    Bitmap(Bitmap),
    Element {
        image: HtmlImageElement,
        /// Revoked once the picture is drawn.
        _url: ObjectUrl,
    },
}

impl Image {
    /// Decodes with `createImageBitmap` where the browser has it, and with an
    /// `<img>` otherwise (or when `createImageBitmap` refuses, as older
    /// Safari does for some options).
    pub(crate) async fn decode(file: &File) -> Result<Self, Error> {
        let bitmap_error = if has_global("createImageBitmap") {
            match decode_bitmap(file).await {
                Ok(bitmap) => return Ok(Self::Bitmap(bitmap)),
                Err(error) => Some(error),
            }
        } else {
            None
        };
        match decode_element(file).await {
            Ok(image) => Ok(image),
            Err(error) => Err(bitmap_error.unwrap_or(error)),
        }
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Bitmap(bitmap) => (bitmap.0.width(), bitmap.0.height()),
            Self::Element { image, .. } => (image.natural_width(), image.natural_height()),
        }
    }
}

/// An `ImageBitmap` that is closed when dropped.
pub(crate) struct Bitmap(ImageBitmap);

impl Drop for Bitmap {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// An object URL that is revoked when dropped.
pub(crate) struct ObjectUrl(String);

impl ObjectUrl {
    fn new(blob: &Blob) -> Result<Self, Error> {
        Url::create_object_url_with_blob(blob)
            .map(Self)
            .map_err(|error| Error::browser(&error))
    }
}

impl Drop for ObjectUrl {
    fn drop(&mut self) {
        let _ = Url::revoke_object_url(&self.0);
    }
}

async fn decode_bitmap(file: &File) -> Result<Bitmap, Error> {
    let options = ImageBitmapOptions::new();
    options.set_image_orientation(ImageOrientation::FromImage);
    let global = js_sys::global();
    let promise = if let Some(window) = global.dyn_ref::<Window>() {
        window.create_image_bitmap_with_blob_and_image_bitmap_options(file, &options)
    } else if let Some(worker) = global.dyn_ref::<WorkerGlobalScope>() {
        worker.create_image_bitmap_with_blob_and_image_bitmap_options(file, &options)
    } else {
        return Err(Error::Browser("no window or worker scope".into()));
    }
    .map_err(|error| Error::Decode(message(&error)))?;
    let bitmap = JsFuture::from(promise)
        .await
        .map_err(|error| Error::Decode(message(&error)))?;
    bitmap
        .dyn_into::<ImageBitmap>()
        .map(Bitmap)
        .map_err(|value| Error::Decode(message(&value)))
}

async fn decode_element(file: &File) -> Result<Image, Error> {
    let document = document()?;
    let image = document
        .create_element("img")
        .map_err(|error| Error::browser(&error))?
        .unchecked_into::<HtmlImageElement>();
    let url = ObjectUrl::new(file)?;
    let loaded = Promise::new(&mut |resolve, reject| {
        image.set_onload(Some(&resolve));
        image.set_onerror(Some(&reject));
    });
    image.set_src(&url.0);
    let result = JsFuture::from(loaded).await;
    image.set_onload(None);
    image.set_onerror(None);
    match result {
        Ok(_) if image.natural_width() > 0 && image.natural_height() > 0 => {
            Ok(Image::Element { image, _url: url })
        }
        Ok(_) => Err(Error::Decode("the picture has no size".into())),
        Err(_) => Err(Error::Decode("the image element could not load it".into())),
    }
}

/// An `OffscreenCanvas` where available, a `<canvas>` element otherwise.
pub(crate) enum Canvas {
    Offscreen(OffscreenCanvas, OffscreenCanvasRenderingContext2d),
    Element(HtmlCanvasElement, CanvasRenderingContext2d),
}

impl Canvas {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, Error> {
        if has_global("OffscreenCanvas") {
            let offscreen = OffscreenCanvas::new(width, height).ok().and_then(|canvas| {
                let context = canvas.get_context("2d").ok()??;
                let context = context
                    .dyn_into::<OffscreenCanvasRenderingContext2d>()
                    .ok()?;
                Some(Self::Offscreen(canvas, context))
            });
            if let Some(offscreen) = offscreen {
                return Ok(offscreen);
            }
        }
        let canvas = document()?
            .create_element("canvas")
            .map_err(|error| Error::browser(&error))?
            .unchecked_into::<HtmlCanvasElement>();
        canvas.set_width(width);
        canvas.set_height(height);
        let context = canvas
            .get_context("2d")
            .map_err(|error| Error::browser(&error))?
            .ok_or_else(|| Error::Browser("no 2d canvas context".into()))?
            .unchecked_into::<CanvasRenderingContext2d>();
        Ok(Self::Element(canvas, context))
    }

    /// Draws `image` scaled to fill the canvas.
    pub(crate) fn draw(&self, image: &Image) -> Result<(), Error> {
        let (width, height) = self.size();
        let (width, height) = (f64::from(width), f64::from(height));
        let context: &JsValue = match self {
            Self::Offscreen(_, context) => context,
            Self::Element(_, context) => context,
        };
        // Smoother downscaling than the default "low"; ignored where unknown.
        let _ = Reflect::set(context, &"imageSmoothingQuality".into(), &"high".into());
        match (self, image) {
            (Self::Offscreen(_, c), Image::Bitmap(b)) => {
                c.draw_image_with_image_bitmap_and_dw_and_dh(&b.0, 0.0, 0.0, width, height)
            }
            (Self::Offscreen(_, c), Image::Element { image: i, .. }) => {
                c.draw_image_with_html_image_element_and_dw_and_dh(i, 0.0, 0.0, width, height)
            }
            (Self::Element(_, c), Image::Bitmap(b)) => {
                c.draw_image_with_image_bitmap_and_dw_and_dh(&b.0, 0.0, 0.0, width, height)
            }
            (Self::Element(_, c), Image::Element { image: i, .. }) => {
                c.draw_image_with_html_image_element_and_dw_and_dh(i, 0.0, 0.0, width, height)
            }
        }
        .map_err(|error| Error::Decode(message(&error)))
    }

    fn size(&self) -> (u32, u32) {
        match self {
            Self::Offscreen(canvas, _) => (canvas.width(), canvas.height()),
            Self::Element(canvas, _) => (canvas.width(), canvas.height()),
        }
    }

    /// Encodes the canvas, or `None` when the browser cannot. The blob's type
    /// is what the browser actually wrote, which is PNG when it does not
    /// know `mime`.
    pub(crate) async fn encode(&self, mime: &str, quality: f64) -> Option<Blob> {
        let promise = match self {
            Self::Offscreen(canvas, _) => {
                let options = ImageEncodeOptions::new();
                options.set_type(mime);
                options.set_quality(quality);
                canvas.convert_to_blob_with_options(&options).ok()?
            }
            Self::Element(canvas, _) => Promise::new(&mut |resolve, reject| {
                if let Err(error) =
                    canvas.to_blob_with_type_and_encoder_options(&resolve, mime, &quality.into())
                {
                    let _ = reject.call1(&JsValue::UNDEFINED, &error);
                }
            }),
        };
        JsFuture::from(promise).await.ok()?.dyn_into::<Blob>().ok()
    }
}

/// A `File` with `blob`'s bytes.
pub(crate) fn file_from_blob(
    blob: &Blob,
    name: &str,
    mime: &str,
    last_modified: f64,
) -> Result<File, Error> {
    let options = FilePropertyBag::new();
    options.set_type(mime);
    options.set_last_modified(last_modified);
    File::new_with_blob_sequence_and_options(&Array::of1(blob), name, &options)
        .map_err(|error| Error::browser(&error))
}

/// Replaces the files of `input`.
pub(crate) fn replace_files(
    input: &HtmlInputElement,
    files: impl IntoIterator<Item = File>,
) -> Result<(), Error> {
    let transfer = DataTransfer::new().map_err(|error| Error::browser(&error))?;
    let items = transfer.items();
    for file in files {
        items
            .add_with_file(&file)
            .map_err(|error| Error::browser(&error))?;
    }
    let files = transfer
        .files()
        .ok_or_else(|| Error::Browser("the DataTransfer has no files".into()))?;
    input.set_files(Some(&files));
    Ok(())
}
