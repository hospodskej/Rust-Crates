//! The values of the other security headers.

use std::borrow::Cow;
use std::fmt;
use std::time::Duration;

use crate::Error;

/// `X-Frame-Options`: whether other sites may show this page in a frame.
///
/// Browsers that support the CSP `frame-ancestors` directive ignore this
/// header when the policy has one; it remains for older browsers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum XFrameOptions {
    /// `DENY`: never in a frame.
    #[default]
    Deny,
    /// `SAMEORIGIN`: only in frames on the same origin.
    SameOrigin,
}

impl XFrameOptions {
    /// The header value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "DENY",
            Self::SameOrigin => "SAMEORIGIN",
        }
    }
}

/// `Referrer-Policy`: how much of this page's URL other sites see in `Referer`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReferrerPolicy {
    /// `no-referrer`
    NoReferrer,
    /// `no-referrer-when-downgrade`
    NoReferrerWhenDowngrade,
    /// `origin`
    Origin,
    /// `origin-when-cross-origin`
    OriginWhenCrossOrigin,
    /// `same-origin`
    SameOrigin,
    /// `strict-origin`
    StrictOrigin,
    /// `strict-origin-when-cross-origin`: the full URL to the same origin, only
    /// the origin to others over HTTPS, nothing over HTTP. The browsers' default.
    #[default]
    StrictOriginWhenCrossOrigin,
    /// `unsafe-url`
    UnsafeUrl,
}

impl ReferrerPolicy {
    /// The header value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoReferrer => "no-referrer",
            Self::NoReferrerWhenDowngrade => "no-referrer-when-downgrade",
            Self::Origin => "origin",
            Self::OriginWhenCrossOrigin => "origin-when-cross-origin",
            Self::SameOrigin => "same-origin",
            Self::StrictOrigin => "strict-origin",
            Self::StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin",
            Self::UnsafeUrl => "unsafe-url",
        }
    }
}

/// `Cross-Origin-Opener-Policy`: whether pages this page opens, or that open
/// it, from other origins share its browsing context group (and `window.opener`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CrossOriginOpenerPolicy {
    /// `same-origin`: cross-origin windows are isolated from this one.
    #[default]
    SameOrigin,
    /// `same-origin-allow-popups`: like `same-origin`, but popups this page
    /// opens keep a reference to it (needed by some OAuth and payment popups).
    SameOriginAllowPopups,
    /// `noopener-allow-popups`
    NoopenerAllowPopups,
    /// `unsafe-none`: no isolation.
    UnsafeNone,
}

impl CrossOriginOpenerPolicy {
    /// The header value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameOrigin => "same-origin",
            Self::SameOriginAllowPopups => "same-origin-allow-popups",
            Self::NoopenerAllowPopups => "noopener-allow-popups",
            Self::UnsafeNone => "unsafe-none",
        }
    }
}

/// `Strict-Transport-Security`: the browser uses only HTTPS for this host
/// for `max_age`.
///
/// Send it only from a site served over HTTPS, which is why
/// [`Cuticle::new`](crate::Cuticle::new) leaves it off.
///
/// ```
/// use std::time::Duration;
/// use cuticle::Hsts;
///
/// // What Rails' `config.force_ssl = true` sends.
/// let hsts = Hsts::new(Duration::from_secs(2 * 365 * 24 * 60 * 60)).include_subdomains();
/// assert_eq!(hsts.to_string(), "max-age=63072000; includeSubDomains");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hsts {
    max_age: Duration,
    include_subdomains: bool,
    preload: bool,
}

impl Hsts {
    /// HTTPS only for `max_age` (rounded down to whole seconds).
    #[must_use]
    pub fn new(max_age: Duration) -> Self {
        Self {
            max_age,
            include_subdomains: false,
            preload: false,
        }
    }

    /// Also for all subdomains (`includeSubDomains`).
    #[must_use]
    pub fn include_subdomains(mut self) -> Self {
        self.include_subdomains = true;
        self
    }

    /// Asks to be put on the browsers' preload list (`preload`), which also
    /// needs `includeSubDomains` (set by this method), a `max_age` of at least
    /// a year and a submission at <https://hstspreload.org>. Hard to undo.
    #[must_use]
    pub fn preload(mut self) -> Self {
        self.include_subdomains = true;
        self.preload = true;
        self
    }
}

impl fmt::Display for Hsts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "max-age={}", self.max_age.as_secs())?;
        if self.include_subdomains {
            f.write_str("; includeSubDomains")?;
        }
        if self.preload {
            f.write_str("; preload")?;
        }
        Ok(())
    }
}

/// Who may use a browser feature, in a [`PermissionsPolicy`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Allow {
    /// `*`: every origin, including frames from other sites.
    All,
    /// `self`: this page's origin.
    SelfOrigin,
    /// `src`: the origin in an iframe's `src` (for the `allow` attribute).
    Src,
    /// A quoted origin, such as `"https://maps.example.com"`.
    Origin(Cow<'static, str>),
}

impl Allow {
    /// An origin, such as `https://maps.example.com`.
    #[must_use]
    pub fn origin(origin: impl Into<Cow<'static, str>>) -> Self {
        Self::Origin(origin.into())
    }
}

/// `Permissions-Policy`: which browser features (camera, geolocation, …) the
/// page and the frames in it may use.
///
/// ```
/// use cuticle::{Allow, PermissionsPolicy};
///
/// let policy = PermissionsPolicy::new()
///     .deny("camera")
///     .allow("geolocation", [Allow::SelfOrigin, Allow::origin("https://maps.example.com")])
///     .allow("fullscreen", [Allow::All]);
/// assert_eq!(
///     policy.to_string(),
///     r#"camera=(), geolocation=(self "https://maps.example.com"), fullscreen=*"#
/// );
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionsPolicy {
    features: Vec<(Cow<'static, str>, Vec<Allow>)>,
}

impl PermissionsPolicy {
    /// An empty policy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Turns off the powerful features a typical website does not use:
    ///
    /// ```text
    /// accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(),
    /// microphone=(), payment=(), usb=()
    /// ```
    #[must_use]
    pub fn strict() -> Self {
        [
            "accelerometer",
            "camera",
            "geolocation",
            "gyroscope",
            "magnetometer",
            "microphone",
            "payment",
            "usb",
        ]
        .into_iter()
        .fold(Self::new(), Self::deny)
    }

    /// Turns a feature off everywhere: `feature=()`.
    #[must_use]
    pub fn deny(self, feature: impl Into<Cow<'static, str>>) -> Self {
        self.allow(feature, [])
    }

    /// Allows a feature for the given origins, replacing an earlier setting.
    /// An empty list turns it off.
    #[must_use]
    pub fn allow(
        mut self,
        feature: impl Into<Cow<'static, str>>,
        allowlist: impl IntoIterator<Item = Allow>,
    ) -> Self {
        let feature = feature.into();
        let allowlist = allowlist.into_iter().collect();
        match self.features.iter_mut().find(|(name, _)| *name == feature) {
            Some((_, existing)) => *existing = allowlist,
            None => self.features.push((feature, allowlist)),
        }
        self
    }

    /// Removes a feature from the policy, leaving the browser's default.
    #[must_use]
    pub fn remove(mut self, feature: &str) -> Self {
        self.features.retain(|(name, _)| name != feature);
        self
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        for (name, allowlist) in &self.features {
            let ok = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !ok {
                return Err(Error::new("Permissions-Policy feature", name));
            }
            for allow in allowlist {
                if let Allow::Origin(origin) = allow {
                    let ok = !origin.is_empty()
                        && origin.bytes().all(|b| {
                            b.is_ascii_graphic() && !matches!(b, b'"' | b'\\' | b',' | b')')
                        });
                    if !ok {
                        return Err(Error::new("Permissions-Policy origin", origin));
                    }
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for PermissionsPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (name, allowlist)) in self.features.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{name}=")?;
            if allowlist.contains(&Allow::All) {
                f.write_str("*")?;
                continue;
            }
            f.write_str("(")?;
            for (j, allow) in allowlist.iter().enumerate() {
                if j > 0 {
                    f.write_str(" ")?;
                }
                match allow {
                    Allow::All => unreachable!("handled above"),
                    Allow::SelfOrigin => f.write_str("self")?,
                    Allow::Src => f.write_str("src")?,
                    Allow::Origin(origin) => write!(f, "\"{origin}\"")?,
                }
            }
            f.write_str(")")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_permissions() {
        assert_eq!(
            PermissionsPolicy::strict().to_string(),
            "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), \
             microphone=(), payment=(), usb=()"
        );
    }

    #[test]
    fn permissions_replace_and_remove() {
        let policy = PermissionsPolicy::strict()
            .allow("payment", [Allow::SelfOrigin])
            .remove("usb")
            .remove("accelerometer");
        assert_eq!(
            policy.to_string(),
            "camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(self)"
        );
    }

    #[test]
    fn permissions_validation() {
        assert!(PermissionsPolicy::strict().validate().is_ok());
        assert!(PermissionsPolicy::new().deny("Camera").validate().is_err());
        assert!(
            PermissionsPolicy::new()
                .allow("camera", [Allow::origin("https://a\", b=*")])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn hsts() {
        let year = Duration::from_secs(31_536_000);
        assert_eq!(Hsts::new(year).to_string(), "max-age=31536000");
        assert_eq!(
            Hsts::new(year).preload().to_string(),
            "max-age=31536000; includeSubDomains; preload"
        );
    }
}
