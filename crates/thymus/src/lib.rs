//! Password authentication building blocks for a web app: what Rails'
//! `has_secure_password`, `authenticate_by` and `password_reset_token` do,
//! without a web framework.
//!
//! The thymus is the organ that trains immune cells to tell self from
//! intruder; this crate tells account owners from impostors.
//!
//! - [`Password`]: the policy for a new password (8+ characters, at most
//!   1024 bytes), kept in a [`secrecy::SecretString`].
//! - [`PasswordHash`]: Argon2id with the OWASP parameters (19 MiB, 2
//!   iterations, 1 lane), stored as a PHC string, verified in constant time.
//! - [`authenticate`]: sign-in that takes as long for an unknown e-mail as for
//!   a wrong password.
//! - [`ResetTokens`]: signed, expiring reset links that stop working once the
//!   password changes.
//! - [`normalize_email`] and [`is_local_path`]: the e-mail form used for
//!   lookups, and which `return_to` addresses are safe to redirect to.
//!
//! ```
//! use std::time::SystemTime;
//! use thymus::{authenticate, normalize_email, Password, PasswordHash, ResetTokens};
//!
//! // Sign-up.
//! let email = normalize_email(" Jana@Example.com ");
//! let mut stored = PasswordHash::new(&Password::new("correct horse")?)?;
//!
//! // Sign-in.
//! assert!(authenticate(Some(&stored), "correct horse"));
//!
//! // Forgotten password: e-mail a link with the token, then on its form:
//! let tokens = ResetTokens::new(&[7; 32]);
//! let token = tokens.issue(1, &stored, SystemTime::now());
//! let user_id = tokens.verify(&token, SystemTime::now(), |_id| Some(stored.clone()))?;
//! stored = PasswordHash::new(&Password::new("battery staple")?)?;
//! // The link is used up: the hash it was bound to is gone.
//! assert!(tokens.verify(&token, SystemTime::now(), |_id| Some(stored.clone())).is_err());
//! # let _ = (email, user_id);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Wiring it into a web app
//!
//! The crate does no I/O: the app owns the users table, the sessions, the
//! e-mails and the rate limits. Argon2 takes tens of milliseconds of CPU, so
//! in an async server run [`PasswordHash::new`], [`PasswordHash::verify`] and
//! [`authenticate`] on a blocking thread (`tokio::task::spawn_blocking`).
//! With Axum, sign-in looks like this:
//!
//! ```ignore
//! async fn sign_in(State(app): State<App>, session: Session, Form(form): Form<SignIn>) -> Response {
//!     let email = thymus::normalize_email(&form.email_address);
//!     // `None` for an unknown e-mail; authenticate still does the work.
//!     let hash: Option<PasswordHash> = app.db.password_hash_by_email(&email).await;
//!     let ok = tokio::task::spawn_blocking(move || thymus::authenticate(hash.as_ref(), &form.password))
//!         .await
//!         .unwrap();
//!     if !ok {
//!         return (StatusCode::UNPROCESSABLE_ENTITY, "Wrong e-mail or password.").into_response();
//!     }
//!     session.renew().await; // a new session ID against session fixation
//!     // ... remember the user in the session ...
//!     let to = session.remove::<String>("return_to").await.filter(|p| thymus::is_local_path(p));
//!     Redirect::to(to.as_deref().unwrap_or("/account")).into_response()
//! }
//! ```
//!
//! Only store a `return_to` from the query string when [`is_local_path`]
//! accepts it, and check it again before redirecting.
//!
//! For a forgotten password, answer the request form the same way whether
//! or not the e-mail has an account, and e-mail
//! `https://<host>/password/edit?token=<ResetTokens::issue(..)>`. On that
//! page, [`ResetTokens::decode`] the token, load the user's hash with an
//! `async` query, then [`ResetTokens::confirm`] it. After saving the new
//! hash, sign the user out everywhere. The token then no longer works, since
//! it was bound to the old hash.
//!
//! Rate-limit both forms (per IP address and per account); this crate
//! makes guessing slow but not impossible.

#![forbid(unsafe_code)]

mod password;
mod token;
mod web;

pub use password::{
    Error, M_COST_KIB, P_COST, Password, PasswordHash, PolicyError, T_COST, authenticate,
};
pub use secrecy;
pub use token::{ResetClaim, ResetTokens, TokenError};
pub use web::{is_local_path, normalize_email};
