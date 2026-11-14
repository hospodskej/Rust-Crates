//! The generated code checked into `tests/generated/` must match what the
//! generator produces now. `tests/generated_code.rs` compiles and runs it.
//!
//! After changing the generator or the fixtures, update the files with
//! `CODON_BLESS=1 cargo test -p codon --all-features --test snapshot`.

#![cfg(feature = "build")]

use std::path::Path;

fn check(name: &str, build: &codon::Build) {
    let generated = build.render().unwrap_or_else(|e| panic!("{e}"));
    let path = Path::new("tests/generated").join(format!("{name}.rs"));
    if std::env::var_os("CODON_BLESS").is_some() {
        std::fs::write(&path, generated.code()).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    // Compare ignoring whitespace, so that a new prettyplease version does
    // not fail the test.
    let normalize = |s: &str| s.split_whitespace().collect::<String>();
    assert!(
        normalize(&committed) == normalize(generated.code()),
        "{} is out of date; rerun with CODON_BLESS=1",
        path.display()
    );
}

#[test]
fn features() {
    let mut build = codon::Build::new();
    build
        .locale("cs", "tests/fixtures/features/cs.yml")
        .locale("de", "tests/fixtures/features/de.yml")
        .url_prefix("de", "at");
    let generated = build.render().unwrap();
    assert_eq!(generated.warnings(), [] as [String; 0]);
    check("features", &build);
}

#[test]
fn piladerflice() {
    let mut build = codon::Build::new();
    build
        .locale("cs", "tests/fixtures/cs.yml")
        .locale("de", "tests/fixtures/de.yml")
        .url_prefix("de", "at")
        .fallback_to_default(true);
    let generated = build.render().unwrap();
    // The German file lacks the Rails validation messages and model
    // attribute names, which the app only shows in Czech.
    assert_eq!(generated.warnings().len(), 34);
    assert!(
        generated.warnings()[0]
            .starts_with("tests/fixtures/de.yml: missing key `support.array.words_connector`")
    );
    check("piladerflice", &build);
}

#[test]
fn piladerflice_without_fallback_lists_every_missing_key() {
    let err = codon::Build::new()
        .locale("cs", "tests/fixtures/cs.yml")
        .locale("de", "tests/fixtures/de.yml")
        .render()
        .unwrap_err();
    let codon::Error::Translations(problems) = &err else {
        panic!("{err}")
    };
    assert_eq!(problems.len(), 34);
    assert!(problems.iter().all(|p| p.file == "tests/fixtures/de.yml"));
    assert_eq!(problems[0].key, "support.array.words_connector");
    let message = err.to_string();
    assert!(message.starts_with("codon: 34 problem(s) in the translations:"));
    assert!(message.contains(
        "\n  tests/fixtures/de.yml: `activerecord.attributes.pricelist_item.price`: missing (it is in tests/fixtures/cs.yml)"
    ));
}
