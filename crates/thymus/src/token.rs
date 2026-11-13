//! Password reset tokens bound to the current password hash.

use core::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use secrecy::{ExposeSecret, SecretSlice};
use sha2::Sha256;

use crate::PasswordHash;

type HmacSha256 = Hmac<Sha256>;

const VERSION: u8 = 1;
const FINGERPRINT_LEN: usize = 16;
const MAC_LEN: usize = 32;
/// version, user id, expiry, fingerprint.
const PAYLOAD_LEN: usize = 1 + 8 + 8 + FINGERPRINT_LEN;
const TOKEN_LEN: usize = PAYLOAD_LEN + MAC_LEN;

/// Domain separation, so the key's two uses can't stand in for each other.
const MAC_CONTEXT: &[u8] = b"thymus.reset-token.v1\0";
const FINGERPRINT_CONTEXT: &[u8] = b"thymus.password-fingerprint.v1\0";

/// Issues and checks password reset tokens, the port of Rails'
/// `password_reset_token` and `find_by_password_reset_token`.
///
/// A token names a user, expires after the [TTL](Self::ttl) (15 minutes by
/// default) and works only while the user's password hash is the one it was
/// issued for: once the password changes (new hash, new salt), every older
/// token is [`TokenError::Stale`]. So a used link can't be used again.
///
/// The token is URL-safe base64 without padding, 87 characters:
///
/// | bytes | field |
/// |---|---|
/// | 1 | format version, `1` |
/// | 8 | user id, `i64` big-endian |
/// | 8 | expiry, Unix seconds, `u64` big-endian |
/// | 16 | password fingerprint: HMAC-SHA256 of the PHC string, cut to 16 bytes |
/// | 32 | HMAC-SHA256 over the 33 bytes above |
///
/// The user id and the expiry are readable by whoever has the link; the hash
/// itself is never in it. Both HMACs use the secret key, each with its own
/// context prefix, and are compared in constant time.
///
/// ```
/// use std::time::{Duration, SystemTime};
/// use thymus::{Password, PasswordHash, ResetTokens, TokenError};
///
/// let tokens = ResetTokens::new(b"a secret key of at least 32 bytes, from config");
/// let hash = PasswordHash::new(&Password::new("old password")?)?;
/// let now = SystemTime::now();
///
/// let token = tokens.issue(42, &hash, now);
/// assert_eq!(tokens.verify(&token, now, |_id| Some(hash.clone())), Ok(42));
///
/// // 16 minutes later:
/// let later = now + Duration::from_secs(16 * 60);
/// assert_eq!(tokens.verify(&token, later, |_id| Some(hash.clone())), Err(TokenError::Expired));
///
/// // After the password changed:
/// let new_hash = PasswordHash::new(&Password::new("new password")?)?;
/// assert_eq!(tokens.verify(&token, now, |_id| Some(new_hash)), Err(TokenError::Stale));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct ResetTokens {
    key: SecretSlice<u8>,
    ttl: Duration,
}

impl ResetTokens {
    /// How long a token works unless [`ttl`](Self::ttl) says otherwise:
    /// 15 minutes, as in Rails.
    pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
    /// The shortest secret key accepted, in bytes.
    pub const MIN_KEY_LEN: usize = 32;

    /// Tokens signed with `secret_key`, which should be at least 32 random
    /// bytes kept in the app's configuration (like Rails' `secret_key_base`).
    /// Changing the key invalidates every token issued with the old one.
    ///
    /// # Panics
    ///
    /// If `secret_key` is shorter than [`Self::MIN_KEY_LEN`] bytes.
    #[must_use]
    pub fn new(secret_key: &[u8]) -> Self {
        assert!(
            secret_key.len() >= Self::MIN_KEY_LEN,
            "the reset token key must have at least {} bytes",
            Self::MIN_KEY_LEN
        );
        Self {
            key: SecretSlice::from(secret_key.to_vec()),
            ttl: Self::DEFAULT_TTL,
        }
    }

    /// Sets how long new tokens work. A token keeps the expiry it was
    /// issued with, so this doesn't change tokens already sent.
    #[must_use]
    pub fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// A token for `user_id` that works until `now` plus the TTL, and only
    /// while the user's hash is still `password_hash`.
    #[must_use]
    pub fn issue(&self, user_id: i64, password_hash: &PasswordHash, now: SystemTime) -> String {
        let expires = unix_secs(now).saturating_add(self.ttl.as_secs());
        let mut token = [0u8; TOKEN_LEN];
        token[0] = VERSION;
        token[1..9].copy_from_slice(&user_id.to_be_bytes());
        token[9..17].copy_from_slice(&expires.to_be_bytes());
        let fingerprint = self.fingerprint(password_hash).finalize().into_bytes();
        token[17..PAYLOAD_LEN].copy_from_slice(&fingerprint[..FINGERPRINT_LEN]);
        let mac = self.mac(&token[..PAYLOAD_LEN]).finalize().into_bytes();
        token[PAYLOAD_LEN..].copy_from_slice(&mac);
        URL_SAFE_NO_PAD.encode(token)
    }

    /// Checks a token in one go: format, signature and expiry, then looks
    /// up the user's current hash with `lookup` and checks that the token
    /// was issued for it. Returns the user id.
    ///
    /// `lookup` runs only for a genuine, unexpired token. When the lookup
    /// is `async` (a database query), use [`decode`](Self::decode) and
    /// [`confirm`](Self::confirm) instead.
    ///
    /// # Errors
    ///
    /// See [`TokenError`]. A user that `lookup` doesn't find gives
    /// [`TokenError::UnknownUser`].
    pub fn verify(
        &self,
        token: &str,
        now: SystemTime,
        lookup: impl FnOnce(i64) -> Option<PasswordHash>,
    ) -> Result<i64, TokenError> {
        let claim = self.decode(token, now)?;
        let hash = lookup(claim.user_id).ok_or(TokenError::UnknownUser)?;
        self.confirm(&claim, &hash)
    }

    /// The first half of [`verify`](Self::verify): checks the format, the
    /// signature and the expiry, and tells which user the token is for.
    /// Load that user's hash, then call [`confirm`](Self::confirm).
    ///
    /// # Errors
    ///
    /// [`TokenError::Malformed`], [`TokenError::BadSignature`] or
    /// [`TokenError::Expired`].
    pub fn decode(&self, token: &str, now: SystemTime) -> Result<ResetClaim, TokenError> {
        // Base64 of TOKEN_LEN bytes; checked first so no long input is decoded.
        if token.len() != (TOKEN_LEN * 4).div_ceil(3) {
            return Err(TokenError::Malformed);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| TokenError::Malformed)?;
        let bytes: [u8; TOKEN_LEN] = bytes.try_into().map_err(|_| TokenError::Malformed)?;
        if bytes[0] != VERSION {
            return Err(TokenError::Malformed);
        }
        self.mac(&bytes[..PAYLOAD_LEN])
            .verify_slice(&bytes[PAYLOAD_LEN..])
            .map_err(|_| TokenError::BadSignature)?;

        let (mut user_id, mut expires) = ([0u8; 8], [0u8; 8]);
        user_id.copy_from_slice(&bytes[1..9]);
        expires.copy_from_slice(&bytes[9..17]);
        let (user_id, expires) = (i64::from_be_bytes(user_id), u64::from_be_bytes(expires));
        if unix_secs(now) >= expires {
            return Err(TokenError::Expired);
        }
        let mut fingerprint = [0u8; FINGERPRINT_LEN];
        fingerprint.copy_from_slice(&bytes[17..PAYLOAD_LEN]);
        Ok(ResetClaim {
            user_id,
            fingerprint,
        })
    }

    /// The second half of [`verify`](Self::verify): whether the token was
    /// issued for `current`, the user's hash as it is now. Returns the user
    /// id.
    ///
    /// # Errors
    ///
    /// [`TokenError::Stale`] if the password changed since the token was
    /// issued.
    pub fn confirm(&self, claim: &ResetClaim, current: &PasswordHash) -> Result<i64, TokenError> {
        self.fingerprint(current)
            .verify_truncated_left(&claim.fingerprint)
            .map(|()| claim.user_id)
            .map_err(|_| TokenError::Stale)
    }

    fn keyed(&self, context: &[u8]) -> HmacSha256 {
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(self.key.expose_secret())
            .expect("HMAC takes keys of any length");
        mac.update(context);
        mac
    }

    fn mac(&self, payload: &[u8]) -> HmacSha256 {
        let mut mac = self.keyed(MAC_CONTEXT);
        mac.update(payload);
        mac
    }

    fn fingerprint(&self, hash: &PasswordHash) -> HmacSha256 {
        let mut mac = self.keyed(FINGERPRINT_CONTEXT);
        mac.update(hash.as_str().as_bytes());
        mac
    }
}

impl fmt::Debug for ResetTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResetTokens")
            .field("key", &"[REDACTED]")
            .field("ttl", &self.ttl)
            .finish()
    }
}

fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// A genuine, unexpired token from [`ResetTokens::decode`], not yet checked
/// against the user's current password hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetClaim {
    user_id: i64,
    fingerprint: [u8; FINGERPRINT_LEN],
}

impl ResetClaim {
    /// The user the token was issued for.
    #[must_use]
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
}

/// Why a reset token was refused.
///
/// Tell the visitor the same thing for all of them ("the link is invalid or
/// has expired"); the variants are for logs and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenError {
    /// Not a token: wrong length, not base64url, or an unknown version.
    Malformed,
    /// The signature doesn't match: the token was altered, or signed with
    /// another key.
    BadSignature,
    /// The token is past its expiry.
    Expired,
    /// The password changed after the token was issued.
    Stale,
    /// The user the token names no longer exists.
    UnknownUser,
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed reset token",
            Self::BadSignature => "reset token signature does not match",
            Self::Expired => "reset token has expired",
            Self::Stale => "reset token was issued for a previous password",
            Self::UnknownUser => "reset token names an unknown user",
        })
    }
}

impl std::error::Error for TokenError {}
