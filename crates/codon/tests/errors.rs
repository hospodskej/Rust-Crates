//! Build errors and warnings of the generator.

#![cfg(feature = "build")]

use codon::{Build, Error};

fn render(cs: &str, de: &str) -> Result<codon::Generated, Error> {
    Build::new()
        .locale_yaml("cs", "cs.yml", format!("cs:\n{cs}"))
        .locale_yaml("de", "de.yml", format!("de:\n{de}"))
        .render()
}

fn problems(cs: &str, de: &str) -> Vec<String> {
    match render(cs, de) {
        Err(Error::Translations(problems)) => problems.iter().map(ToString::to_string).collect(),
        other => panic!("expected translation problems, got {other:?}"),
    }
}

#[test]
fn missing_keys_are_all_listed() {
    let found = problems(
        "  a: A\n  b:\n    c: C\n    d: D\n",
        "  a: A\n  b:\n    x: X\n",
    );
    assert_eq!(
        found,
        [
            "de.yml: `b.c`: missing (it is in cs.yml)",
            "de.yml: `b.d`: missing (it is in cs.yml)"
        ]
    );
}

#[test]
fn fallback_turns_missing_keys_into_warnings() {
    let generated = Build::new()
        .locale_yaml("cs", "cs.yml", "cs:\n  a: A\n  b: B\n")
        .locale_yaml("de", "de.yml", "de:\n  a: DA\n  extra: E\n")
        .fallback_to_default(true)
        .render()
        .unwrap();
    assert_eq!(
        generated.warnings(),
        [
            "de.yml: missing key `b`, using the text from cs.yml",
            "de.yml: `extra` is not in cs.yml, ignored"
        ]
    );
    assert!(generated.code().contains("de: (missing, uses cs)"));
    assert!(!generated.code().contains("extra"));
}

#[test]
fn placeholders_must_match() {
    let found = problems(
        "  hi: \"Ahoj %{name}\"\n  n:\n    one: \"%{count} x %{a}\"\n    other: \"%{count}\"\n",
        "  hi: \"Hallo %{nme}\"\n  n:\n    one: \"eins\"\n    other: \"%{count} %{a}\"\n",
    );
    assert_eq!(
        found,
        ["de.yml: `hi`: uses the placeholders %{nme} but cs.yml uses %{name}"]
    );
}

#[test]
fn kinds_must_match() {
    let found = problems("  a: A\n  b: [x]\n", "  a: [x]\n  b: B\n");
    assert_eq!(
        found,
        [
            "de.yml: `a`: is a list here but a string in cs.yml",
            "de.yml: `b`: is a string here but a list in cs.yml"
        ]
    );
}

#[test]
fn plural_needs_other() {
    let yaml = "  n:\n    one: x\n    few: y\n";
    let found = problems(yaml, yaml);
    assert!(
        found
            .contains(&"cs.yml: `n`: plural without an `other` form (it has: one, few)".to_owned())
    );
}

#[test]
fn name_clashes() {
    let yaml = "  a-b: x\n  a_b: y\n  t_html: z\n  t: w\n  m-x:\n    x: 1\n  m_x:\n    y: 2\n";
    let found = problems(yaml, yaml);
    assert_eq!(
        found,
        [
            "cs.yml: `a_b`: becomes the function `a_b`, like `a-b`",
            "cs.yml: `t`: becomes the function `t`, like `t_html`",
            "cs.yml: `m_x`: becomes the module `m_x`, like `m-x`"
        ]
    );
}

#[test]
fn placeholder_clash() {
    let yaml = "  a: \"%{type} %{type_}\"\n";
    let found = problems(yaml, yaml);
    assert_eq!(
        found,
        ["cs.yml: `a`: the placeholders %{type} and %{type_} both become the parameter `type_`"]
    );
}

#[test]
fn plural_and_plain_key_clash() {
    let yaml = "  n: plain\n  n_one: one\n  n_other: other\n";
    let found = problems(yaml, yaml);
    assert!(found[0].contains("`n` exists both as a plain key and as plural forms"));
}

#[test]
fn unused_plural_form_warns() {
    let generated = render(
        "  n:\n    one: x\n    other: y\n",
        "  n:\n    one: x\n    few: z\n    other: y\n",
    )
    .unwrap();
    assert_eq!(
        generated.warnings(),
        ["de.yml: `n`: the plural form `few` is never used for `de`"]
    );
}

#[test]
fn yaml_and_config_errors() {
    let err = render("  a: \"unterminated\n", "  a: x\n").unwrap_err();
    assert!(
        matches!(&err, Error::Yaml { file, .. } if file == "cs.yml"),
        "{err}"
    );

    let err = Build::new()
        .locale_yaml("cs", "cs.yml", "de:\n  a: x\n")
        .render()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "codon: cs.yml: expected the top-level key `cs:` (found: de)"
    );

    let err = Build::new().render().unwrap_err();
    assert!(matches!(err, Error::Config(_)));

    let err = Build::new()
        .locale_yaml("cs", "cs.yml", "cs:\n  a: x\n")
        .url_prefix("cs", "cz")
        .render()
        .unwrap_err();
    assert!(err.to_string().contains("default locale"), "{err}");

    let err = Build::new()
        .locale_yaml("cs", "cs.yml", "cs:\n  a: x\n")
        .locale_yaml("de", "de.yml", "de:\n  a: x\n")
        .url_prefix("de", "a/t")
        .render()
        .unwrap_err();
    assert!(err.to_string().contains("one path segment"), "{err}");

    let err = Build::new()
        .locale("cs", "does/not/exist.yml")
        .render()
        .unwrap_err();
    assert!(
        err.to_string().starts_with("codon: does/not/exist.yml: "),
        "{err}"
    );
    assert!(std::error::Error::source(&err).is_some());

    let err = Build::new()
        .locale_yaml("cs", "cs.yml", "cs:\n  a: x\n")
        .module_name("type")
        .render()
        .unwrap_err();
    assert!(matches!(err, Error::Config(_)));
}

#[test]
fn module_name_and_unknown_language() {
    let generated = Build::new()
        .locale_yaml("xx", "xx.yml", "xx:\n  a: x\n")
        .module_name("tr")
        .render()
        .unwrap();
    assert!(generated.code().contains("pub mod tr {"));
    assert_eq!(
        generated.warnings(),
        ["no plural rule known for `xx`; using one/other (`one` for 1)"]
    );
}

#[test]
fn generate_writes_the_file() {
    let dir = std::env::temp_dir().join(format!("codon-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("out.rs");
    let path = Build::new()
        .locale_yaml("cs", "cs.yml", "cs:\n  a: x\n")
        .output(&out)
        .generate()
        .unwrap();
    assert_eq!(path, out);
    assert!(std::fs::read_to_string(&out).unwrap().contains("pub fn a("));
    std::fs::remove_dir_all(&dir).unwrap();
}
