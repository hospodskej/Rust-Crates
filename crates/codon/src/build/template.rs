//! Rails interpolation: `%{name}` placeholders.

use std::collections::BTreeSet;

/// A piece of a translation string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    /// Literal text.
    Text(String),
    /// A `%{name}` placeholder.
    Arg(String),
}

fn is_placeholder_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Splits a string into text and placeholders.
///
/// Like Rails, `%%` stands for a literal `%` (only in strings that are
/// interpolated, `unescape_percent`). Anything else that is not a valid
/// `%{name}` is kept as text.
pub(crate) fn parse(text: &str, unescape_percent: bool) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('%') {
        literal.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if unescape_percent && after.starts_with('%') {
            literal.push('%');
            rest = &after[1..];
        } else if let Some(name) = after
            .strip_prefix('{')
            .and_then(|a| a.split_once('}'))
            .map(|(name, _)| name)
            .filter(|name| is_placeholder_name(name))
        {
            if !literal.is_empty() {
                segments.push(Segment::Text(std::mem::take(&mut literal)));
            }
            segments.push(Segment::Arg(name.to_owned()));
            rest = &after[name.len() + 2..];
        } else {
            literal.push('%');
            rest = after;
        }
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        segments.push(Segment::Text(literal));
    }
    segments
}

/// The placeholder names used in a string.
pub(crate) fn placeholders(text: &str) -> BTreeSet<String> {
    parse(text, true)
        .into_iter()
        .filter_map(|s| match s {
            Segment::Arg(name) => Some(name),
            Segment::Text(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::Segment::{Arg, Text};
    use super::*;

    #[test]
    fn parses_placeholders() {
        assert_eq!(
            parse("Hi %{name}, %{count}%!", true),
            [
                Text("Hi ".into()),
                Arg("name".into()),
                Text(", ".into()),
                Arg("count".into()),
                Text("%!".into())
            ]
        );
        assert_eq!(parse("%{a}%{b}", true), [Arg("a".into()), Arg("b".into())]);
        assert_eq!(parse("", true), []);
    }

    #[test]
    fn keeps_invalid_placeholders() {
        assert_eq!(
            parse("%{} %{a b} %{x", true),
            [Text("%{} %{a b} %{x".into())]
        );
        assert_eq!(parse("100 %", true), [Text("100 %".into())]);
    }

    #[test]
    fn percent_escape() {
        assert_eq!(parse("%%{a} %%", true), [Text("%{a} %".into())]);
        assert_eq!(parse("%%{a}", false), [Text("%".into()), Arg("a".into())]);
    }

    #[test]
    fn placeholder_set() {
        let set = placeholders("%{b} %{a} %{b} %%{c}");
        assert_eq!(set.into_iter().collect::<Vec<_>>(), ["a", "b"]);
    }
}
