# attenuator

Shrinks photos in the browser before they are uploaded, in Rust/WASM ([web-sys](https://docs.rs/web-sys)). A photo picked in a file input is scaled down to at most 1600 px on its longer side, re-encoded as WebP through a canvas and swapped back into the input, so the form uploads a few hundred KB instead of a 5-10 MB phone photo. The server needs no image library.

An attenuator is the electronic part that reduces a signal's strength; this one reduces images.

```rust
use attenuator::{Format, Outcome, Shrink};

async fn on_change(input: web_sys::HtmlInputElement) {
    let shrink = Shrink::new().max_side(1600).quality(0.82).format(Format::WebP);
    match shrink.shrink_input(&input).await {
        Ok(Some(Outcome::Shrunk { width, height, .. })) => { /* the input now holds the WebP */ }
        Ok(_) => {}  // nothing picked, or the original is kept
        Err(_) => {} // the browser can't read it: the original is uploaded as it is
    }
}
```

`Shrink::shrink(file)` does the same for a single `web_sys::File` and returns `Outcome::Shrunk { file, width, height, format }` or `Outcome::Unchanged { file, reason }`.

## Rules

- Only `image/jpeg`, `image/png` and `image/webp` are processed. Anything else (GIF, SVG, HEIC, ...) is left alone.
- The longer side becomes at most 1600 px (`.max_side(…)`), keeping the aspect ratio, rounded to whole pixels, never enlarged (`attenuator::target_size`).
- EXIF orientation is applied (`imageOrientation: "from-image"`), so portrait phone photos stay upright.
- Encoded as WebP at quality 0.82 (`.quality(…)`, `.format(…)`). A browser that can't write WebP (Safari returns a PNG) gets JPEG at the same quality, named `.jpg`.
- The original is kept when it already fits and is at most 500 000 bytes (`.small_enough(…)`), or when the result is not smaller.
- The new file keeps the name with the extension swapped (`IMG_1234.JPG` → `IMG_1234.webp`) and the original's `lastModified`.
- A picture the browser can't read is an `Error`; the input keeps the original.
- `shrink_input` shrinks the first file only and swaps it in through a `DataTransfer`, unless the input's file changed in the meantime.

Decoding uses `createImageBitmap` where available, else an `<img>` with an object URL; drawing uses `OffscreenCanvas`, else a `<canvas>`. Object URLs are revoked and bitmaps closed even if the future is dropped, and no Rust closures are handed to JavaScript.

A form submitted while a photo is being shrunk uploads the original, so disable the submit button until `shrink_input` is done (as below), or prevent the submit and `requestSubmit()` afterwards.

## With a Leptos 0.8 island

```rust
use attenuator::Shrink;
use leptos::{prelude::*, task::spawn_local};
use web_sys::HtmlInputElement;

#[island]
pub fn PhotoInput(name: String) -> impl IntoView {
    let (shrinking, set_shrinking) = signal(false);
    view! {
        <input type="file" name=name accept="image/jpeg,image/png,image/webp"
            on:change=move |ev| {
                let input: HtmlInputElement = event_target(&ev);
                set_shrinking.set(true);
                spawn_local(async move {
                    // On an error the input keeps the original, which uploads fine.
                    let _ = Shrink::new().shrink_input(&input).await;
                    set_shrinking.set(false);
                });
            }
        />
        <button type="submit" disabled=shrinking>"Save"</button>
    }
}
```

## Targets

The crate compiles on every target, so code that uses it builds and tests natively, but the browser calls only work on `wasm32-unknown-unknown` in a browser. `target_size`, `Shrink::accepts` and `Shrink::plan` are plain Rust.

## Browser test

`tests/browser/run.mjs` builds `examples/browser_harness.rs` for wasm32, binds it with `wasm-bindgen` and checks the real thing in headless Chromium with Playwright: large JPEG and PNG pictures become WebP of at most 1600 px and smaller, EXIF orientation, the kept-original cases, the `<img>`/`<canvas>` fallbacks, object URL cleanup and the JPEG fallback when WebP can't be written. It needs the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` of the same version as the `wasm-bindgen` crate in `Cargo.lock`, and node with Playwright and Chromium:

```sh
cargo install wasm-bindgen-cli --version <same as wasm-bindgen in Cargo.lock>
node crates/attenuator/tests/browser/run.mjs
```

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
