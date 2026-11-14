//! Turning YAML keys into Rust identifiers.

/// Keywords that can be used as raw identifiers (`r#type`).
const RAW_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "static", "struct", "trait", "true", "try", "type", "unsafe", "use", "where",
    "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof",
    "unsized", "virtual", "yield",
];

/// Keywords that cannot be raw identifiers.
const NON_RAW_KEYWORDS: &[&str] = &["crate", "self", "super", "Self"];

/// An identifier: its name and whether it must be written as `r#name`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Name {
    pub(crate) text: String,
    pub(crate) raw: bool,
}

impl Name {
    pub(crate) fn ident(&self) -> proc_macro2::Ident {
        let span = proc_macro2::Span::call_site();
        if self.raw {
            proc_macro2::Ident::new_raw(&self.text, span)
        } else {
            proc_macro2::Ident::new(&self.text, span)
        }
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.raw {
            f.write_str("r#")?;
        }
        f.write_str(&self.text)
    }
}

/// Replaces everything but ASCII letters, digits and `_` with `_`, and
/// prefixes a leading digit with `n` (`404` → `n404`; an underscore would
/// trip `clippy::used_underscore_items` in the calling code).
fn sanitize(key: &str) -> String {
    let mut s: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s == "_" {
        s.insert(0, '_');
        if s == "_" {
            s.push('_');
        }
    }
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, 'n');
    }
    s
}

/// A function or module name for a YAML key: keywords become raw
/// identifiers (`r#type`), except those that cannot (`crate_`).
pub(crate) fn item_name(key: &str) -> Name {
    let text = sanitize(key);
    if NON_RAW_KEYWORDS.contains(&text.as_str()) {
        Name {
            text: text + "_",
            raw: false,
        }
    } else {
        let raw = RAW_KEYWORDS.contains(&text.as_str());
        Name { text, raw }
    }
}

/// A parameter name for a placeholder: keywords get a `_` suffix (`type_`),
/// because raw identifiers cannot be used in `format!` strings.
pub(crate) fn param_name(placeholder: &str) -> String {
    let text = sanitize(placeholder);
    if RAW_KEYWORDS.contains(&text.as_str()) || NON_RAW_KEYWORDS.contains(&text.as_str()) {
        text + "_"
    } else {
        text
    }
}

/// `cs` → `Cs`, `de-AT` → `DeAt`, `pt_BR` → `PtBr`.
pub(crate) fn variant_name(code: &str) -> Option<String> {
    let mut out = String::new();
    for part in code.split(['-', '_']) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.extend(chars.map(|c| c.to_ascii_lowercase()));
        }
    }
    let valid = out.starts_with(|c: char| c.is_ascii_alphabetic())
        && out.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_names() {
        assert_eq!(item_name("domu").to_string(), "domu");
        assert_eq!(item_name("type").to_string(), "r#type");
        assert_eq!(item_name("crate").to_string(), "crate_");
        assert_eq!(item_name("Self").to_string(), "Self_");
        assert_eq!(item_name("404").to_string(), "n404");
        assert_eq!(item_name("1st-step").to_string(), "n1st_step");
        assert_eq!(item_name("domů").to_string(), "dom_");
        assert_eq!(item_name("_").to_string(), "__");
        assert_eq!(item_name("").to_string(), "__");
        assert_eq!(item_name("a b").to_string(), "a_b");
    }

    #[test]
    fn idents_are_valid() {
        for key in ["type", "crate", "404", "x", "gen", "async"] {
            let _ = item_name(key).ident();
        }
    }

    #[test]
    fn param_names() {
        assert_eq!(param_name("email"), "email");
        assert_eq!(param_name("type"), "type_");
        assert_eq!(param_name("self"), "self_");
        assert_eq!(param_name("1"), "n1");
    }

    #[test]
    fn variant_names() {
        assert_eq!(variant_name("cs").as_deref(), Some("Cs"));
        assert_eq!(variant_name("de-AT").as_deref(), Some("DeAt"));
        assert_eq!(variant_name("pt_BR").as_deref(), Some("PtBr"));
        assert_eq!(variant_name("zh-Hant").as_deref(), Some("ZhHant"));
        assert_eq!(variant_name("1x"), None);
        assert_eq!(variant_name(""), None);
        assert_eq!(variant_name("a.b"), None);
    }
}
