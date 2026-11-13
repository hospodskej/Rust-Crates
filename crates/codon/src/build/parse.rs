//! Reading one locale file into a flat list of translations.

use std::fmt::Write as _;

use yaml_rust2::{Yaml, YamlLoader};

use super::Problem;
use crate::plural::Category;

/// A translation value of one locale.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// A plain string (non-string scalars are rendered as strings).
    Text(String),
    /// A YAML sequence of scalars.
    List(Vec<String>),
    /// Plural forms, in CLDR order.
    Plural(Vec<(Category, String)>),
}

impl Value {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Text(_) => "a string",
            Self::List(_) => "a list",
            Self::Plural(_) => "a plural",
        }
    }
}

/// One translation: its key path (the last segment is the logical key, e.g.
/// the stem of `count_one`/`count_other`) and value.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub(crate) path: Vec<String>,
    pub(crate) value: Value,
}

impl Entry {
    pub(crate) fn dotted(&self) -> String {
        self.path.join(".")
    }
}

/// Parses a locale file whose top-level key is the locale code.
pub(crate) fn parse_locale(
    code: &str,
    file: &str,
    source: &str,
    problems: &mut Vec<Problem>,
) -> Result<Vec<Entry>, String> {
    let docs = YamlLoader::load_from_str(source).map_err(|e| e.to_string())?;
    let root = match docs.as_slice() {
        [] => return Err("the file is empty".to_owned()),
        [doc] => doc,
        _ => return Err("the file contains more than one YAML document".to_owned()),
    };
    let Yaml::Hash(top) = root else {
        return Err(format!(
            "expected a mapping with the top-level key `{code}:`"
        ));
    };
    let Some(tree) = top
        .iter()
        .find_map(|(k, v)| (scalar_to_string(k).as_deref() == Some(code)).then_some(v))
    else {
        let found: Vec<String> = top.keys().filter_map(scalar_to_string).collect();
        return Err(format!(
            "expected the top-level key `{code}:` (found: {})",
            found.join(", ")
        ));
    };
    let mut entries = Vec::new();
    match tree {
        Yaml::Hash(_) => flatten(tree, &mut Vec::new(), file, &mut entries, problems),
        Yaml::Null => {}
        _ => return Err(format!("`{code}:` must be a mapping of translation keys")),
    }
    Ok(entries)
}

/// The text of a scalar, `None` for anything else.
fn scalar_to_string(yaml: &Yaml) -> Option<String> {
    match yaml {
        Yaml::String(s) | Yaml::Real(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

fn problem(file: &str, path: &[String], message: impl Into<String>) -> Problem {
    Problem {
        file: file.to_owned(),
        key: path.join("."),
        message: message.into(),
    }
}

/// A mapping whose keys are all plural categories (`one:`, `other:`, …).
fn plural_map(map: &yaml_rust2::yaml::Hash) -> bool {
    !map.is_empty()
        && map
            .keys()
            .all(|k| scalar_to_string(k).is_some_and(|k| k.parse::<Category>().is_ok()))
}

fn flatten(
    node: &Yaml,
    path: &mut Vec<String>,
    file: &str,
    out: &mut Vec<Entry>,
    problems: &mut Vec<Problem>,
) {
    let Yaml::Hash(map) = node else {
        unreachable!("flatten is only called on mappings")
    };
    // Scalars of this mapping, collected first so that `x_one`/`x_other`
    // siblings can be merged into one plural.
    let mut scalars: Vec<(String, String, usize)> = Vec::new();
    for (key, value) in map {
        let Some(key) = scalar_to_string(key) else {
            problems.push(problem(file, path, "a mapping key is not a scalar"));
            continue;
        };
        path.push(key.clone());
        if key == "<<" {
            problems.push(problem(
                file,
                path,
                "YAML merge keys (`<<: *anchor`) are not supported",
            ));
        } else {
            match value {
                Yaml::Hash(inner) if plural_map(inner) => {
                    let forms = inner
                        .iter()
                        .filter_map(|(k, v)| {
                            let category = scalar_to_string(k)?.parse().ok()?;
                            if let Some(text) = scalar_to_string(v) {
                                Some((category, text))
                            } else {
                                problems.push(problem(
                                    file,
                                    path,
                                    format!("the plural form `{category}` is not a string"),
                                ));
                                None
                            }
                        })
                        .collect();
                    push_plural(path.clone(), forms, file, out, problems);
                }
                Yaml::Hash(_) => flatten(value, path, file, out, problems),
                Yaml::Array(items) => {
                    let mut list = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        match scalar_to_string(item) {
                            Some(text) => list.push(text),
                            None => problems.push(problem(
                                file,
                                path,
                                format!("list item {i} is not a string"),
                            )),
                        }
                    }
                    out.push(Entry {
                        path: path.clone(),
                        value: Value::List(list),
                    });
                }
                // `key:` with no value is a missing translation, like in Rails.
                Yaml::Null => {}
                Yaml::Alias(_) | Yaml::BadValue => {
                    problems.push(problem(file, path, "unsupported YAML value"));
                }
                _ => {
                    let text = scalar_to_string(value).unwrap_or_default();
                    scalars.push((key, text, out.len()));
                }
            }
        }
        path.pop();
    }
    merge_suffixed_plurals(scalars, path, file, out, problems);
}

/// Splits `count_one` into (`count`, `One`).
fn plural_suffix(key: &str) -> Option<(&str, Category)> {
    let (stem, suffix) = key.rsplit_once('_')?;
    let category = suffix.parse().ok()?;
    (!stem.is_empty()).then_some((stem, category))
}

/// Turns sibling keys `x_one`, `x_other`, … into one plural entry `x`, and
/// every other scalar into a text entry, keeping the YAML order (each entry
/// is inserted where its first key was).
fn merge_suffixed_plurals(
    scalars: Vec<(String, String, usize)>,
    path: &mut Vec<String>,
    file: &str,
    out: &mut Vec<Entry>,
    problems: &mut Vec<Problem>,
) {
    let mut stems: Vec<(&str, Vec<usize>)> = Vec::new();
    for (i, (key, _, _)) in scalars.iter().enumerate() {
        if let Some((stem, _)) = plural_suffix(key) {
            match stems.iter_mut().find(|(s, _)| *s == stem) {
                Some((_, members)) => members.push(i),
                None => stems.push((stem, vec![i])),
            }
        }
    }
    let mut taken = vec![false; scalars.len()];
    // ((insert position, index among the scalars), entry); positions refer
    // to `out` before insertion.
    let mut new_entries: Vec<((usize, usize), Entry)> = Vec::new();
    for (stem, members) in stems {
        if members.len() < 2 {
            continue;
        }
        let forms: Vec<(Category, String)> = members
            .iter()
            .map(|&i| {
                let (key, text, _) = &scalars[i];
                (
                    plural_suffix(key).expect("grouped by suffix").1,
                    text.clone(),
                )
            })
            .collect();
        for &i in &members {
            taken[i] = true;
        }
        path.push(stem.to_owned());
        if scalars.iter().any(|(k, _, _)| k == stem) {
            problems.push(problem(
                file,
                path,
                format!(
                    "`{stem}` exists both as a plain key and as plural forms (`{stem}_one`, …)"
                ),
            ));
        }
        let position = (scalars[members[0]].2, members[0]);
        let mut plural_out = Vec::new();
        push_plural(path.clone(), forms, file, &mut plural_out, problems);
        new_entries.extend(plural_out.into_iter().map(|e| (position, e)));
        path.pop();
    }
    for (i, (key, text, position)) in scalars.into_iter().enumerate() {
        if !taken[i] {
            path.push(key);
            new_entries.push((
                (position, i),
                Entry {
                    path: path.clone(),
                    value: Value::Text(text),
                },
            ));
            path.pop();
        }
    }
    // Insert from the back so that earlier positions stay valid.
    new_entries.sort_by_key(|(position, _)| *position);
    for ((position, _), entry) in new_entries.into_iter().rev() {
        out.insert(position, entry);
    }
}

fn push_plural(
    path: Vec<String>,
    mut forms: Vec<(Category, String)>,
    file: &str,
    out: &mut Vec<Entry>,
    problems: &mut Vec<Problem>,
) {
    forms.sort_by_key(|(c, _)| *c);
    if !forms.iter().any(|(c, _)| *c == Category::Other) {
        let mut message = String::from("plural without an `other` form (it has: ");
        for (i, (c, _)) in forms.iter().enumerate() {
            if i > 0 {
                message.push_str(", ");
            }
            let _ = write!(message, "{c}");
        }
        message.push(')');
        problems.push(problem(file, &path, message));
        return;
    }
    out.push(Entry {
        path,
        value: Value::Plural(forms),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> (Vec<Entry>, Vec<Problem>) {
        let mut problems = Vec::new();
        let entries = parse_locale("cs", "cs.yml", yaml, &mut problems).unwrap();
        (entries, problems)
    }

    fn keys(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(Entry::dotted).collect()
    }

    #[test]
    fn flattens_in_yaml_order() {
        let (entries, problems) =
            parse("cs:\n  b: x\n  a:\n    z: 1\n    y: true\n  c: [p, 2.5]\n  n:\n");
        assert!(problems.is_empty());
        assert_eq!(keys(&entries), ["b", "a.z", "a.y", "c"]);
        assert_eq!(entries[1].value, Value::Text("1".into()));
        assert_eq!(entries[2].value, Value::Text("true".into()));
        assert_eq!(
            entries[3].value,
            Value::List(vec!["p".into(), "2.5".into()])
        );
    }

    #[test]
    fn plurals_both_styles() {
        let (entries, problems) = parse(
            "cs:\n  first: f\n  n_one: a\n  mid: m\n  n_other: b\n  m:\n    other: o\n    one: x\n  last: l\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys(&entries), ["first", "n", "mid", "m", "last"]);
        assert_eq!(
            entries[1].value,
            Value::Plural(vec![
                (Category::One, "a".into()),
                (Category::Other, "b".into())
            ])
        );
        assert_eq!(
            entries[3].value,
            Value::Plural(vec![
                (Category::One, "x".into()),
                (Category::Other, "o".into())
            ])
        );
    }

    #[test]
    fn single_suffix_is_a_plain_key() {
        let (entries, problems) = parse("cs:\n  payment_other: x\n  step_one: y\n");
        assert!(problems.is_empty());
        assert_eq!(keys(&entries), ["payment_other", "step_one"]);
    }

    #[test]
    fn plural_without_other() {
        let (_, problems) = parse("cs:\n  a:\n    one: x\n    few: y\n  b_one: x\n  b_few: y\n");
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0].key, "a");
        assert!(problems[0].message.contains("`other`"));
        assert_eq!(problems[1].key, "b");
    }

    #[test]
    fn bad_files() {
        let mut p = Vec::new();
        assert!(
            parse_locale("cs", "f", "de:\n  a: b\n", &mut p)
                .unwrap_err()
                .contains("`cs:`")
        );
        assert!(parse_locale("cs", "f", "", &mut p).is_err());
        assert!(parse_locale("cs", "f", "cs: [1]", &mut p).is_err());
        assert!(parse_locale("cs", "f", "cs:\n  a: \"x\n", &mut p).is_err());
    }

    #[test]
    fn anchors_and_merge_keys() {
        let (entries, problems) = parse("cs:\n  a: &x Ahoj\n  b: *x\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(entries[1].value, Value::Text("Ahoj".into()));
        let (_, problems) = parse("cs:\n  base: &b\n    x: 1\n  c:\n    <<: *b\n");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("merge keys"));
    }

    #[test]
    fn nested_lists_are_reported() {
        let (_, problems) = parse("cs:\n  a:\n    - x\n    - [y]\n");
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].key, "a");
    }
}
