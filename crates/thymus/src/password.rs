//! Password policy, Argon2id hashing and the timing-safe sign-in check.

use core::fmt;
use core::str::FromStr;

use argon2::password_hash::phc;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use secrecy::{ExposeSecret, SecretString};

/// Argon2id memory cost in KiB (19 MiB), as the OWASP Password Storage Cheat
/// Sheet recommends.
pub const M_COST_KIB: u32 = 19 * 1024;
/// Argon2id iterations (OWASP: 2 with 19 MiB).
pub const T_COST: u32 = 2;
/// Argon2id lanes (OWASP: 1).
pub const P_COST: u32 = 1;

/// The hasher with the OWASP parameters. Verification always uses the
/// parameters stored in the hash itself, so raising these later keeps old
/// hashes working.
fn argon2() -> Argon2<'static> {
    let params = Params::new(M_COST_KIB, T_COST, P_COST, Some(Params::DEFAULT_OUTPUT_LEN))
        .expect("the OWASP parameters are valid Argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// A new password that meets the policy: at least [`Password::MIN_CHARS`]
/// characters and at most [`Password::MAX_BYTES`] bytes.
///
/// The text is kept in a [`SecretString`], which wipes it from memory when
/// dropped and never shows it in `Debug` output.
///
/// The policy is for *setting* a password (sign-up, reset, change). Signing
/// in checks whatever was typed with [`PasswordHash::verify`] or
/// [`authenticate`], without the policy, as Rails does.
///
/// ```
/// use thymus::{Password, PolicyError};
///
/// assert!(Password::new("correct horse").is_ok());
/// assert_eq!(Password::new("short").unwrap_err(), PolicyError::TooShort { min: 8 });
/// // Counted in characters, not bytes: eight Czech letters are enough.
/// assert!(Password::new("žluťoučk").is_ok());
/// ```
#[derive(Clone)]
pub struct Password(SecretString);

impl Password {
    /// The shortest allowed password, in characters (Unicode scalar values),
    /// as Rails' `validates :password, length: { minimum: 8 }` counts them.
    pub const MIN_CHARS: usize = 8;
    /// The longest allowed password, in bytes. Argon2 has no limit of its
    /// own (unlike bcrypt's 72 bytes); this one only keeps huge inputs out.
    pub const MAX_BYTES: usize = 1024;

    /// Checks `raw` against the policy.
    ///
    /// # Errors
    ///
    /// [`PolicyError::TooShort`] under [`Self::MIN_CHARS`] characters,
    /// [`PolicyError::TooLong`] over [`Self::MAX_BYTES`] bytes.
    pub fn new(raw: impl Into<String>) -> Result<Self, PolicyError> {
        let secret = SecretString::from(raw.into());
        let text = secret.expose_secret();
        if text.len() > Self::MAX_BYTES {
            return Err(PolicyError::TooLong {
                max: Self::MAX_BYTES,
            });
        }
        if text.chars().count() < Self::MIN_CHARS {
            return Err(PolicyError::TooShort {
                min: Self::MIN_CHARS,
            });
        }
        Ok(Self(secret))
    }

    /// The password text. Keep it out of logs.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password([REDACTED])")
    }
}

/// Why a new password was refused by [`Password::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyError {
    /// Fewer than `min` characters.
    TooShort {
        /// The minimum, in characters.
        min: usize,
    },
    /// More than `max` bytes.
    TooLong {
        /// The maximum, in bytes.
        max: usize,
    },
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { min } => write!(f, "the password must have at least {min} characters"),
            Self::TooLong { max } => write!(f, "the password must have at most {max} bytes"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// A failure to hash a password or to read a stored hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Hashing failed, e.g. the system's random number generator did not
    /// give a salt.
    Hashing,
    /// The text is not an Argon2id PHC string (`$argon2id$v=19$m=…,t=…,p=…$salt$hash`).
    InvalidFormat,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Hashing => "the password could not be hashed",
            Self::InvalidFormat => "not an Argon2id PHC string",
        })
    }
}

impl std::error::Error for Error {}

/// An Argon2id password hash, stored as a PHC string such as
/// `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`. The column for it in the
/// database is plain text (Rails calls it `password_digest`).
///
/// ```
/// use thymus::{Password, PasswordHash};
///
/// let hash = PasswordHash::new(&Password::new("correct horse")?)?;
/// assert!(hash.as_str().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
/// assert!(hash.verify("correct horse"));
/// assert!(!hash.verify("battery staple"));
///
/// // Back from the database:
/// let stored: PasswordHash = hash.as_str().parse()?;
/// assert!(stored.verify("correct horse"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    /// Hashes `password` with Argon2id, the OWASP parameters
    /// ([`M_COST_KIB`], [`T_COST`], [`P_COST`]) and a fresh random salt.
    ///
    /// # Errors
    ///
    /// [`Error::Hashing`] if the random salt or the hash could not be made.
    pub fn new(password: &Password) -> Result<Self, Error> {
        let hash = argon2()
            .hash_password(password.expose_secret().as_bytes())
            .map_err(|_| Error::Hashing)?;
        Ok(Self(hash.to_string()))
    }

    /// Whether `password` is the one this hash was made from. The hash
    /// outputs are compared in constant time.
    ///
    /// Input over [`Password::MAX_BYTES`] is refused without hashing, since
    /// no such password can have been set.
    #[must_use]
    pub fn verify(&self, password: &str) -> bool {
        #[cfg(test)]
        tests::count_verification();
        if password.len() > Password::MAX_BYTES {
            return false;
        }
        let Ok(parsed) = phc::PasswordHash::new(&self.0) else {
            return false;
        };
        argon2()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    }

    /// The PHC string, for storing.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The PHC string, for storing.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl FromStr for PasswordHash {
    type Err = Error;

    /// Reads a stored hash. Only Argon2id, version 19, with valid
    /// parameters, a salt and an output, is accepted.
    fn from_str(s: &str) -> Result<Self, Error> {
        let parsed = phc::PasswordHash::new(s).map_err(|_| Error::InvalidFormat)?;
        let valid = parsed.algorithm == Algorithm::Argon2id.ident()
            && parsed.version == Some(Version::V0x13.into())
            && parsed.salt.is_some()
            && parsed.hash.is_some()
            && Params::try_from(&parsed).is_ok();
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(Error::InvalidFormat)
        }
    }
}

impl TryFrom<String> for PasswordHash {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        s.parse::<Self>()?;
        Ok(Self(s))
    }
}

impl TryFrom<&str> for PasswordHash {
    type Error = Error;

    fn try_from(s: &str) -> Result<Self, Error> {
        s.parse()
    }
}

impl AsRef<str> for PasswordHash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<PasswordHash> for String {
    fn from(hash: PasswordHash) -> Self {
        hash.0
    }
}

impl fmt::Display for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Shows only the algorithm, so hashes don't end up in logs.
impl fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordHash($argon2id$…)")
    }
}

/// A hash of a password no one knows, with the same parameters as
/// [`PasswordHash::new`], so checking it costs as much as checking a real one.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$edcsp5o9SPOpin/Sh2zXNA$0wH7KHAVrU5RBUPidX5mNJ784sHi7J/ckg5S2HMdvR8";

/// Signs in, taking as long for an unknown account as for a wrong password.
///
/// Pass the stored hash of the user found by e-mail, or `None` if there is
/// no such user. With `None` the password is still checked against a fixed
/// dummy hash, and the answer is `false`. So the response time does not tell
/// an attacker which e-mails have accounts. This is what Rails'
/// `User.authenticate_by(email_address:, password:)` does.
///
/// ```
/// use thymus::{authenticate, normalize_email, Password, PasswordHash};
///
/// let stored = PasswordHash::new(&Password::new("correct horse")?)?;
/// let find = |email: &str| (email == "jana@example.com").then_some(&stored);
///
/// assert!(authenticate(find(&normalize_email(" Jana@Example.com ")), "correct horse"));
/// assert!(!authenticate(find("jana@example.com"), "wrong"));
/// assert!(!authenticate(find("nobody@example.com"), "correct horse"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn authenticate(found: Option<&PasswordHash>, password: &str) -> bool {
    if let Some(hash) = found {
        hash.verify(password)
    } else {
        let dummy = PasswordHash(DUMMY_HASH.to_owned());
        // Always false; the work is what matters.
        std::hint::black_box(dummy.verify(password));
        false
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static VERIFICATIONS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn count_verification() {
        VERIFICATIONS.with(|n| n.set(n.get() + 1));
    }

    fn verifications() -> usize {
        VERIFICATIONS.with(Cell::get)
    }

    #[test]
    fn dummy_hash_is_a_valid_hash_with_the_current_parameters() {
        let dummy: PasswordHash = DUMMY_HASH.parse().unwrap();
        let parsed = phc::PasswordHash::new(dummy.as_str()).unwrap();
        let params = Params::try_from(&parsed).unwrap();
        assert_eq!(
            (params.m_cost(), params.t_cost(), params.p_cost()),
            (M_COST_KIB, T_COST, P_COST)
        );
    }

    #[test]
    fn unknown_user_still_runs_a_verification() {
        let before = verifications();
        assert!(!authenticate(None, "whatever password"));
        assert_eq!(verifications(), before + 1);
    }
}
