//! Locale prefixes in URL paths.
//!
//! The default locale is served without a prefix (`/kontakt`); every other
//! locale under its own first path segment (`/at/kontakt`). The prefix names
//! the market and does not have to equal the locale code: German (`de`)
//! translations can be served under `/at`.
//!
//! Paths may carry a query string and an `#anchor`; both are kept.
//!
//! ```
//! use codon::path::Prefixes;
//!
//! let paths = Prefixes::new([("de", "at")]);
//!
//! assert_eq!(paths.localize("/kontakt#cenik", "de"), "/at/kontakt#cenik");
//! assert_eq!(paths.localize("/", "de"), "/at");
//! assert_eq!(paths.localize("/?x=1", "de"), "/at?x=1");
//! assert_eq!(paths.localize("/at/kontakt", "cs"), "/kontakt");
//!
//! assert_eq!(paths.strip("/at/eshop/tramy"), "/eshop/tramy");
//! assert_eq!(paths.strip("/at"), "/");
//! assert_eq!(paths.strip("/atlas"), "/atlas");
//!
//! assert_eq!(paths.detect("/at/kontakt"), Some(("de", "/kontakt".into())));
//! assert_eq!(paths.detect("/kontakt"), None);
//! ```
//!
//! The generated `Locale` enum has a ready-made table,
//! `Locale::PATHS: Prefixes<Locale>`, built from `Build::url_prefix`.

use alloc::borrow::Cow;
use alloc::format;
use alloc::vec::Vec;

/// A table of URL prefixes, one per non-default locale.
///
/// `L` is the locale type: a `&str` code, or the generated `Locale` enum.
/// Prefixes are matched case-sensitively and must be a single path segment
/// (no `/`, `?` or `#`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefixes<L: Clone + 'static = &'static str> {
    entries: Cow<'static, [(L, &'static str)]>,
}

impl<L: Clone + 'static> Prefixes<L> {
    /// Builds the table from `(locale, prefix)` pairs.
    ///
    /// # Panics
    ///
    /// If a prefix is empty or contains `/`, `?` or `#`.
    pub fn new(entries: impl IntoIterator<Item = (L, &'static str)>) -> Self {
        let entries: Vec<_> = entries.into_iter().collect();
        for (_, prefix) in &entries {
            assert!(
                is_valid_prefix(prefix),
                "invalid URL prefix {prefix:?}: it must be one non-empty path segment"
            );
        }
        Self {
            entries: Cow::Owned(entries),
        }
    }

    /// Builds the table from a static slice, in a `const` context.
    ///
    /// The prefixes are not validated; the generated code only passes
    /// prefixes that were checked at build time.
    ///
    /// ```
    /// use codon::path::Prefixes;
    ///
    /// const PATHS: Prefixes = Prefixes::from_static(&[("de", "at")]);
    /// assert_eq!(PATHS.prefix_for("de"), Some("at"));
    /// ```
    #[must_use]
    pub const fn from_static(entries: &'static [(L, &'static str)]) -> Self {
        Self {
            entries: Cow::Borrowed(entries),
        }
    }

    /// The `(locale, prefix)` pairs.
    #[must_use]
    pub fn entries(&self) -> &[(L, &'static str)] {
        &self.entries
    }

    /// The URL prefix of a locale, `None` for an unprefixed locale.
    // Taking the locale by value lets callers pass `"de"` or `Locale::De`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn prefix_for<Q>(&self, locale: Q) -> Option<&'static str>
    where
        L: PartialEq<Q>,
    {
        self.entries
            .iter()
            .find(|(l, _)| *l == locale)
            .map(|&(_, prefix)| prefix)
    }

    /// The locale served under a URL prefix (`"at"`, without slashes).
    #[must_use]
    pub fn locale_for(&self, prefix: &str) -> Option<L> {
        self.entries
            .iter()
            .find(|(_, p)| *p == prefix)
            .map(|(l, _)| l.clone())
    }

    /// The same path for the given locale: the old prefix (if any) is
    /// replaced by the locale's prefix, or removed for an unprefixed locale.
    ///
    /// Anything that is not an absolute path (a full URL, a relative path,
    /// `mailto:`…) is returned unchanged.
    ///
    /// ```
    /// # use codon::path::Prefixes;
    /// let paths = Prefixes::new([("de", "at")]);
    /// assert_eq!(paths.localize("/eshop?typ=buk", "de"), "/at/eshop?typ=buk");
    /// assert_eq!(paths.localize("/at/eshop", "de"), "/at/eshop");
    /// assert_eq!(paths.localize("/at/eshop", "cs"), "/eshop");
    /// assert_eq!(paths.localize("https://example.com/", "de"), "https://example.com/");
    /// ```
    #[must_use]
    pub fn localize<'a, Q>(&self, path: &'a str, locale: Q) -> Cow<'a, str>
    where
        L: PartialEq<Q>,
    {
        if !path.starts_with('/') {
            return Cow::Borrowed(path);
        }
        let base = self.strip(path);
        let Some(prefix) = self.prefix_for(locale) else {
            return base;
        };
        // "/" and "/?x" become "/at" and "/at?x", not "/at/".
        let rest = match base.strip_prefix('/') {
            Some(after) if after.is_empty() || after.starts_with(['?', '#']) => after,
            _ => &base,
        };
        Cow::Owned(format!("/{prefix}{rest}"))
    }

    /// The path without its locale prefix. Always starts with `/`.
    ///
    /// ```
    /// # use codon::path::Prefixes;
    /// let paths = Prefixes::new([("de", "at")]);
    /// assert_eq!(paths.strip("/at?x=1"), "/?x=1");
    /// assert_eq!(paths.strip("/at#top"), "/#top");
    /// assert_eq!(paths.strip("/kontakt"), "/kontakt");
    /// ```
    #[must_use]
    pub fn strip<'a>(&self, path: &'a str) -> Cow<'a, str> {
        let stripped = match self.split_prefix(path) {
            Some((_, rest)) => rest,
            None => path,
        };
        if stripped.starts_with('/') {
            Cow::Borrowed(stripped)
        } else {
            Cow::Owned(format!("/{stripped}"))
        }
    }

    /// The locale whose prefix the path starts with, and the path without it.
    /// `None` when the path has no locale prefix (the default locale).
    ///
    /// ```
    /// # use codon::path::Prefixes;
    /// let paths = Prefixes::new([("de", "at")]);
    /// assert_eq!(paths.detect("/at"), Some(("de", "/".into())));
    /// assert_eq!(paths.detect("/at?x=1"), Some(("de", "/?x=1".into())));
    /// assert_eq!(paths.detect("/atlas"), None);
    /// ```
    #[must_use]
    pub fn detect<'a>(&self, path: &'a str) -> Option<(L, Cow<'a, str>)> {
        let (locale, _) = self.split_prefix(path)?;
        Some((locale.clone(), self.strip(path)))
    }

    /// Finds the entry whose prefix starts the path, returning it and the
    /// rest of the path after the prefix.
    fn split_prefix<'a>(&self, path: &'a str) -> Option<(&L, &'a str)> {
        let after_slash = path.strip_prefix('/')?;
        self.entries.iter().find_map(|(locale, prefix)| {
            let rest = after_slash.strip_prefix(prefix)?;
            (rest.is_empty() || rest.starts_with(['/', '?', '#'])).then_some((locale, rest))
        })
    }
}

impl<L: Clone + 'static> Default for Prefixes<L> {
    fn default() -> Self {
        Self::from_static(&[])
    }
}

fn is_valid_prefix(prefix: &str) -> bool {
    !prefix.is_empty() && !prefix.contains(['/', '?', '#'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Prefixes {
        Prefixes::new([("de", "at")])
    }

    #[test]
    fn localize_matches_the_ruby_helper() {
        let p = paths();
        let cases = [
            ("/kontakt#cenik", "de", "/at/kontakt#cenik"),
            ("/", "de", "/at"),
            ("/?x=1", "de", "/at?x=1"),
            ("/#top", "de", "/at#top"),
            ("/eshop/tramy?a=1#b", "de", "/at/eshop/tramy?a=1#b"),
            ("/at/kontakt", "de", "/at/kontakt"),
            ("/at", "de", "/at"),
            ("/at", "cs", "/"),
            ("/at?x=1", "cs", "/?x=1"),
            ("/kontakt", "cs", "/kontakt"),
            ("/atlas", "de", "/at/atlas"),
            ("/atlas", "cs", "/atlas"),
            ("kontakt", "de", "kontakt"),
            ("", "de", ""),
            ("https://piladerflice.cz/", "de", "https://piladerflice.cz/"),
            ("/kontakt", "fr", "/kontakt"),
        ];
        for (path, locale, expected) in cases {
            assert_eq!(p.localize(path, locale), expected, "{path} {locale}");
        }
    }

    #[test]
    fn strip_and_detect() {
        let p = paths();
        assert_eq!(p.strip("/at/eshop/tramy"), "/eshop/tramy");
        assert_eq!(p.strip("/at"), "/");
        assert_eq!(p.strip("/atlas"), "/atlas");
        assert_eq!(p.strip("/at/"), "/");
        assert_eq!(p.strip("/"), "/");
        assert_eq!(p.strip("/at#x"), "/#x");
        assert_eq!(p.detect("/at/kontakt"), Some(("de", "/kontakt".into())));
        assert_eq!(p.detect("/at#x"), Some(("de", "/#x".into())));
        assert_eq!(p.detect("/kontakt"), None);
        assert_eq!(p.detect("/"), None);
        assert_eq!(p.detect("at/kontakt"), None);
        assert_eq!(p.detect("/AT/kontakt"), None);
    }

    #[test]
    fn borrows_when_possible() {
        let p = paths();
        assert!(matches!(p.strip("/at/x"), Cow::Borrowed("/x")));
        assert!(matches!(p.localize("/x", "cs"), Cow::Borrowed("/x")));
    }

    #[test]
    fn several_prefixes() {
        let p = Prefixes::new([("de", "at"), ("de-CH", "ch"), ("en", "a")]);
        assert_eq!(p.detect("/ch/x"), Some(("de-CH", "/x".into())));
        assert_eq!(p.detect("/a/x"), Some(("en", "/x".into())));
        assert_eq!(p.detect("/at/x"), Some(("de", "/x".into())));
        assert_eq!(p.localize("/ch/x", "de"), "/at/x");
        assert_eq!(p.locale_for("ch"), Some("de-CH"));
        assert_eq!(p.locale_for("x"), None);
        assert_eq!(p.prefix_for("cs"), None);
        assert_eq!(p.entries().len(), 3);
    }

    #[test]
    fn owned_locale_lookup() {
        let p = paths();
        let code = alloc::string::String::from("de");
        assert_eq!(p.localize("/", code.as_str()), "/at");
    }

    #[test]
    fn enum_locale() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        enum Locale {
            Cs,
            De,
        }
        const PATHS: Prefixes<Locale> = Prefixes::from_static(&[(Locale::De, "at")]);
        assert_eq!(PATHS.localize("/kontakt", Locale::De), "/at/kontakt");
        assert_eq!(PATHS.localize("/at/kontakt", Locale::Cs), "/kontakt");
        assert_eq!(PATHS.detect("/at"), Some((Locale::De, "/".into())));
        assert_eq!(Prefixes::<Locale>::default().detect("/at"), None);
    }

    #[test]
    #[should_panic(expected = "invalid URL prefix")]
    fn rejects_bad_prefix() {
        let _ = Prefixes::new([("de", "at/x")]);
    }
}
