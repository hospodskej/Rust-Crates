// Drives the crate in headless Chromium: builds examples/browser_harness.rs
// for wasm32, binds it with wasm-bindgen-cli and checks real shrinking.
//
//   node crates/attenuator/tests/browser/run.mjs
//
// Needs the wasm32-unknown-unknown target, `wasm-bindgen` (the CLI, same
// version as the wasm-bindgen crate in Cargo.lock) on PATH, and Playwright
// with Chromium (`require("playwright")` must work from node).

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const crate = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const target = process.env.CARGO_TARGET_DIR ?? resolve(crate, "../../target");
const out = join(target, "attenuator-browser");

execFileSync(
  "cargo",
  ["build", "-p", "attenuator", "--example", "browser_harness", "--target", "wasm32-unknown-unknown", "--release"],
  { stdio: "inherit", cwd: crate },
);
execFileSync(
  "wasm-bindgen",
  [
    "--target", "web", "--no-typescript", "--out-dir", out, "--out-name", "harness",
    join(target, "wasm32-unknown-unknown/release/examples/browser_harness.wasm"),
  ],
  { stdio: "inherit" },
);

const origin = "http://attenuator.test";
const page_html = `<!doctype html><meta charset="utf-8"><input type="file" id="photo">
<script type="module">
  import init, * as harness from "./harness.js";
  await init();
  window.harness = harness;
  window.ready = true;
</script>`;

// Helpers that run inside the page.
const helpers = () => {
  // A photo-like picture: gradients, shapes and some grain, so encoders
  // cannot squeeze it to nothing. Returns a File.
  window.makePicture = async (width, height, type, quality, name, { halves = false } = {}) => {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const c = canvas.getContext("2d");
    if (halves) {
      c.fillStyle = "rgb(255,0,0)";
      c.fillRect(0, 0, width / 2, height);
      c.fillStyle = "rgb(0,0,255)";
      c.fillRect(width / 2, 0, width / 2, height);
    } else {
      const g = c.createLinearGradient(0, 0, width, height);
      g.addColorStop(0, "#2a6"); g.addColorStop(0.5, "#fd5"); g.addColorStop(1, "#46c");
      c.fillStyle = g;
      c.fillRect(0, 0, width, height);
      let seed = 1;
      const rnd = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
      for (let i = 0; i < 400; i++) {
        c.fillStyle = `hsla(${rnd() * 360},70%,50%,0.5)`;
        c.beginPath();
        c.arc(rnd() * width, rnd() * height, rnd() * width / 10, 0, 7);
        c.fill();
      }
      const grain = c.getImageData(0, 0, width, height);
      for (let i = 0; i < grain.data.length; i += 4) {
        const n = (rnd() - 0.5) * 40;
        grain.data[i] += n; grain.data[i + 1] += n; grain.data[i + 2] += n;
      }
      c.putImageData(grain, 0, 0);
    }
    const blob = await new Promise((r) => canvas.toBlob(r, type, quality));
    return new File([blob], name, { type, lastModified: 1700000000000 });
  };

  // `file` (a JPEG) with an EXIF APP1 segment saying "orientation 6"
  // (rotate 90 degrees clockwise to display).
  window.withOrientation6 = async (file) => {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const tiff = [0x4d, 0x4d, 0x00, 0x2a, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0, 0];
    const payload = [0x45, 0x78, 0x69, 0x66, 0, 0, ...tiff];
    const length = payload.length + 2;
    const app1 = [0xff, 0xe1, length >> 8, length & 0xff, ...payload];
    const out = new Uint8Array(bytes.length + app1.length);
    out.set(bytes.subarray(0, 2));
    out.set(app1, 2);
    out.set(bytes.subarray(2), 2 + app1.length);
    return new File([out], file.name, { type: "image/jpeg", lastModified: file.lastModified });
  };

  // What a File really holds: its magic bytes, decoded size and, optionally,
  // the colour at a point.
  window.inspect = async (file, points = []) => {
    const head = new Uint8Array(await file.slice(0, 12).arrayBuffer());
    const ascii = (a, b) => String.fromCharCode(...head.subarray(a, b));
    const magic = ascii(0, 4) === "RIFF" && ascii(8, 12) === "WEBP" ? "webp"
      : head[0] === 0xff && head[1] === 0xd8 ? "jpeg"
      : head[1] === 0x50 && head[2] === 0x4e ? "png" : "other";
    const bitmap = await createImageBitmap(file);
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const c = canvas.getContext("2d");
    c.drawImage(bitmap, 0, 0);
    const colours = points.map(([x, y]) => Array.from(c.getImageData(x * bitmap.width, y * bitmap.height, 1, 1).data.slice(0, 3)));
    return { magic, width: bitmap.width, height: bitmap.height, size: file.size, type: file.type, name: file.name, colours };
  };
};

const browser = await chromium.launch();
let failures = 0;
const check = (name, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `  ${JSON.stringify(detail)}` : ""}`);
  if (!ok) failures++;
};

async function newPage(setup) {
  const context = await browser.newContext();
  await context.route(`${origin}/**`, (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/") return route.fulfill({ contentType: "text/html", body: page_html });
    const types = { ".js": "text/javascript", ".wasm": "application/wasm" };
    const ext = path.slice(path.lastIndexOf("."));
    return route.fulfill({ contentType: types[ext] ?? "application/octet-stream", body: readFileSync(join(out, path)) });
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => check(`no page error (${e.message})`, false));
  if (setup) await page.addInitScript(setup);
  await page.goto(`${origin}/`);
  await page.waitForFunction(() => window.ready);
  await page.evaluate(helpers);
  return page;
}

try {
  // --- The normal path: createImageBitmap + OffscreenCanvas, WebP. ---
  let page = await newPage();
  const used = await page.evaluate(() => ({ bitmap: "createImageBitmap" in window, offscreen: "OffscreenCanvas" in window }));
  check("Chromium has createImageBitmap and OffscreenCanvas", used.bitmap && used.offscreen, used);

  // A large JPEG picked in a real file input.
  const big = await page.evaluate(async () => {
    const file = await makePicture(4000, 3000, "image/jpeg", 0.95, "IMG_1234.JPG");
    return { size: file.size, bytes: Array.from(new Uint8Array(await file.arrayBuffer())) };
  });
  await page.setInputFiles("#photo", { name: "IMG_1234.JPG", mimeType: "image/jpeg", buffer: Buffer.from(big.bytes) });
  let r = await page.evaluate(async () => {
    const input = document.getElementById("photo");
    const result = await harness.shrinkInput(input, {});
    return { result, file: await inspect(input.files[0]), count: input.files.length };
  });
  check("large JPEG via the input: shrunk", r.result.status === "shrunk", r.result);
  check("  input now holds WebP named IMG_1234.webp", r.file.magic === "webp" && r.file.type === "image/webp" && r.file.name === "IMG_1234.webp" && r.count === 1, r.file);
  check("  1600x1200 (at most 1600 px)", r.file.width === 1600 && r.file.height === 1200 && r.result.width === 1600, r.file);
  check(`  smaller (${big.size} -> ${r.file.size} bytes)`, r.file.size < big.size && r.file.size === r.result.size);

  // A large PNG, portrait.
  r = await page.evaluate(async () => {
    const file = await makePicture(3000, 4000, "image/png", undefined, "scan.png");
    const result = await harness.shrinkFile(file, {});
    return { before: file.size, result };
  });
  check(`large PNG: WebP 1200x1600, smaller (${r.before} -> ${r.result.size})`, r.result.status === "shrunk" && r.result.type === "image/webp" && r.result.name === "scan.webp" && r.result.width === 1200 && r.result.height === 1600 && r.result.size < r.before, r.result);
  check("  keeps lastModified", r.result.lastModified === 1700000000000, r.result.lastModified);

  // Fits, but over 500 000 bytes: re-encoded at the same size.
  r = await page.evaluate(async () => {
    const file = await makePicture(1600, 1200, "image/png", undefined, "big.png");
    return { before: file.size, result: await harness.shrinkFile(file, {}) };
  });
  check(`fits but ${r.before} bytes > 500 000: re-encoded at 1600x1200`, r.before > 500000 && r.result.status === "shrunk" && r.result.width === 1600 && r.result.height === 1200, r.result);

  // Already small enough.
  r = await page.evaluate(async () => {
    const file = await makePicture(800, 600, "image/jpeg", 0.8, "small.jpg");
    return { before: file.size, result: await harness.shrinkFile(file, {}) };
  });
  check(`800x600, ${r.before} bytes: kept (small enough)`, r.result.status === "unchanged" && r.result.reason === "small-enough" && r.result.name === "small.jpg" && r.result.width === 800, r.result);

  // Re-encoding would not be smaller.
  r = await page.evaluate(async () => {
    const file = await makePicture(1000, 800, "image/jpeg", 0.2, "tiny.jpg");
    return { before: file.size, result: await harness.shrinkFile(file, { smallEnough: 0 }) };
  });
  check(`low-quality JPEG (${r.before} bytes), WebP 0.82 is bigger: kept`, r.result.status === "unchanged" && r.result.reason === "not-smaller" && r.result.name === "tiny.jpg", r.result);

  // Types that are left alone.
  r = await page.evaluate(async () => {
    const png = await makePicture(3000, 3000, "image/png", undefined, "x.png");
    const gif = new File([png], "anim.gif", { type: "image/gif" });
    const svg = new File(["<svg/>"], "logo.svg", { type: "image/svg+xml" });
    return [await harness.shrinkFile(gif, {}), await harness.shrinkFile(svg, {})];
  });
  check("GIF and SVG: left alone", r.every((x) => x.status === "unchanged" && x.reason === "unsupported-type"), r);

  // EXIF orientation 6: stored 3200x1600 red|blue, displayed 1600x3200 red over blue.
  r = await page.evaluate(async () => {
    const plain = await makePicture(3200, 1600, "image/jpeg", 0.95, "side.jpg", { halves: true });
    const file = await withOrientation6(plain);
    const result = await harness.shrinkFile(file, {});
    // shrinkFile reports; shrink again through the input to get the File.
    const dt = new DataTransfer(); dt.items.add(file);
    const input = document.getElementById("photo"); input.files = dt.files;
    await harness.shrinkInput(input, {});
    return { result, file: await inspect(input.files[0], [[0.5, 0.1], [0.5, 0.9]]) };
  });
  check("EXIF orientation 6: upright 800x1600", r.result.width === 800 && r.result.height === 1600 && r.file.width === 800 && r.file.height === 1600, r.result);
  const red = ([R, G, B]) => R > 200 && G < 60 && B < 60;
  const blue = ([R, G, B]) => B > 200 && R < 60 && G < 60;
  check("  rotated clockwise (red on top, blue below)", red(r.file.colours[0]) && blue(r.file.colours[1]), r.file.colours);

  // A file the browser cannot read: an error, and the input keeps the original.
  r = await page.evaluate(async () => {
    const junk = new File([new Uint8Array(5000).fill(7)], "broken.jpg", { type: "image/jpeg" });
    const dt = new DataTransfer(); dt.items.add(junk);
    const input = document.getElementById("photo"); input.files = dt.files;
    const result = await harness.shrinkInput(input, {});
    return { result, kept: input.files[0] === junk };
  });
  check("unreadable picture: error, input keeps the original", r.result.status === "error" && r.kept, r.result);

  // The pick changes while shrinking: the newer file is left in place.
  r = await page.evaluate(async () => {
    const first = await makePicture(4000, 3000, "image/jpeg", 0.9, "first.jpg");
    const second = new File(["x"], "second.txt", { type: "text/plain" });
    const input = document.getElementById("photo");
    let dt = new DataTransfer(); dt.items.add(first); input.files = dt.files;
    const pending = harness.shrinkInput(input, {});
    await new Promise((r) => setTimeout(r, 0)); // shrinking has started
    dt = new DataTransfer(); dt.items.add(second); input.files = dt.files;
    return { result: await pending, name: input.files[0].name };
  });
  check("pick changed while shrinking: newer pick untouched", r.result.status === "none" && r.name === "second.txt", r);

  // No input file at all.
  r = await page.evaluate(async () => {
    const input = document.createElement("input"); input.type = "file";
    return harness.shrinkInput(input, {});
  });
  check("empty input: nothing to do", r.status === "none", r);

  // Custom settings.
  r = await page.evaluate(async () => {
    const file = await makePicture(4000, 3000, "image/jpeg", 0.95, "a.jpeg");
    return harness.shrinkFile(file, { maxSide: 1000, quality: 0.6, format: "jpeg" });
  });
  check("max_side 1000 + JPEG: a.jpg 1000x750", r.status === "shrunk" && r.type === "image/jpeg" && r.name === "a.jpg" && r.width === 1000 && r.height === 750, r);
  await page.context().close();

  // --- Fallbacks: no createImageBitmap, no OffscreenCanvas. ---
  page = await newPage(() => {
    delete window.createImageBitmap;
    delete window.OffscreenCanvas;
    window.urls = { created: 0, revoked: 0 };
    const create = URL.createObjectURL, revoke = URL.revokeObjectURL;
    URL.createObjectURL = (b) => { window.urls.created++; return create(b); };
    URL.revokeObjectURL = (u) => { window.urls.revoked++; return revoke(u); };
  });
  r = await page.evaluate(async () => {
    const gone = !("createImageBitmap" in window) && !("OffscreenCanvas" in window);
    // inspect() needs createImageBitmap, so look at the result's header only.
    const plain = await makePicture(3200, 1600, "image/jpeg", 0.95, "side.jpg", { halves: true });
    const file = await withOrientation6(plain);
    const result = await harness.shrinkFile(file, {});
    const big = await harness.shrinkFile(await makePicture(4000, 3000, "image/png", undefined, "p.png"), {});
    const junk = await harness.shrinkFile(new File([new Uint8Array(100)], "bad.png", { type: "image/png" }), {});
    return { gone, result, big, junk, urls: window.urls };
  });
  check("fallback page has neither API", r.gone);
  check("<img> + <canvas>: EXIF picture upright 800x1600 WebP", r.result.status === "shrunk" && r.result.type === "image/webp" && r.result.width === 800 && r.result.height === 1600, r.result);
  check("<img> + <canvas>: large PNG -> 1600x1200 WebP", r.big.status === "shrunk" && r.big.width === 1600 && r.big.height === 1200, r.big);
  check("<img>: unreadable picture is an error", r.junk.status === "error", r.junk);
  check("every object URL revoked", r.urls.created === 3 && r.urls.revoked === 3, r.urls);
  await page.context().close();

  // --- A browser that cannot write WebP (hands back PNG, like old Safari). ---
  page = await newPage(() => {
    const toBlob = HTMLCanvasElement.prototype.toBlob;
    HTMLCanvasElement.prototype.toBlob = function (cb, type, q) {
      return toBlob.call(this, cb, type === "image/webp" ? "image/png" : type, q);
    };
    const convert = OffscreenCanvas.prototype.convertToBlob;
    OffscreenCanvas.prototype.convertToBlob = function (o = {}) {
      return convert.call(this, o.type === "image/webp" ? { ...o, type: "image/png" } : o);
    };
  });
  r = await page.evaluate(async () => {
    const file = await makePicture(4000, 3000, "image/jpeg", 0.95, "IMG_9.jpeg");
    const result = await harness.shrinkFile(file, {});
    return { before: file.size, result };
  });
  check("no WebP encoder: falls back to JPEG IMG_9.jpg", r.result.status === "shrunk" && r.result.type === "image/jpeg" && r.result.name === "IMG_9.jpg" && r.result.width === 1600 && r.result.size < r.before, r.result);
  await page.context().close();
} finally {
  await browser.close();
}

console.log(failures ? `\n${failures} check(s) failed` : "\nall browser checks passed");
process.exit(failures ? 1 : 0);
