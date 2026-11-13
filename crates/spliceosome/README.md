# spliceosome

Cuts the junk out of uploaded images.

`spliceosome` turns an untrusted upload (a profile picture, say) into a safe
JPEG, PNG or WebP file. It works on the container structure only, so it is
fast, needs no image library or C dependency, never decodes a pixel and runs
anywhere Rust does, `no_std` and WebAssembly included.

- **The bytes decide the format**, not the file name or the content type the
  browser claims. Anything other than JPEG, PNG or WebP (GIF, SVG, HTML,
  scripts) is refused, and so are animated WebP files.
- **Limits come first**: the file size, and the pixel count read from the
  image header, so a tiny file that would decode to gigabytes (a
  decompression bomb) is refused without being decoded.
- **The file is rebuilt from only the parts a viewer needs.** Metadata (EXIF
  with the camera and the GPS position, XMP, IPTC, comments, PNG text chunks,
  time stamps) and anything appended after the end of the image (a script
  hidden in a polyglot file, say) are left out. Colour information is kept,
  and a JPEG keeps its EXIF orientation alone, so phone photos still stand the
  right way up.

The compressed picture data is copied as it is, so the result looks exactly
like the upload. Broken files are refused rather than repaired, and no input
makes the sanitiser panic.

## Usage

```rust
use spliceosome::{Error, Limits};

fn avatar(upload: &[u8]) -> Result<(String, &'static str, Vec<u8>), Error> {
    let limits = Limits { max_bytes: 2 * 1024 * 1024, max_pixels: 25_000_000 };
    let clean = spliceosome::sanitize(upload, &limits)?;

    let file_name = format!("avatar.{}", clean.extension()); // "jpg", "png" or "webp"
    let content_type = clean.mime(); // "image/jpeg", ...
    let (_width, _height) = clean.dimensions();
    Ok((file_name, content_type, clean.into_bytes()))
}
```

`Limits::default()` is the 2 MiB / 25 megapixel pair above. A `Sanitizer`
holds the limits if you prefer an object: `Sanitizer::new(limits).sanitize(upload)`.

Errors are meant for logs (`"image is 8000x8000 pixels, over the limit of
25000000 pixels"`); map them to your own messages for the user:

| Error | Meaning |
|---|---|
| `TooLarge` | the file is over `max_bytes` |
| `TooManyPixels` | width x height is over `max_pixels` |
| `UnsupportedFormat` | not a JPEG, PNG or WebP |
| `Animated` | an animated WebP |
| `Truncated`, `Malformed` | not a readable image |

## What is kept

| Format | Kept | Left out |
|---|---|---|
| JPEG | tables, frame, scans, restart interval, APP0 (JFIF), APP2 (ICC profile), APP14 (Adobe); the EXIF orientation, rewritten as a minimal EXIF block | APP1 (EXIF, XMP), APP13 (IPTC), comments, other application segments, data after the end-of-image marker |
| PNG | `IHDR`, `PLTE`, `IDAT`, `IEND`, `tRNS`, `gAMA`, `cHRM`, `sRGB`, `iCCP`, `sBIT` | text and EXIF chunks, `tIME`, APNG animation (the default image stays), unknown chunks, data after `IEND` |
| WebP | `VP8 ` or `VP8L` picture, `ALPH`, `ICCP` behind a fresh `VP8X` header | `EXIF`, `XMP `, unknown chunks, data after the RIFF container |

Every PNG chunk checksum is verified.

## Serving the result

A sanitised file is a much smaller risk than the upload, but still serve it
with the content type from `Clean::mime`, `X-Content-Type-Options: nosniff`
and, ideally, a restrictive `Content-Security-Policy`. Also cap the request
body size in the web server in front of your app, so oversized uploads never
reach it.

## Testing

Besides the unit and integration tests, `tests/no_panic.rs` throws random and
mutated images at the sanitiser and checks that it never panics, that the
output is never much larger than the input and that sanitising twice changes
nothing. Run it longer with `PROPTEST_CASES=1000000 cargo test --release
--test no_panic`.

## Licence

Licensed under either of
[Apache License, Version 2.0](https://github.com/hospodskej/rust-crates/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/hospodskej/rust-crates/blob/main/LICENSE-MIT)
at your option.
