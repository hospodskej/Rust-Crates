# thymus

Password authentication building blocks for a web app: what Rails'
`has_secure_password`, `authenticate_by` and `password_reset_token` do, as a
plain library with no web framework.

The thymus is the organ that trains immune cells to tell self from intruder;
this crate tells account owners from impostors.

- **`Password`**: the policy for a new password, 8+ characters (counted as
  characters, not bytes) and at most 1024 bytes, kept in a `SecretString`.
- **`PasswordHash`**: Argon2id with the OWASP parameters (19 MiB, 2
  iterations, 1 lane), stored as a PHC string, verified in constant time.
- **`authenticate`**: sign-in that takes as long for an unknown e-mail as for
  a wrong password, by checking a dummy hash when no user was found.
- **`ResetTokens`**: HMAC-SHA256 signed, expiring (15 minutes by default)
  reset tokens that stop working once the password changes.
- **`normalize_email`** and **`is_local_path`**: the e-mail form used for
  lookups, and which `return_to` paths are safe to redirect to after sign-in.

```rust
use std::time::SystemTime;
use thymus::{authenticate, normalize_email, Password, PasswordHash, ResetTokens};

// Sign-up: store the normalized e-mail and the PHC string.
let email = normalize_email(" Jana@Example.com ");
let hash = PasswordHash::new(&Password::new("correct horse")?)?;
let stored: String = hash.as_str().to_owned();

// Sign-in: `None` when no user has that e-mail.
let found: Option<PasswordHash> = Some(stored.parse()?);
assert!(authenticate(found.as_ref(), "correct horse"));

// Forgotten password: e-mail a link with the token, check it on the form.
let tokens = ResetTokens::new(&secret_key_from_config);
let token = tokens.issue(user_id, &hash, SystemTime::now());
let user_id = tokens.verify(&token, SystemTime::now(), |id| load_hash(id))?;
```

## Reset token format

87 characters of URL-safe base64 without padding, over 65 bytes:

| bytes | field |
|---|---|
| 1 | format version, `1` |
| 8 | user id, `i64` big-endian |
| 8 | expiry, Unix seconds, `u64` big-endian |
| 16 | password fingerprint: HMAC-SHA256 of the stored PHC string, first 16 bytes |
| 32 | HMAC-SHA256 over the 33 bytes above |

The hash itself is never in the token. Because the fingerprint covers the
PHC string, salt included, a new password makes every older token `Stale`.

## In a web app

The crate does no I/O. Run hashing and `authenticate` on a blocking thread
(Argon2 takes tens of milliseconds of CPU), rate-limit the sign-in and reset
forms, renew the session ID at sign-in, give the same answer to a reset
request whether or not the e-mail has an account, and sign the user out
everywhere after a reset. The crate documentation has an Axum example.

## Licence

Dual-licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.
