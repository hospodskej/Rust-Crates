//! Compiles the code generated from the fixtures (kept up to date by
//! `tests/snapshot.rs`) and checks how it behaves.

mod features {
    include!("generated/features.rs");
}

mod piladerflice {
    include!("generated/piladerflice.rs");
}

use codon::{Escaped, Html};
use features::{Locale, t};

#[test]
fn locale_enum() {
    assert_eq!(Locale::default(), Locale::Cs);
    assert_eq!(Locale::DEFAULT, Locale::Cs);
    assert_eq!(Locale::ALL, [Locale::Cs, Locale::De]);
    assert_eq!(Locale::De.as_str(), "de");
    assert_eq!(Locale::De.to_string(), "de");
    assert_eq!("de".parse::<Locale>(), Ok(Locale::De));
    assert_eq!("CS".parse::<Locale>(), Ok(Locale::Cs));
    assert_eq!(
        "fr".parse::<Locale>().unwrap_err().to_string(),
        "unknown locale `fr`"
    );
    assert_eq!(Locale::Cs.plural_rule(), codon::plural::PluralRule::Czech);
}

#[test]
fn url_prefixes() {
    assert_eq!(Locale::Cs.url_prefix(), None);
    assert_eq!(Locale::De.url_prefix(), Some("at"));
    assert_eq!(Locale::from_url_prefix("at"), Some(Locale::De));
    assert_eq!(Locale::from_url_prefix("de"), None);
    assert_eq!(
        Locale::PATHS.localize("/kontakt#cenik", Locale::De),
        "/at/kontakt#cenik"
    );
    assert_eq!(Locale::PATHS.localize("/at?x=1", Locale::Cs), "/?x=1");
    assert_eq!(
        Locale::PATHS.detect("/at/eshop"),
        Some((Locale::De, "/eshop".into()))
    );
    assert_eq!(Locale::PATHS.detect("/atlas"), None);
}

#[test]
fn plain_strings() {
    assert_eq!(t::site_name(Locale::Cs), "Pila Derflice");
    assert_eq!(t::nav::domu(Locale::De), "Startseite");
    assert_eq!(t::keywords::r#type(Locale::Cs), "Typ");
    assert_eq!(t::keywords::crate_(Locale::De), "Kiste");
    assert_eq!(t::keywords::n404(Locale::Cs), "Nenalezeno");
    assert_eq!(t::keywords::r#match(Locale::De), "Treffer");
    assert_eq!(t::keywords::first_step(Locale::Cs), "První krok");
    assert_eq!(t::scalars::year(Locale::Cs), "2024");
    assert_eq!(t::scalars::price(Locale::Cs), "12.5");
    assert_eq!(t::scalars::enabled(Locale::De), "false");
    assert_eq!(t::not_interpolated(Locale::Cs), "100 %% {jistota}");
    assert_eq!(t::order::payment_other(Locale::Cs), "Jiná platba");
}

#[test]
fn interpolation() {
    assert_eq!(t::greeting(Locale::Cs, "Jano"), "Dobrý den, Jano!");
    assert_eq!(t::vat(Locale::De, 20), "MwSt. 20 %");
    assert_eq!(
        t::escapes(Locale::Cs, 5),
        "5 % a {složené} závorky",
        "%% is a literal % and braces are kept"
    );
    assert_eq!(t::product_kind(Locale::De, "Buche"), "Art: Buche");
    assert_eq!(t::locale_name(Locale::Cs, Locale::De), "Jazyk: de");
    // Parameters are in alphabetical order: date, number.
    assert_eq!(
        t::order::summary(Locale::Cs, "1. 5.", 42),
        "Objednávka 42 ze dne 1. 5."
    );
}

#[test]
fn plurals() {
    let cs: Vec<String> = [0, 1, 2, 4, 5, 22, -3]
        .into_iter()
        .map(|n| t::order::items(Locale::Cs, n))
        .collect();
    assert_eq!(
        cs,
        [
            "Žádná položka",
            "1 položka",
            "2 položky",
            "4 položky",
            "5 položek",
            "22 položek",
            "-3 položky"
        ]
    );
    assert_eq!(t::order::items(Locale::De, 0), "Kein Artikel");
    assert_eq!(t::order::items(Locale::De, 1), "1 Artikel");
    assert_eq!(t::order::items(Locale::De, 3), "3 Artikel");

    assert_eq!(t::order::pieces(Locale::Cs, 1), "kus");
    assert_eq!(t::order::pieces(Locale::Cs, 3), "kusy");
    assert_eq!(t::order::pieces(Locale::Cs, 0), "kusů");
    assert_eq!(t::order::pieces(Locale::De, 2), "Stück");

    assert_eq!(
        t::order::in_cart(Locale::Cs, 1, "Hlavní"),
        "1 položka v košíku Hlavní"
    );
    assert_eq!(
        t::order::in_cart(Locale::De, 7, "Haupt"),
        "7 Artikel im Warenkorb Haupt"
    );
}

#[test]
fn lists() {
    assert_eq!(
        t::steps(Locale::De),
        ["Produkt wählen", "In den Warenkorb", "Bestellen"]
    );
    assert_eq!(t::highlights(Locale::Cs)[1].as_str(), "<i>Levně</i>");
}

#[test]
fn html() {
    assert_eq!(
        t::intro(Locale::Cs),
        Html::from_static("<strong>Kvalitní</strong> dřevo")
    );
    let link = Html::from_trusted(r#"<a href="/privacy">zásadami</a>"#);
    assert_eq!(
        t::privacy(Locale::Cs, &link, "<Jan & spol.>").as_str(),
        r#"Souhlasím se <a href="/privacy">zásadami</a> a se zpracováním údajů pro &lt;Jan &amp; spol.&gt;."#
    );
    assert_eq!(
        t::privacy(Locale::De, Html::escape("<x>"), Escaped('"')).as_str(),
        "Ich stimme der &lt;x&gt; und der Datenverarbeitung für &quot; zu."
    );
    assert_eq!(t::basket(Locale::Cs, 3).as_str(), "<b>3</b> položky");
    assert_eq!(t::basket(Locale::De, 1).as_str(), "<b>1</b> Artikel");
    assert_eq!(
        t::terms(Locale::Cs, "Štěpán <Merta>").as_str(),
        "<h2>Obchodní podmínky</h2>\n<p>Prodávající: Štěpán &lt;Merta&gt;</p>\n"
    );
}

#[test]
fn lookup() {
    assert_eq!(features::lookup(Locale::De, "nav.domu"), Some("Startseite"));
    assert_eq!(
        features::lookup(Locale::Cs, "keywords.404"),
        Some("Nenalezeno")
    );
    assert_eq!(features::lookup(Locale::Cs, "keywords.type"), Some("Typ"));
    assert_eq!(features::lookup(Locale::Cs, "greeting"), None);
    assert_eq!(features::lookup(Locale::Cs, "intro_html"), None);
    assert_eq!(features::lookup(Locale::Cs, "nav"), None);
    assert_eq!(features::lookup(Locale::Cs, "missing.key"), None);
}

#[cfg(feature = "serde")]
#[test]
fn serde() {
    assert_eq!(serde_json::to_string(&Locale::De).unwrap(), r#""de""#);
    assert_eq!(
        serde_json::from_str::<Locale>(r#""cs""#).unwrap(),
        Locale::Cs
    );
    assert!(serde_json::from_str::<Locale>(r#""fr""#).is_err());
}

#[test]
fn piladerflice_translations() {
    use piladerflice::{Locale, lookup, t};

    assert_eq!(t::site::name(Locale::Cs), "Pila Derflice");
    assert_eq!(
        t::auth::reset::new_intro(Locale::Cs, "jan@example.cz"),
        "Nastavte nové heslo pro účet jan@example.cz."
    );
    assert_eq!(t::auth::order_item_count(Locale::Cs, 1), "1 položka");
    assert_eq!(t::auth::order_item_count(Locale::Cs, 7), "7 položek");
    assert_eq!(t::auth::order_item_count(Locale::De, 7), "7 Artikel");
    let link = Html::from_trusted("<a href=\"/privacy\">x</a>");
    assert!(
        t::auth::register_privacy(Locale::Cs, &link)
            .as_str()
            .ends_with("dle <a href=\"/privacy\">x</a>.")
    );
    // Missing in de.yml: falls back to the Czech text.
    assert_eq!(
        t::support::array::two_words_connector(Locale::De),
        t::support::array::two_words_connector(Locale::Cs)
    );
    assert!(
        t::legal::terms_content(Locale::Cs, 15)
            .as_str()
            .contains("15 Kč")
    );
    assert_eq!(
        lookup(Locale::De, "site.name"),
        Some(t::site::name(Locale::De))
    );
}
