//! Small helpers for the sign-in forms.

/// The form of an e-mail address used to store and look up accounts:
/// surrounding whitespace removed, lowercased. The port of Rails'
/// `normalizes :email_address, with: ->(email) { email.strip.downcase }`.
///
/// Use it both when saving a user and when signing in, so `" Jana@Example.com"`
/// finds the account of `jana@example.com`.
///
/// ```
/// assert_eq!(thymus::normalize_email("  Jana.Nova@Example.CZ\n"), "jana.nova@example.cz");
/// ```
#[must_use]
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Whether a `return_to` address may be followed after signing in: only a
/// path on this site.
///
/// It must start with `/` not followed by `/` or `\`, and contain no
/// backslash and no control character. Browsers read `//evil.com` and
/// `/\evil.com` as another site, and strip tabs and newlines from URLs, so
/// `/\t/evil.com` would become `//evil.com`. Anything with a scheme
/// (`https:`, `javascript:`) doesn't start with `/`.
///
/// The exact rule of `local_path?` in the Rails app:
/// `path.match?(%r{\A/(?![/\\])}) && !path.match?(/[\\[:cntrl:]]/)`.
///
/// ```
/// use thymus::is_local_path;
///
/// assert!(is_local_path("/kosik/udaje"));
/// assert!(is_local_path("/ok?x=1"));
/// assert!(!is_local_path("//evil.com"));
/// assert!(!is_local_path("/\\evil.com"));
/// assert!(!is_local_path("https://evil.com"));
/// ```
#[must_use]
pub fn is_local_path(path: &str) -> bool {
    let mut chars = path.chars();
    chars.next() == Some('/')
        && !matches!(chars.next(), Some('/' | '\\'))
        && !path.chars().any(|c| c == '\\' || c.is_control())
}
