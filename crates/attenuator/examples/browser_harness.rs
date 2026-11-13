//! The page `tests/browser/run.mjs` drives in headless Chromium: it exposes
//! [`attenuator::Shrink`] to JavaScript. See the README for how to run it.

use attenuator::{Error, Format, Outcome, Reason, Shrink};
use js_sys::{Object, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use web_sys::{File, HtmlInputElement};

fn set(object: &Object, key: &str, value: impl Into<JsValue>) {
    let _ = Reflect::set(object, &key.into(), &value.into());
}

fn number(options: &JsValue, key: &str) -> Option<f64> {
    Reflect::get(options, &key.into()).ok()?.as_f64()
}

fn settings(options: &JsValue) -> Shrink {
    let mut shrink = Shrink::new();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        if let Some(max_side) = number(options, "maxSide") {
            shrink = shrink.max_side(max_side as u32);
        }
        if let Some(bytes) = number(options, "smallEnough") {
            shrink = shrink.small_enough(bytes as u64);
        }
    }
    if let Some(quality) = number(options, "quality") {
        shrink = shrink.quality(quality);
    }
    if Reflect::get(options, &"format".into())
        .ok()
        .and_then(|format| format.as_string())
        .is_some_and(|format| format == "jpeg")
    {
        shrink = shrink.format(Format::Jpeg);
    }
    shrink
}

fn report(result: Result<Option<Outcome>, Error>) -> JsValue {
    let object = Object::new();
    match result {
        Ok(None) => set(&object, "status", "none"),
        Ok(Some(outcome)) => {
            let file = outcome.file();
            set(&object, "name", file.name());
            set(&object, "type", file.type_());
            set(&object, "size", outcome.size());
            set(&object, "lastModified", file.last_modified());
            if let Some((width, height)) = outcome.dimensions() {
                set(&object, "width", width);
                set(&object, "height", height);
            }
            match outcome {
                Outcome::Shrunk { .. } => set(&object, "status", "shrunk"),
                Outcome::Unchanged { reason, .. } => {
                    set(&object, "status", "unchanged");
                    let reason = match reason {
                        Reason::UnsupportedType => "unsupported-type",
                        Reason::SmallEnough { .. } => "small-enough",
                        Reason::NotSmaller { .. } => "not-smaller",
                        _ => "other",
                    };
                    set(&object, "reason", reason);
                }
            }
        }
        Err(error) => {
            set(&object, "status", "error");
            set(&object, "error", error.to_string());
        }
    }
    object.into()
}

/// Shrinks `file` with the settings in `options`
/// (`{ maxSide, quality, smallEnough, format: "webp" | "jpeg" }`).
#[wasm_bindgen(js_name = shrinkFile)]
pub fn shrink_file(file: File, options: &JsValue) -> Promise {
    let shrink = settings(options);
    future_to_promise(async move { Ok(report(shrink.shrink(file).await.map(Some))) })
}

/// Shrinks the file picked in `input` and swaps it in.
#[wasm_bindgen(js_name = shrinkInput)]
pub fn shrink_input(input: HtmlInputElement, options: &JsValue) -> Promise {
    let shrink = settings(options);
    future_to_promise(async move { Ok(report(shrink.shrink_input(&input).await)) })
}
