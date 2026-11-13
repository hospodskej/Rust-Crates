//! Merging the locales into one checked model of the translations.

use std::collections::{BTreeSet, HashMap};

use super::ident::{self, Name};
use super::parse::{self, Entry, Value};
use super::template;
use super::{Error, Problem};
use crate::plural::{Category, PluralRule};

pub(crate) struct Input {
    pub(crate) code: String,
    pub(crate) file: String,
    pub(crate) yaml: String,
}

pub(crate) struct Options {
    pub(crate) fallback: bool,
    pub(crate) url_prefixes: Vec<(String, String)>,
    pub(crate) module: String,
}

pub(crate) struct LocaleInfo {
    pub(crate) code: String,
    pub(crate) variant: String,
    pub(crate) file: String,
    pub(crate) rule: PluralRule,
    pub(crate) prefix: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Text,
    List,
    Plural,
}

pub(crate) struct Item {
    /// The YAML key path; the last segment is the logical key (with `_html`,
    /// without plural suffixes).
    pub(crate) path: Vec<String>,
    pub(crate) name: Name,
    pub(crate) html: bool,
    pub(crate) kind: Kind,
    /// Placeholders, sorted, without `count` for plurals.
    pub(crate) params: Vec<String>,
    /// One value per locale; missing ones hold the default locale's value.
    pub(crate) values: Vec<Value>,
    /// Per locale: whether the value is the default locale's fallback.
    pub(crate) fallback: Vec<bool>,
}

impl Item {
    pub(crate) fn dotted(&self) -> String {
        self.path.join(".")
    }

    /// Whether the strings are interpolated (have parameters).
    pub(crate) fn interpolated(&self) -> bool {
        self.kind == Kind::Plural || !self.params.is_empty()
    }
}

pub(crate) struct Model {
    pub(crate) locales: Vec<LocaleInfo>,
    pub(crate) items: Vec<Item>,
    pub(crate) module: Name,
}

pub(crate) fn build(
    inputs: &[Input],
    options: &Options,
    warnings: &mut Vec<String>,
) -> Result<Model, Error> {
    let locales = locale_infos(inputs, options, warnings)?;
    let module = module_name(&options.module)?;

    let mut problems = Vec::new();
    let mut parsed: Vec<Vec<Entry>> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let entries = parse::parse_locale(&input.code, &input.file, &input.yaml, &mut problems)
            .map_err(|message| Error::Yaml {
                file: input.file.clone(),
                message,
            })?;
        parsed.push(entries);
    }

    let default_file = &locales[0].file;
    let mut by_key: Vec<HashMap<String, Value>> = parsed
        .iter()
        .map(|entries| {
            entries
                .iter()
                .map(|e| (e.dotted(), e.value.clone()))
                .collect()
        })
        .collect();

    let mut items = Vec::new();
    for entry in &parsed[0] {
        let key = entry.dotted();
        let mut values = vec![entry.value.clone()];
        let mut fallback = vec![false];
        for (locale, map) in locales.iter().zip(&mut by_key).skip(1) {
            if let Some(value) = map.remove(&key) {
                if std::mem::discriminant(&value) != std::mem::discriminant(&entry.value) {
                    problems.push(Problem {
                        file: locale.file.clone(),
                        key: key.clone(),
                        message: format!(
                            "is {} here but {} in {default_file}",
                            value.kind(),
                            entry.value.kind()
                        ),
                    });
                }
                values.push(value);
                fallback.push(false);
            } else if options.fallback {
                warnings.push(format!(
                    "{}: missing key `{key}`, using the text from {default_file}",
                    locale.file
                ));
                values.push(entry.value.clone());
                fallback.push(true);
            } else {
                problems.push(Problem {
                    file: locale.file.clone(),
                    key: key.clone(),
                    message: format!("missing (it is in {default_file})"),
                });
                values.push(entry.value.clone());
                fallback.push(true);
            }
        }
        let params = check_placeholders(&key, &values, &locales, &mut problems);
        unused_plural_forms(&key, &values, &fallback, &locales, warnings);
        items.push(new_item(
            entry,
            values,
            fallback,
            params,
            default_file,
            &mut problems,
        ));
    }

    for (locale, map) in locales.iter().zip(&by_key).skip(1) {
        let mut extra: Vec<&String> = map.keys().collect();
        extra.sort();
        for key in extra {
            warnings.push(format!(
                "{}: `{key}` is not in {default_file}, ignored",
                locale.file
            ));
        }
    }

    check_names(&items, default_file, &mut problems);

    if problems.is_empty() {
        Ok(Model {
            locales,
            items,
            module,
        })
    } else {
        Err(Error::Translations(problems))
    }
}

fn new_item(
    entry: &Entry,
    values: Vec<Value>,
    fallback: Vec<bool>,
    params: Vec<String>,
    default_file: &str,
    problems: &mut Vec<Problem>,
) -> Item {
    let last = entry.path.last().expect("keys are never empty");
    let stem = last.strip_suffix("_html");
    if stem == Some("") {
        problems.push(Problem {
            file: default_file.to_owned(),
            key: entry.dotted(),
            message: "`_html` needs a name before it".to_owned(),
        });
    }
    let kind = match entry.value {
        Value::Text(_) => Kind::Text,
        Value::List(_) => Kind::List,
        Value::Plural(_) => Kind::Plural,
    };
    Item {
        path: entry.path.clone(),
        name: ident::item_name(stem.unwrap_or(last)),
        html: stem.is_some(),
        kind,
        params,
        values,
        fallback,
    }
}

fn module_name(name: &str) -> Result<Name, Error> {
    let ident = ident::item_name(name);
    if ident.text == name && !ident.raw {
        Ok(ident)
    } else {
        Err(Error::Config(format!(
            "module name `{name}` is not a plain Rust identifier"
        )))
    }
}

fn locale_infos(
    inputs: &[Input],
    options: &Options,
    warnings: &mut Vec<String>,
) -> Result<Vec<LocaleInfo>, Error> {
    if inputs.is_empty() {
        return Err(Error::Config(
            "no locales; add one with .locale()".to_owned(),
        ));
    }
    let mut locales: Vec<LocaleInfo> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let variant = ident::variant_name(&input.code).ok_or_else(|| {
            Error::Config(format!(
                "locale code `{}` cannot be turned into an enum variant",
                input.code
            ))
        })?;
        if let Some(other) = locales.iter().find(|l| l.variant == variant) {
            return Err(Error::Config(format!(
                "locales `{}` and `{}` are the same (both `Locale::{variant}`)",
                other.code, input.code
            )));
        }
        let rule = PluralRule::for_locale(&input.code).unwrap_or_else(|| {
            warnings.push(format!(
                "no plural rule known for `{}`; using one/other (`one` for 1)",
                input.code
            ));
            PluralRule::OneOther
        });
        locales.push(LocaleInfo {
            code: input.code.clone(),
            variant,
            file: input.file.clone(),
            rule,
            prefix: None,
        });
    }
    for (code, prefix) in &options.url_prefixes {
        if prefix.is_empty() || prefix.contains(['/', '?', '#']) {
            return Err(Error::Config(format!(
                "URL prefix `{prefix}` must be one path segment, without `/`, `?` or `#`"
            )));
        }
        if locales.iter().any(|l| l.prefix.as_deref() == Some(prefix)) {
            return Err(Error::Config(format!(
                "URL prefix `{prefix}` is used twice"
            )));
        }
        let index = locales
            .iter()
            .position(|l| &l.code == code)
            .ok_or_else(|| Error::Config(format!("URL prefix for unknown locale `{code}`")))?;
        if index == 0 {
            return Err(Error::Config(format!(
                "`{code}` is the default locale, which is served without a URL prefix"
            )));
        }
        locales[index].prefix = Some(prefix.clone());
    }
    Ok(locales)
}

/// Warns about plural forms that the locale's rule never selects.
fn unused_plural_forms(
    key: &str,
    values: &[Value],
    fallback: &[bool],
    locales: &[LocaleInfo],
    warnings: &mut Vec<String>,
) {
    for ((value, locale), fallback) in values.iter().zip(locales).zip(fallback) {
        let Value::Plural(forms) = value else {
            continue;
        };
        if *fallback {
            continue;
        }
        for (category, _) in forms {
            let used = *category == Category::Other
                || *category == Category::Zero
                || locale.rule.categories().contains(category);
            if !used {
                warnings.push(format!(
                    "{}: `{key}`: the plural form `{category}` is never used for `{}`",
                    locale.file, locale.code
                ));
            }
        }
    }
}

/// The placeholder set of one locale's value.
fn placeholder_set(value: &Value) -> BTreeSet<String> {
    match value {
        Value::Text(text) => template::placeholders(text),
        Value::List(_) => BTreeSet::new(),
        Value::Plural(forms) => {
            let mut set: BTreeSet<String> = forms
                .iter()
                .flat_map(|(_, text)| template::placeholders(text))
                .collect();
            set.remove("count");
            set
        }
    }
}

fn check_placeholders(
    key: &str,
    values: &[Value],
    locales: &[LocaleInfo],
    problems: &mut Vec<Problem>,
) -> Vec<String> {
    let expected = placeholder_set(&values[0]);
    for (value, locale) in values.iter().zip(locales).skip(1) {
        let found = placeholder_set(value);
        if found != expected {
            problems.push(Problem {
                file: locale.file.clone(),
                key: key.to_owned(),
                message: format!(
                    "uses the placeholders {} but {} uses {}",
                    list(&found),
                    locales[0].file,
                    list(&expected)
                ),
            });
        }
    }
    expected.into_iter().collect()
}

fn list(set: &BTreeSet<String>) -> String {
    if set.is_empty() {
        "(none)".to_owned()
    } else {
        set.iter()
            .map(|p| format!("%{{{p}}}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Reports keys that map to the same function or module name, and
/// placeholders that map to the same parameter name.
fn check_names(items: &[Item], file: &str, problems: &mut Vec<Problem>) {
    // (module path, kind, name) -> first dotted key
    let mut seen: HashMap<(Vec<Name>, bool, Name), String> = HashMap::new();
    for item in items {
        let key = item.dotted();
        let modules: Vec<Name> = item.path[..item.path.len() - 1]
            .iter()
            .map(|k| module_ident(k))
            .collect();
        for depth in 0..modules.len() {
            let parent = modules[..depth].to_vec();
            let module_key = item.path[..=depth].join(".");
            let slot = (parent, true, modules[depth].clone());
            if let Some(other) = seen.get(&slot) {
                if *other != module_key {
                    problems.push(Problem {
                        file: file.to_owned(),
                        key: module_key.clone(),
                        message: format!("becomes the module `{}`, like `{other}`", modules[depth]),
                    });
                }
            } else {
                seen.insert(slot, module_key);
            }
        }
        let slot = (modules, false, item.name.clone());
        if let Some(other) = seen.get(&slot) {
            problems.push(Problem {
                file: file.to_owned(),
                key: key.clone(),
                message: format!("becomes the function `{}`, like `{other}`", item.name),
            });
        } else {
            seen.insert(slot, key.clone());
        }

        let mut params: HashMap<String, &str> = HashMap::new();
        if item.kind == Kind::Plural {
            params.insert("count".to_owned(), "count");
        }
        for placeholder in &item.params {
            let param = param_ident(placeholder);
            if let Some(other) = params.insert(param.clone(), placeholder) {
                problems.push(Problem {
                    file: file.to_owned(),
                    key: key.clone(),
                    message: format!(
                        "the placeholders %{{{other}}} and %{{{placeholder}}} both become the parameter `{param}`"
                    ),
                });
            }
        }
    }
}

/// The module name for a YAML key; `Locale` is taken by the imported enum.
pub(crate) fn module_ident(key: &str) -> Name {
    let mut name = ident::item_name(key);
    if name.text == "Locale" {
        name.text.push('_');
    }
    name
}

/// The parameter name for a placeholder; `locale` and `html` are taken.
pub(crate) fn param_ident(placeholder: &str) -> String {
    let mut name = ident::param_name(placeholder);
    if name == "locale" || name == "html" {
        name.push('_');
    }
    name
}
