# codon

Compile-time checked translations for Rust. `codon` reads Rails-style YAML
locale files in your `build.rs` and generates a typed function for every
key, so a missing translation, a mistyped key or a wrong interpolation
argument is a compile error rather than a "translation missing" in production.

A codon is the unit through which DNA is *translated*.

- One function per key: `t::nav::domu(locale)`.
- `%{name}` placeholders become typed parameters.
- Plurals (`one`/`few`/`other`, nested or `key_one`/`key_other` siblings)
  take `count: i64` and follow the CLDR rules of each language.
- `_html` keys return trusted `Html` and escape what you interpolate.
- Lists, numbers and booleans work too.
- Locale prefixes in URLs (`/kontakt` ↔ `/at/kontakt`).
- The runtime is tiny and `no_std` + `alloc`, so it runs in
  `wasm32-unknown-unknown` (e.g. Leptos islands) as well as on the server.

## Setup

```toml
[dependencies]
codon = "0.1"

[build-dependencies]
codon = { version = "0.1", features = ["build"] }
```

`build.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    codon::Build::new()
        .locale("cs", "locales/cs.yml") // the first locale is the default
        .locale("de", "locales/de.yml")
        .url_prefix("de", "at")         // German pages live under /at
        .generate()?;                   // writes $OUT_DIR/codon.rs
    Ok(())
}
```

`src/i18n.rs`:

```rust
codon::include!(); // = include!(concat!(env!("OUT_DIR"), "/codon.rs"));
```

## Example

`locales/cs.yml` (`de.yml` has the same keys under `de:`):

```yaml
cs:
  nav:
    domu: "Domů"
  auth:
    new_intro: "Nastavte nové heslo pro účet %{email}."
    register_privacy_html: "Vytvořením účtu souhlasíte se %{privacy_link}."
    order_item_count_one: "%{count} položka"
    order_item_count_few: "%{count} položky"
    order_item_count_other: "%{count} položek"
  steps:
    - "Vyberte produkt"
    - "Objednejte"
```

```rust
use crate::i18n::{Locale, t};
use codon::Html;

let locale: Locale = "de".parse().unwrap_or_default();

t::nav::domu(locale);                                  // &'static str
t::auth::new_intro(locale, "jan@example.cz");          // String
t::auth::order_item_count(Locale::Cs, 3);              // "3 položky"
t::steps(locale);                                      // &'static [&'static str]

let link = Html::from_trusted(r#"<a href="/privacy">zásadami</a>"#);
t::auth::register_privacy(locale, &link);              // Html, link inserted as is
t::auth::register_privacy(locale, "<script>");         // Html, "&lt;script&gt;"

crate::i18n::lookup(locale, "nav.domu");               // Option<&'static str>
```

Each generated function carries the key and the text of every locale in its
doc comment, so your editor shows them on hover.

## Generated items

```rust
pub enum Locale { Cs, De }          // Copy, Eq, Hash, Ord, Debug, Default (first locale)
impl Locale {
    pub const ALL: [Locale; 2];
    pub const DEFAULT: Locale;
    pub const PATHS: codon::path::Prefixes<Locale>;
    pub const fn as_str(self) -> &'static str;
    pub const fn url_prefix(self) -> Option<&'static str>;
    pub fn from_url_prefix(prefix: &str) -> Option<Locale>;
    pub const fn plural_rule(self) -> codon::plural::PluralRule;
}
// Display, FromStr (ASCII case-insensitive), and with the `serde` feature
// Serialize/Deserialize as the code ("cs").

pub mod t { /* one module per YAML mapping, one function per key */ }
pub fn lookup(locale: Locale, key: &str) -> Option<&'static str>;
```

| YAML | Rust |
|---|---|
| `key: "text"` | `fn key(locale) -> &'static str` |
| `key: "Hi %{name}"` | `fn key(locale, name: impl Display) -> String` |
| `key: [a, b]` | `fn key(locale) -> &'static [&'static str]` |
| `key: {one: …, other: …}` or `key_one`/`key_other` | `fn key(locale, count: i64, …) -> String` |
| `key_html: "<b>%{x}</b>"` | `fn key(locale, x: impl HtmlArg) -> Html` |

Rules:

- **Parameters** come in alphabetical order, after `count` for plurals. All
  locales must use the same placeholders, or the build fails naming the key
  and the file. `%%` is a literal `%` in strings with placeholders.
- **Names**: Rust keywords become raw identifiers (`r#type`; `crate_`,
  `self_`, `super_`, `Self_` for the ones that cannot be raw); other
  characters become `_`; a leading digit gets an `n` (`404` → `n404`).
  Keys that clash after this are a build error.
- **Plurals** need an `other` form. A `zero` form, when present, is used for
  0 (as in Rails). Forms the language needs but the file lacks fall back to
  `other`; forms the language never uses (German `few`) produce a warning.
  Sibling keys are a plural when at least two share a stem and end in
  `_zero`/`_one`/`_two`/`_few`/`_many`/`_other`.
- **HTML**: values interpolated into `_html` keys are escaped. `Html` itself
  is inserted as is. Strings, numbers, `char` and `bool` implement `HtmlArg`
  directly; for any other `Display` type use `codon::Escaped(value)`. (A
  blanket `impl<T: Display> HtmlArg for T` would also cover `Html`, which
  must not be escaped, so it is not possible.)
- **Missing keys**: a key of the default locale missing in another locale is
  a build error, and all of them are listed at once. With
  `.fallback_to_default(true)` they become cargo warnings and use the
  default locale's text. Keys only present in a non-default locale are
  ignored with a warning.
- `lookup` covers plain strings only (no placeholders, plurals, lists or
  `_html`).
- The generated code refers to the runtime as `::codon`, so do not rename
  the dependency.

## URL prefixes

The default locale is served without a prefix, every other locale under its
own first path segment, which names the market and may differ from the locale
code:

```rust
use codon::path::Prefixes;

let paths = Prefixes::new([("de", "at")]);
assert_eq!(paths.localize("/kontakt#cenik", "de"), "/at/kontakt#cenik");
assert_eq!(paths.localize("/?x=1", "de"), "/at?x=1");
assert_eq!(paths.strip("/at/eshop/tramy"), "/eshop/tramy");
assert_eq!(paths.strip("/atlas"), "/atlas");
assert_eq!(paths.detect("/at/kontakt"), Some(("de", "/kontakt".into())));

// Or with the generated enum:
Locale::PATHS.localize("/kontakt", Locale::De); // "/at/kontakt"
```

## Features

- `build`: the code generator. Use it in `[build-dependencies]` only; it pulls
  in a YAML parser, `syn` and `prettyplease`, none of which reach your binary.
- `serde`: `Serialize`/`Deserialize` for the generated `Locale`.

## Limitations

- Only `%{name}` interpolation; Rails' `%<name>s` format directives are left
  as text.
- YAML anchors work, merge keys (`<<: *default`) do not. Files are parsed as
  YAML 1.2, so `yes`/`no` stay strings.
- Plural rules cover integer counts of the languages in
  `codon::plural::PluralRule`; other languages get a warning and the
  `one`-for-1 rule.

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE)
or [MIT license](../../LICENSE-MIT) at your option.
