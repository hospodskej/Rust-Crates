//! The code generator, used from `build.rs`.

mod codegen;
mod ident;
mod model;
mod parse;
mod template;

use std::fmt;
use std::path::{Path, PathBuf};

/// Generates typed translation functions from Rails-style YAML locale files.
///
/// Call it from `build.rs`; the first locale is the default one.
///
/// ```no_run
/// codon::Build::new()
///     .locale("cs", "locales/cs.yml")
///     .locale("de", "locales/de.yml")
///     .url_prefix("de", "at")
///     .generate()
///     .unwrap();
/// ```
///
/// and include the result with `codon::include!()`.
///
/// # Input
///
/// Each file is a Rails locale file: a mapping whose only top-level key is
/// the locale code (`cs:`), with nested mappings of translation keys below.
///
/// # Generated code
///
/// - `pub enum Locale` with a variant per locale (`cs` → `Cs`, `de-AT` →
///   `DeAt`), `Default` = the first locale, `ALL`, `DEFAULT`, `as_str()`,
///   `FromStr` (case-insensitive), `Display`, `url_prefix()`,
///   `from_url_prefix()`, `PATHS` (a [`path::Prefixes`](crate::path::Prefixes)
///   table) and `plural_rule()`. With codon's `serde` feature it is
///   `Serialize`/`Deserialize` as its code.
/// - `pub mod t` (see [`Build::module_name`]) mirroring the YAML tree: the key
///   `nav.domu` becomes `t::nav::domu(locale: Locale)`. Keys that are Rust
///   keywords become raw identifiers (`r#type`, or `crate_` for the few that
///   cannot be raw), other characters (including non-ASCII letters) become
///   `_`, and a leading digit gets an `n` prefix (`404` → `n404`). Two keys
///   that end up with the same name are an error.
/// - `pub fn lookup(locale: Locale, key: &str) -> Option<&'static str>` for
///   dotted keys of the plain strings (no placeholders, no plurals, lists or
///   `_html`), for the rare key that is only known at runtime.
///
/// Each function returns:
///
/// | YAML | Rust |
/// |---|---|
/// | `key: "text"` | `fn key(locale) -> &'static str` |
/// | `key: "Hi %{name}"` | `fn key(locale, name: impl Display) -> String` |
/// | `key: [a, b]` | `fn key(locale) -> &'static [&'static str]` |
/// | `key: {one: …, other: …}` or `key_one: …` + `key_other: …` | `fn key(locale, count: i64, …) -> String` |
/// | `key_html: "<b>%{name}</b>"` | `fn key(locale, name: impl HtmlArg) -> Html` |
///
/// - Placeholders are Rails' `%{name}`; `%%` is a literal `%` in strings that
///   have placeholders. Parameters come in **alphabetical order**, after
///   `count` for plurals. Every locale must use the same placeholders.
/// - Plurals pick their form with the locale's CLDR rule
///   ([`plural::PluralRule`](crate::plural::PluralRule)); `count` is also
///   available as `%{count}`. A `zero` form, when present, is used for 0
///   (as in Rails); a form the language needs but the YAML lacks falls back
///   to `other`, which is required. Sibling keys form a plural when at least
///   two of them share a stem and end in `_zero`, `_one`, `_two`, `_few`,
///   `_many` or `_other`; a lone `payment_other` stays a plain key.
/// - Keys ending in `_html` drop the suffix and return [`Html`](crate::Html);
///   interpolated values are HTML-escaped unless they are `Html` themselves
///   (see [`HtmlArg`](crate::HtmlArg)). Lists of `_html` keys return
///   `&'static [Html]`.
/// - Numbers and booleans are rendered as strings; `key:` with no value
///   counts as missing.
///
/// # Missing keys
///
/// A key of the default locale that another locale lacks is an error (all of
/// them are listed). With [`Build::fallback_to_default`] it is a cargo warning
/// instead and the default locale's text is used. Keys that only a
/// non-default locale has are ignored with a warning.
#[derive(Clone, Debug)]
pub struct Build {
    locales: Vec<Source>,
    fallback: bool,
    url_prefixes: Vec<(String, String)>,
    module: String,
    output: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct Source {
    code: String,
    origin: Origin,
}

#[derive(Clone, Debug)]
enum Origin {
    File(PathBuf),
    Inline { name: String, yaml: String },
}

impl Origin {
    fn name(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Inline { name, .. } => name.clone(),
        }
    }
}

impl Default for Build {
    fn default() -> Self {
        Self::new()
    }
}

impl Build {
    /// A builder with no locales, writing `$OUT_DIR/codon.rs`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            locales: Vec::new(),
            fallback: false,
            url_prefixes: Vec::new(),
            module: "t".to_owned(),
            output: None,
        }
    }

    /// Adds a locale read from a YAML file. The first locale added is the
    /// default one. Relative paths are relative to the crate root (the
    /// working directory of build scripts).
    pub fn locale(&mut self, code: impl Into<String>, path: impl AsRef<Path>) -> &mut Self {
        self.locales.push(Source {
            code: code.into(),
            origin: Origin::File(path.as_ref().to_path_buf()),
        });
        self
    }

    /// Adds a locale from YAML source text; `name` stands for the file name
    /// in messages. Useful for embedded or generated YAML, and in tests.
    ///
    /// ```
    /// let generated = codon::Build::new()
    ///     .locale_yaml("en", "en.yml", "en:\n  hello: \"Hello, %{name}!\"\n")
    ///     .render()
    ///     .unwrap();
    /// assert!(generated.code().contains("pub fn hello"));
    /// ```
    pub fn locale_yaml(
        &mut self,
        code: impl Into<String>,
        name: impl Into<String>,
        yaml: impl Into<String>,
    ) -> &mut Self {
        self.locales.push(Source {
            code: code.into(),
            origin: Origin::Inline {
                name: name.into(),
                yaml: yaml.into(),
            },
        });
        self
    }

    /// Whether a key missing in a non-default locale falls back to the
    /// default locale's text (with a cargo warning) instead of failing the
    /// build. Off by default.
    pub fn fallback_to_default(&mut self, fallback: bool) -> &mut Self {
        self.fallback = fallback;
        self
    }

    /// Serves a non-default locale under a URL prefix, e.g.
    /// `.url_prefix("de", "at")` for `/at/kontakt`. The prefix is one path
    /// segment, without slashes. Locales without a prefix (always the default
    /// one) are served at the root.
    pub fn url_prefix(
        &mut self,
        locale: impl Into<String>,
        prefix: impl Into<String>,
    ) -> &mut Self {
        self.url_prefixes.push((locale.into(), prefix.into()));
        self
    }

    /// The name of the generated module of translation functions, `t` by
    /// default.
    pub fn module_name(&mut self, name: impl Into<String>) -> &mut Self {
        self.module = name.into();
        self
    }

    /// Writes the generated code to this file instead of `$OUT_DIR/codon.rs`.
    pub fn output(&mut self, path: impl AsRef<Path>) -> &mut Self {
        self.output = Some(path.as_ref().to_path_buf());
        self
    }

    /// Reads the locales and generates the code without writing or printing
    /// anything.
    ///
    /// # Errors
    ///
    /// If a file cannot be read or parsed, the configuration is invalid, or
    /// the translations have problems (missing keys, placeholders that differ
    /// between locales, plurals without `other`, name clashes, …). All
    /// translation problems are reported at once.
    pub fn render(&self) -> Result<Generated, Error> {
        let mut inputs = Vec::with_capacity(self.locales.len());
        for source in &self.locales {
            let yaml = match &source.origin {
                Origin::File(path) => std::fs::read_to_string(path).map_err(|e| Error::Io {
                    path: path.clone(),
                    source: e,
                })?,
                Origin::Inline { yaml, .. } => yaml.clone(),
            };
            inputs.push(model::Input {
                code: source.code.clone(),
                file: source.origin.name(),
                yaml,
            });
        }
        let options = model::Options {
            fallback: self.fallback,
            url_prefixes: self.url_prefixes.clone(),
            module: self.module.clone(),
        };
        let mut warnings = Vec::new();
        let model = model::build(&inputs, &options, &mut warnings)?;
        let code = codegen::generate(&model);
        Ok(Generated { code, warnings })
    }

    /// Generates the code and writes it to `$OUT_DIR/codon.rs` (or the
    /// [`output`](Build::output) file), only touching the file when its
    /// content changes. Prints `cargo:rerun-if-changed` for every locale
    /// file and `cargo:warning` for every warning. Returns the path written.
    ///
    /// # Errors
    ///
    /// As [`Build::render`], and if `OUT_DIR` is not set (outside a build
    /// script, without [`output`](Build::output)) or the file cannot be
    /// written.
    pub fn generate(&self) -> Result<PathBuf, Error> {
        for source in &self.locales {
            if let Origin::File(path) = &source.origin {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
        let generated = self.render()?;
        for warning in &generated.warnings {
            println!("cargo:warning=codon: {warning}");
        }
        let path = if let Some(path) = &self.output {
            path.clone()
        } else {
            let dir = std::env::var_os("OUT_DIR").ok_or_else(|| {
                Error::Config(
                    "OUT_DIR is not set; call generate() from build.rs or set output()".to_owned(),
                )
            })?;
            PathBuf::from(dir).join("codon.rs")
        };
        let unchanged = std::fs::read_to_string(&path).is_ok_and(|old| old == generated.code);
        if !unchanged {
            std::fs::write(&path, &generated.code).map_err(|e| Error::Io {
                path: path.clone(),
                source: e,
            })?;
        }
        Ok(path)
    }
}

/// Generated code and the warnings found while generating it.
#[derive(Clone, Debug)]
pub struct Generated {
    code: String,
    warnings: Vec<String>,
}

impl Generated {
    /// The generated Rust source.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Warnings (missing keys with fallback, keys only in a non-default
    /// locale, unknown plural rules), without the `cargo:warning=` prefix.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The generated Rust source.
    #[must_use]
    pub fn into_code(self) -> String {
        self.code
    }
}

/// A problem with a translation key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The locale file (as given to [`Build::locale`]).
    pub file: String,
    /// The dotted key, without the locale (`nav.domu`).
    pub key: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.key.is_empty() {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(f, "{}: `{}`: {}", self.file, self.key, self.message)
        }
    }
}

/// Why code generation failed.
///
/// `Debug` prints the same readable message as `Display`, so that a
/// `build.rs` returning `Result<(), Box<dyn Error>>` shows it as is.
#[non_exhaustive]
pub enum Error {
    /// A file could not be read or written.
    Io {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        source: std::io::Error,
    },
    /// A file is not valid YAML or not a locale file.
    Yaml {
        /// The file.
        file: String,
        /// What is wrong.
        message: String,
    },
    /// The builder configuration is invalid.
    Config(String),
    /// The translations have problems; all of them are listed.
    Translations(Vec<Problem>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "codon: {}: {source}", path.display()),
            Self::Yaml { file, message } => write!(f, "codon: {file}: {message}"),
            Self::Config(message) => write!(f, "codon: {message}"),
            Self::Translations(problems) => {
                write!(
                    f,
                    "codon: {} problem(s) in the translations:",
                    problems.len()
                )?;
                for problem in problems {
                    write!(f, "\n  {problem}")?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
