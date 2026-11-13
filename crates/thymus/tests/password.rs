//! Hashing, verifying, the stored format and the password policy.

use thymus::{Error, Password, PasswordHash, PolicyError, authenticate};

fn hash(password: &str) -> PasswordHash {
    PasswordHash::new(&Password::new(password).unwrap()).unwrap()
}

#[test]
fn hashes_and_verifies() {
    let stored = hash("correct horse");
    assert!(stored.verify("correct horse"));
    assert!(!stored.verify("correct horsE"));
    assert!(!stored.verify(""));
    assert!(
        stored
            .as_str()
            .starts_with("$argon2id$v=19$m=19456,t=2,p=1$")
    );
}

#[test]
fn every_hash_has_its_own_salt() {
    assert_ne!(hash("correct horse"), hash("correct horse"));
}

#[test]
fn round_trips_through_the_phc_string() {
    let stored = hash("correct horse");
    let parsed: PasswordHash = stored.as_str().parse().unwrap();
    assert_eq!(parsed, stored);
    let from_string = PasswordHash::try_from(stored.clone().into_string()).unwrap();
    assert!(from_string.verify("correct horse"));
}

#[test]
fn rejects_what_is_not_an_argon2id_phc_string() {
    for bad in [
        "",
        "correct horse",
        "$2a$12$R9h/cIPz0gi.URNNX3kh2OPST9/PgBkqquzi.Ss7KIUgO2t0jWMUW",
        "$argon2i$v=19$m=19456,t=2,p=1$ZNwGlRD4Vo9+9R2DG31SPw$xeXIgp1GH/iuC0dcbYzTzwgqQJU0x6pn5oEWENpAQ5o",
        "$argon2id$v=16$m=19456,t=2,p=1$ZNwGlRD4Vo9+9R2DG31SPw$xeXIgp1GH/iuC0dcbYzTzwgqQJU0x6pn5oEWENpAQ5o",
        "$argon2id$v=19$m=19456,t=2,p=1$ZNwGlRD4Vo9+9R2DG31SPw",
        "$argon2id$v=19$m=1,t=2,p=1$ZNwGlRD4Vo9+9R2DG31SPw$xeXIgp1GH/iuC0dcbYzTzwgqQJU0x6pn5oEWENpAQ5o",
    ] {
        assert_eq!(
            bad.parse::<PasswordHash>(),
            Err(Error::InvalidFormat),
            "{bad:?}"
        );
        assert_eq!(
            PasswordHash::try_from(bad.to_owned()),
            Err(Error::InvalidFormat)
        );
    }
}

#[test]
fn debug_output_hides_secrets() {
    let password = Password::new("correct horse").unwrap();
    assert!(!format!("{password:?}").contains("horse"));
    let stored = PasswordHash::new(&password).unwrap();
    assert!(!format!("{stored:?}").contains(&stored.as_str()[20..]));
}

#[test]
fn policy_counts_characters_not_bytes() {
    assert_eq!(
        Password::new("").unwrap_err(),
        PolicyError::TooShort { min: 8 }
    );
    assert_eq!(
        Password::new("1234567").unwrap_err(),
        PolicyError::TooShort { min: 8 }
    );
    assert!(Password::new("12345678").is_ok());
    // 7 characters but 14 bytes: too short, as in Rails.
    assert_eq!(
        Password::new("žžžžžžž").unwrap_err(),
        PolicyError::TooShort { min: 8 }
    );
    assert!(Password::new("žžžžžžžž").is_ok());
    assert_eq!(
        Password::new("12345678").unwrap().expose_secret(),
        "12345678"
    );
}

#[test]
fn policy_has_a_maximum_in_bytes() {
    assert!(Password::new("a".repeat(1024)).is_ok());
    assert_eq!(
        Password::new("a".repeat(1025)).unwrap_err(),
        PolicyError::TooLong { max: 1024 }
    );
    // 600 characters, 1200 bytes.
    assert_eq!(
        Password::new("ž".repeat(600)).unwrap_err(),
        PolicyError::TooLong { max: 1024 }
    );
}

#[test]
fn verify_refuses_overlong_input() {
    assert!(!hash("correct horse").verify(&"a".repeat(1025)));
}

#[test]
fn authenticate_checks_the_found_hash() {
    let stored = hash("correct horse");
    assert!(authenticate(Some(&stored), "correct horse"));
    assert!(!authenticate(Some(&stored), "wrong"));
}

#[test]
fn authenticate_without_a_user_is_always_false() {
    assert!(!authenticate(None, "correct horse"));
    assert!(!authenticate(None, ""));
    assert!(!authenticate(None, "thymus-dummy-password-never-matches"));
}
