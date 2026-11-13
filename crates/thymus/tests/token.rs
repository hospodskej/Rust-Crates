//! Password reset tokens.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use thymus::{Password, PasswordHash, ResetTokens, TokenError};

const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn hash(password: &str) -> PasswordHash {
    PasswordHash::new(&Password::new(password).unwrap()).unwrap()
}

#[test]
fn valid_token_gives_the_user_id() {
    let tokens = ResetTokens::new(KEY);
    let stored = hash("old password");
    for id in [1, 42, -7, i64::MAX, i64::MIN] {
        let token = tokens.issue(id, &stored, now());
        assert_eq!(token.len(), 87);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        let mut looked_up = None;
        let result = tokens.verify(&token, now(), |user| {
            looked_up = Some(user);
            Some(stored.clone())
        });
        assert_eq!(result, Ok(id));
        assert_eq!(looked_up, Some(id));
    }
}

#[test]
fn decode_and_confirm_in_two_steps() {
    let tokens = ResetTokens::new(KEY);
    let stored = hash("old password");
    let token = tokens.issue(42, &stored, now());
    let claim = tokens.decode(&token, now()).unwrap();
    assert_eq!(claim.user_id(), 42);
    assert_eq!(tokens.confirm(&claim, &stored), Ok(42));
}

#[test]
fn expires_after_15_minutes_by_default() {
    let tokens = ResetTokens::new(KEY);
    let stored = hash("old password");
    let token = tokens.issue(42, &stored, now());
    let at = |secs| now() + Duration::from_secs(secs);
    assert_eq!(
        tokens.verify(&token, at(15 * 60 - 1), |_| Some(stored.clone())),
        Ok(42)
    );
    assert_eq!(
        tokens.verify(&token, at(15 * 60), |_| Some(stored.clone())),
        Err(TokenError::Expired)
    );
    assert_eq!(tokens.decode(&token, at(86_400)), Err(TokenError::Expired));
}

#[test]
fn ttl_is_configurable() {
    let tokens = ResetTokens::new(KEY).ttl(Duration::from_secs(60));
    let stored = hash("old password");
    let token = tokens.issue(42, &stored, now());
    assert!(
        tokens
            .decode(&token, now() + Duration::from_secs(59))
            .is_ok()
    );
    assert_eq!(
        tokens.decode(&token, now() + Duration::from_secs(60)),
        Err(TokenError::Expired)
    );
}

#[test]
fn expired_lookup_is_never_run() {
    let tokens = ResetTokens::new(KEY);
    let token = tokens.issue(42, &hash("old password"), now());
    let result = tokens.verify(&token, now() + Duration::from_secs(3600), |_| {
        panic!("looked up the user of an expired token")
    });
    assert_eq!(result, Err(TokenError::Expired));
}

#[test]
fn stale_after_the_password_changes() {
    let tokens = ResetTokens::new(KEY);
    let old = hash("old password");
    let token = tokens.issue(42, &old, now());
    // Even the same password again gets a new salt, so a new hash.
    let new = hash("old password");
    assert_eq!(
        tokens.verify(&token, now(), |_| Some(new.clone())),
        Err(TokenError::Stale)
    );
    let claim = tokens.decode(&token, now()).unwrap();
    assert_eq!(tokens.confirm(&claim, &new), Err(TokenError::Stale));
}

#[test]
fn unknown_user() {
    let tokens = ResetTokens::new(KEY);
    let token = tokens.issue(42, &hash("old password"), now());
    assert_eq!(
        tokens.verify(&token, now(), |_| None),
        Err(TokenError::UnknownUser)
    );
}

#[test]
fn tampered_token_has_a_bad_signature() {
    let tokens = ResetTokens::new(KEY);
    let stored = hash("old password");
    let token = tokens.issue(42, &stored, now());
    let bytes = URL_SAFE_NO_PAD.decode(&token).unwrap();

    // Another user id, a later expiry, another fingerprint, a changed MAC.
    for (index, value) in [(8, 43), (16, 0xff), (20, 0), (64, 0)] {
        let mut altered = bytes.clone();
        altered[index] ^= value | 1;
        let altered = URL_SAFE_NO_PAD.encode(altered);
        assert_eq!(
            tokens.verify(&altered, now(), |_| Some(stored.clone())),
            Err(TokenError::BadSignature),
            "byte {index}"
        );
    }
}

#[test]
fn token_from_another_key_has_a_bad_signature() {
    let stored = hash("old password");
    let token = ResetTokens::new(b"another key, another app, 32+ bytes").issue(42, &stored, now());
    assert_eq!(
        ResetTokens::new(KEY).verify(&token, now(), |_| Some(stored.clone())),
        Err(TokenError::BadSignature)
    );
}

#[test]
fn malformed_input() {
    let tokens = ResetTokens::new(KEY);
    let token = tokens.issue(42, &hash("old password"), now());
    let mut wrong_version = URL_SAFE_NO_PAD.decode(&token).unwrap();
    wrong_version[0] = 2;
    for bad in [
        String::new(),
        "not a token".to_owned(),
        token[..86].to_owned(),
        format!("{token}A"),
        format!("{}=", &token[..86]),
        format!("{}+", &token[..86]),
        "é".repeat(43) + "a",
        "a".repeat(10_000),
        URL_SAFE_NO_PAD.encode(wrong_version),
    ] {
        assert_eq!(
            tokens.decode(&bad, now()),
            Err(TokenError::Malformed),
            "{bad:?}"
        );
    }
}

#[test]
#[should_panic(expected = "at least 32 bytes")]
fn short_key_is_refused() {
    let _ = ResetTokens::new(b"too short");
}

#[test]
fn debug_output_hides_the_key() {
    let debug = format!("{:?}", ResetTokens::new(KEY));
    assert!(!debug.contains("0123"), "{debug}");
}
