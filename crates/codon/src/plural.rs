//! CLDR cardinal plural rules for integer counts.
//!
//! The generated plural functions take `count: i64` and pick a form with
//! [`PluralRule::category`]. The rules follow the
//! [CLDR plural rules](https://www.unicode.org/cldr/charts/latest/supplemental/language_plural_rules.html)
//! restricted to integers (forms used only for decimals, such as Czech
//! `many`, never come up), and are kept in a small hand-written table instead
//! of pulling in ICU data.
//!
//! ```
//! use codon::plural::{Category, PluralRule};
//!
//! let cs = PluralRule::for_locale("cs").unwrap();
//! assert_eq!(cs.category(1), Category::One);
//! assert_eq!(cs.category(3), Category::Few);
//! assert_eq!(cs.category(5), Category::Other);
//!
//! let de = PluralRule::for_locale("de-AT").unwrap();
//! assert_eq!(de.category(1), Category::One);
//! assert_eq!(de.category(3), Category::Other);
//! ```

use core::fmt;
use core::str::FromStr;

/// A CLDR plural category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// `zero`
    Zero,
    /// `one`
    One,
    /// `two`
    Two,
    /// `few`
    Few,
    /// `many`
    Many,
    /// `other`, the form every plural translation must have.
    Other,
}

impl Category {
    /// All categories, in CLDR order.
    pub const ALL: [Self; 6] = [
        Self::Zero,
        Self::One,
        Self::Two,
        Self::Few,
        Self::Many,
        Self::Other,
    ];

    /// The CLDR name of the category, also used as the YAML key
    /// (`one`, `few`, …).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error returned when a string is not a plural category name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownCategory;

impl fmt::Display for UnknownCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a plural category (zero, one, two, few, many, other)")
    }
}

impl core::error::Error for UnknownCategory {}

impl FromStr for Category {
    type Err = UnknownCategory;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|c| c.as_str() == s)
            .ok_or(UnknownCategory)
    }
}

/// A plural rule shared by a group of languages.
///
/// Every rule is evaluated on the absolute value of the count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PluralRule {
    /// No plural forms: `other` only (Chinese, Japanese, Korean, Vietnamese,
    /// Thai, Indonesian, …).
    Invariant,
    /// `one` for 1 (English, German, Dutch, the Scandinavian languages,
    /// Italian, Spanish, Hungarian, Greek, Finnish, Estonian, Bulgarian, …).
    OneOther,
    /// `one` for 0 and 1 (French, Portuguese).
    ZeroOneAreOne,
    /// Czech and Slovak: `one` for 1, `few` for 2–4.
    Czech,
    /// Polish: `one`, `few` for 2–4, 22–24, …, `many` for the rest.
    Polish,
    /// Russian, Ukrainian, Belarusian: `one` for 1, 21, 31, …; `few` for
    /// 2–4, 22–24, …; `many` for the rest.
    EastSlavic,
    /// Croatian, Serbian, Bosnian: like [`EastSlavic`](Self::EastSlavic) but
    /// with `other` instead of `many`.
    SerboCroatian,
    /// Slovenian: `one` for …01, `two` for …02, `few` for …03–04.
    Slovenian,
    /// Lithuanian: `one` for 1, 21, …; `few` for 2–9, 22–29, ….
    Lithuanian,
    /// Latvian: `zero` for 0, 10–20, 30, …; `one` for 1, 21, ….
    Latvian,
    /// Romanian: `one` for 1, `few` for 0 and 2–19, 101–119, ….
    Romanian,
    /// Arabic: `zero`, `one`, `two`, `few` for 3–10, `many` for 11–99.
    Arabic,
    /// Hebrew: `one` for 1, `two` for 2.
    Hebrew,
    /// Irish: `one`, `two`, `few` for 3–6, `many` for 7–10.
    Irish,
}

impl PluralRule {
    /// The rule for a locale code such as `cs`, `de-AT` or `pt_BR`, looked up
    /// by its language subtag. `None` for languages not in the table.
    #[must_use]
    pub fn for_locale(locale: &str) -> Option<Self> {
        let language = locale.split(['-', '_']).next().unwrap_or_default();
        let mut lower = [0u8; 8];
        let bytes = language.as_bytes();
        if bytes.is_empty() || bytes.len() > lower.len() {
            return None;
        }
        for (dst, src) in lower.iter_mut().zip(bytes) {
            *dst = src.to_ascii_lowercase();
        }
        let rule = match &lower[..bytes.len()] {
            b"ja" | b"zh" | b"ko" | b"vi" | b"th" | b"id" | b"ms" | b"lo" | b"my" | b"km"
            | b"jv" | b"yo" | b"ig" => Self::Invariant,
            b"en" | b"de" | b"nl" | b"sv" | b"da" | b"nb" | b"nn" | b"no" | b"fi" | b"et"
            | b"it" | b"es" | b"ca" | b"gl" | b"eu" | b"hu" | b"el" | b"bg" | b"tr" | b"az"
            | b"ka" | b"kk" | b"uz" | b"sq" | b"af" | b"fy" | b"lb" | b"rm" | b"sw" | b"ur"
            | b"ta" | b"te" | b"ml" | b"mn" | b"ne" | b"ps" | b"so" | b"eo" | b"fo" | b"ia"
            | b"ky" | b"tk" | b"ug" => Self::OneOther,
            b"fr" | b"pt" | b"hy" | b"hi" | b"bn" | b"fa" | b"am" | b"zu" | b"kn" | b"gu" => {
                Self::ZeroOneAreOne
            }
            b"cs" | b"sk" => Self::Czech,
            b"pl" => Self::Polish,
            b"ru" | b"uk" | b"be" => Self::EastSlavic,
            b"hr" | b"sr" | b"bs" | b"sh" => Self::SerboCroatian,
            b"sl" => Self::Slovenian,
            b"lt" => Self::Lithuanian,
            b"lv" => Self::Latvian,
            b"ro" | b"mo" => Self::Romanian,
            b"ar" => Self::Arabic,
            b"he" | b"iw" => Self::Hebrew,
            b"ga" => Self::Irish,
            _ => return None,
        };
        Some(rule)
    }

    /// The categories that [`category`](Self::category) can return, in CLDR
    /// order. Categories used only for decimals are not included.
    ///
    /// ```
    /// use codon::plural::{Category, PluralRule};
    ///
    /// assert_eq!(PluralRule::Czech.categories(), [Category::One, Category::Few, Category::Other]);
    /// ```
    #[must_use]
    pub const fn categories(self) -> &'static [Category] {
        use Category::{Few, Many, One, Other, Two, Zero};
        match self {
            Self::Invariant => &[Other],
            Self::OneOther | Self::ZeroOneAreOne => &[One, Other],
            Self::Czech | Self::SerboCroatian | Self::Lithuanian | Self::Romanian => {
                &[One, Few, Other]
            }
            Self::Polish | Self::EastSlavic => &[One, Few, Many],
            Self::Slovenian => &[One, Two, Few, Other],
            Self::Latvian => &[Zero, One, Other],
            Self::Arabic => &[Zero, One, Two, Few, Many, Other],
            Self::Hebrew => &[One, Two, Other],
            Self::Irish => &[One, Two, Few, Many, Other],
        }
    }

    /// The plural category for an integer count.
    ///
    /// ```
    /// use codon::plural::{Category, PluralRule};
    ///
    /// assert_eq!(PluralRule::Polish.category(22), Category::Few);
    /// assert_eq!(PluralRule::Polish.category(25), Category::Many);
    /// assert_eq!(PluralRule::Czech.category(-2), Category::Few);
    /// ```
    #[must_use]
    // One arm per rule keeps the table easy to compare with CLDR.
    #[allow(clippy::too_many_lines)]
    pub const fn category(self, count: i64) -> Category {
        let n = count.unsigned_abs();
        let n10 = n % 10;
        let n100 = n % 100;
        match self {
            Self::Invariant => Category::Other,
            Self::OneOther => {
                if n == 1 {
                    Category::One
                } else {
                    Category::Other
                }
            }
            Self::ZeroOneAreOne => {
                if n <= 1 {
                    Category::One
                } else {
                    Category::Other
                }
            }
            Self::Czech => match n {
                1 => Category::One,
                2..=4 => Category::Few,
                _ => Category::Other,
            },
            Self::Polish => {
                if n == 1 {
                    Category::One
                } else if matches!(n10, 2..=4) && !matches!(n100, 12..=14) {
                    Category::Few
                } else {
                    Category::Many
                }
            }
            Self::EastSlavic => {
                if n10 == 1 && n100 != 11 {
                    Category::One
                } else if matches!(n10, 2..=4) && !matches!(n100, 12..=14) {
                    Category::Few
                } else {
                    Category::Many
                }
            }
            Self::SerboCroatian => {
                if n10 == 1 && n100 != 11 {
                    Category::One
                } else if matches!(n10, 2..=4) && !matches!(n100, 12..=14) {
                    Category::Few
                } else {
                    Category::Other
                }
            }
            Self::Slovenian => match n100 {
                1 => Category::One,
                2 => Category::Two,
                3 | 4 => Category::Few,
                _ => Category::Other,
            },
            Self::Lithuanian => {
                if matches!(n100, 11..=19) {
                    Category::Other
                } else if n10 == 1 {
                    Category::One
                } else if n10 >= 2 {
                    Category::Few
                } else {
                    Category::Other
                }
            }
            Self::Latvian => {
                if n10 == 0 || matches!(n100, 11..=19) {
                    Category::Zero
                } else if n10 == 1 {
                    Category::One
                } else {
                    Category::Other
                }
            }
            Self::Romanian => {
                if n == 1 {
                    Category::One
                } else if n == 0 || matches!(n100, 1..=19) {
                    Category::Few
                } else {
                    Category::Other
                }
            }
            Self::Arabic => match n {
                0 => Category::Zero,
                1 => Category::One,
                2 => Category::Two,
                _ => match n100 {
                    3..=10 => Category::Few,
                    11..=99 => Category::Many,
                    _ => Category::Other,
                },
            },
            Self::Hebrew => match n {
                1 => Category::One,
                2 => Category::Two,
                _ => Category::Other,
            },
            Self::Irish => match n {
                1 => Category::One,
                2 => Category::Two,
                3..=6 => Category::Few,
                7..=10 => Category::Many,
                _ => Category::Other,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Category::{Few, Many, One, Other, Two, Zero};
    use super::*;

    fn categories(rule: PluralRule, counts: &[i64]) -> alloc::vec::Vec<Category> {
        counts.iter().map(|&n| rule.category(n)).collect()
    }

    #[test]
    fn czech() {
        assert_eq!(
            categories(PluralRule::Czech, &[0, 1, 2, 3, 4, 5, 11, 21, 22, 100]),
            [Other, One, Few, Few, Few, Other, Other, Other, Other, Other]
        );
        assert_eq!(PluralRule::for_locale("sk"), Some(PluralRule::Czech));
    }

    #[test]
    fn german_and_english() {
        for code in ["de", "de-AT", "de_CH", "en", "EN-gb"] {
            let rule = PluralRule::for_locale(code).unwrap();
            assert_eq!(rule, PluralRule::OneOther, "{code}");
            assert_eq!(
                categories(rule, &[0, 1, 2, 11, 21, -1]),
                [Other, One, Other, Other, Other, One]
            );
        }
    }

    #[test]
    fn french() {
        assert_eq!(
            categories(PluralRule::for_locale("fr").unwrap(), &[0, 1, 2]),
            [One, One, Other]
        );
    }

    #[test]
    fn polish() {
        assert_eq!(
            categories(PluralRule::Polish, &[0, 1, 2, 4, 5, 12, 14, 22, 25, 101]),
            [Many, One, Few, Few, Many, Many, Many, Few, Many, Many]
        );
    }

    #[test]
    fn russian() {
        assert_eq!(
            categories(PluralRule::EastSlavic, &[1, 2, 5, 11, 12, 21, 22, 25, 111]),
            [One, Few, Many, Many, Many, One, Few, Many, Many]
        );
    }

    #[test]
    fn croatian() {
        assert_eq!(
            categories(PluralRule::SerboCroatian, &[1, 3, 5, 11, 21]),
            [One, Few, Other, Other, One]
        );
    }

    #[test]
    fn slovenian() {
        assert_eq!(
            categories(PluralRule::Slovenian, &[1, 2, 3, 5, 101, 102, 104]),
            [One, Two, Few, Other, One, Two, Few]
        );
    }

    #[test]
    fn baltic() {
        assert_eq!(
            categories(PluralRule::Lithuanian, &[0, 1, 2, 9, 10, 11, 19, 21]),
            [Other, One, Few, Few, Other, Other, Other, One]
        );
        assert_eq!(
            categories(PluralRule::Latvian, &[0, 1, 2, 10, 11, 21, 30]),
            [Zero, One, Other, Zero, Zero, One, Zero]
        );
    }

    #[test]
    fn romanian() {
        assert_eq!(
            categories(PluralRule::Romanian, &[0, 1, 2, 19, 20, 101, 120]),
            [Few, One, Few, Few, Other, Few, Other]
        );
    }

    #[test]
    fn semitic_and_irish() {
        assert_eq!(
            categories(PluralRule::Arabic, &[0, 1, 2, 3, 10, 11, 99, 100, 103]),
            [Zero, One, Two, Few, Few, Many, Many, Other, Few]
        );
        assert_eq!(
            categories(PluralRule::Hebrew, &[1, 2, 3]),
            [One, Two, Other]
        );
        assert_eq!(
            categories(PluralRule::Irish, &[1, 2, 3, 7, 11]),
            [One, Two, Few, Many, Other]
        );
    }

    #[test]
    fn invariant_and_unknown() {
        assert_eq!(
            categories(PluralRule::for_locale("ja").unwrap(), &[0, 1, 2]),
            [Other, Other, Other]
        );
        assert_eq!(PluralRule::for_locale("xx"), None);
        assert_eq!(PluralRule::for_locale(""), None);
        assert_eq!(PluralRule::for_locale("toolonglanguage"), None);
    }

    #[test]
    fn extreme_counts() {
        assert_eq!(PluralRule::Czech.category(i64::MIN), Other);
        assert_eq!(PluralRule::OneOther.category(i64::MAX), Other);
    }

    #[test]
    fn categories_match_the_rules() {
        let rules = [
            PluralRule::Invariant,
            PluralRule::OneOther,
            PluralRule::ZeroOneAreOne,
            PluralRule::Czech,
            PluralRule::Polish,
            PluralRule::EastSlavic,
            PluralRule::SerboCroatian,
            PluralRule::Slovenian,
            PluralRule::Lithuanian,
            PluralRule::Latvian,
            PluralRule::Romanian,
            PluralRule::Arabic,
            PluralRule::Hebrew,
            PluralRule::Irish,
        ];
        for rule in rules {
            let mut seen: alloc::vec::Vec<Category> = (0..1000).map(|n| rule.category(n)).collect();
            seen.sort();
            seen.dedup();
            assert_eq!(seen, rule.categories(), "{rule:?}");
        }
    }

    #[test]
    fn category_names() {
        for c in Category::ALL {
            assert_eq!(c.as_str().parse::<Category>(), Ok(c));
        }
        assert_eq!("plenty".parse::<Category>(), Err(UnknownCategory));
    }
}
