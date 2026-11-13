//! The Content-Security-Policy builder.

use std::borrow::Cow;
use std::fmt;

use crate::Error;
use crate::nonce::CspNonce;

/// One entry in a CSP source list, such as `'self'` or `https://example.com`.
///
/// Keywords are rendered with the quotes the CSP grammar requires, so
/// [`Source::SelfOrigin`] becomes `'self'` and [`Source::Nonce`] becomes
/// `'nonce-…'` with the nonce of the current request. Hosts and schemes are
/// rendered as given.
///
/// ```
/// use cuticle::Source;
///
/// assert_eq!(Source::SelfOrigin.to_string(), "'self'");
/// assert_eq!(Source::Data.to_string(), "data:");
/// assert_eq!(Source::host("https://www.googletagmanager.com").to_string(), "https://www.googletagmanager.com");
/// assert_eq!(Source::Sha256("abc=".into()).to_string(), "'sha256-abc='");
/// // The nonce placeholder is filled in per request by the layer.
/// assert_eq!(Source::Nonce.to_string(), "'nonce-{nonce}'");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Source {
    /// `'self'`: the page's own origin.
    SelfOrigin,
    /// `'none'`: nothing. Only meaningful on its own.
    None,
    /// `'nonce-…'`: replaced by the request's [`CspNonce`] when the header is sent.
    Nonce,
    /// `'unsafe-inline'`. Ignored by browsers for scripts and styles when a nonce or hash is present.
    UnsafeInline,
    /// `'unsafe-eval'`: allows `eval()` and friends.
    UnsafeEval,
    /// `'unsafe-hashes'`: lets hashes match inline event handlers and `style` attributes.
    UnsafeHashes,
    /// `'wasm-unsafe-eval'`: allows compiling WebAssembly (needed by Leptos hydration and islands)
    /// without allowing JavaScript `eval()`.
    WasmUnsafeEval,
    /// `'strict-dynamic'`: scripts loaded by a trusted (nonced) script are trusted too.
    StrictDynamic,
    /// `'report-sample'`: include the first characters of the blocked code in reports.
    ReportSample,
    /// `'inline-speculation-rules'`: allows inline `<script type="speculationrules">`.
    InlineSpeculationRules,
    /// `data:` URLs.
    Data,
    /// `blob:` URLs.
    Blob,
    /// `https:`: any host over HTTPS.
    Https,
    /// Any scheme, written without the colon, such as `"wss"` for `wss:`.
    Scheme(Cow<'static, str>),
    /// A host source such as `"https://www.google.com"` or `"https://*.google-analytics.com"`.
    Host(Cow<'static, str>),
    /// `'sha256-…'` with the base64 digest of an allowed inline script or style.
    Sha256(Cow<'static, str>),
    /// `'sha384-…'` with the base64 digest of an allowed inline script or style.
    Sha384(Cow<'static, str>),
    /// `'sha512-…'` with the base64 digest of an allowed inline script or style.
    Sha512(Cow<'static, str>),
    /// Written to the header exactly as given. Used for the values of
    /// `report-uri`, `report-to`, `sandbox` and other non-source directives.
    Raw(Cow<'static, str>),
}

impl Source {
    /// A host source, such as `https://www.googletagmanager.com`.
    #[must_use]
    pub fn host(host: impl Into<Cow<'static, str>>) -> Self {
        Self::Host(host.into())
    }

    /// A scheme source, written without the colon (`"wss"` gives `wss:`).
    #[must_use]
    pub fn scheme(scheme: impl Into<Cow<'static, str>>) -> Self {
        Self::Scheme(scheme.into())
    }

    /// A value written to the header as given.
    #[must_use]
    pub fn raw(value: impl Into<Cow<'static, str>>) -> Self {
        Self::Raw(value.into())
    }

    fn keyword(&self) -> Option<&'static str> {
        Some(match self {
            Self::SelfOrigin => "'self'",
            Self::None => "'none'",
            Self::UnsafeInline => "'unsafe-inline'",
            Self::UnsafeEval => "'unsafe-eval'",
            Self::UnsafeHashes => "'unsafe-hashes'",
            Self::WasmUnsafeEval => "'wasm-unsafe-eval'",
            Self::StrictDynamic => "'strict-dynamic'",
            Self::ReportSample => "'report-sample'",
            Self::InlineSpeculationRules => "'inline-speculation-rules'",
            Self::Data => "data:",
            Self::Blob => "blob:",
            Self::Https => "https:",
            _ => return None,
        })
    }

    /// Writes the source, calling `nonce` where the nonce goes.
    fn render(&self, out: &mut Renderer<'_>) {
        if let Some(keyword) = self.keyword() {
            out.push(keyword);
            return;
        }
        match self {
            Self::Nonce => {
                out.push("'nonce-");
                out.nonce();
                out.push("'");
            }
            Self::Scheme(scheme) => {
                out.push(scheme.trim_end_matches(':'));
                out.push(":");
            }
            Self::Host(value) | Self::Raw(value) => out.push(value),
            Self::Sha256(digest) => out.quoted("sha256-", digest),
            Self::Sha384(digest) => out.quoted("sha384-", digest),
            Self::Sha512(digest) => out.quoted("sha512-", digest),
            _ => unreachable!("keywords are handled above"),
        }
    }

    fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Scheme(v) => check_scheme(v),
            Self::Host(v) => check_token("CSP host source", v),
            Self::Raw(v) => check_token("CSP value", v),
            Self::Sha256(v) | Self::Sha384(v) | Self::Sha512(v) => check_digest(v),
            _ => Ok(()),
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut renderer = Renderer::new(Some("{nonce}"));
        self.render(&mut renderer);
        f.write_str(&renderer.finish().concat())
    }
}

/// A Content-Security-Policy, built directive by directive.
///
/// Directives keep the order in which they were first set. Setting a
/// directive again replaces its sources; [`Csp::extend`] adds to them.
/// Put [`Source::Nonce`] wherever the per-request nonce should go.
///
/// ```
/// use cuticle::{Csp, Source};
///
/// let csp = Csp::new()
///     .default_src([Source::SelfOrigin])
///     .script_src([Source::SelfOrigin, Source::host("https://www.googletagmanager.com"), Source::Nonce])
///     .object_src([Source::None])
///     .upgrade_insecure_requests();
///
/// assert_eq!(
///     csp.to_string(),
///     "default-src 'self'; \
///      script-src 'self' https://www.googletagmanager.com 'nonce-{nonce}'; \
///      object-src 'none'; upgrade-insecure-requests"
/// );
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Csp {
    directives: Vec<Directive>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Directive {
    name: Cow<'static, str>,
    sources: Vec<Source>,
}

macro_rules! directives {
    ($($(#[$doc:meta])* $method:ident => $name:literal;)*) => {
        $(
            $(#[$doc])*
            #[must_use]
            pub fn $method(self, sources: impl IntoIterator<Item = Source>) -> Self {
                self.directive($name, sources)
            }
        )*
    };
}

impl Csp {
    /// An empty policy with no directives. Browsers allow everything until
    /// directives are added.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A strict starting point: everything from the site's own origin, inline
    /// scripts only with the nonce, no plugins, no framing.
    ///
    /// ```text
    /// default-src 'self'; script-src 'self' 'nonce-…'; style-src 'self';
    /// img-src 'self' data:; font-src 'self'; connect-src 'self';
    /// object-src 'none'; base-uri 'self'; form-action 'self';
    /// frame-ancestors 'none'
    /// ```
    ///
    /// This policy does not allow WebAssembly; use [`Csp::leptos`] for a
    /// Leptos app.
    #[must_use]
    pub fn strict() -> Self {
        Self::new()
            .default_src([Source::SelfOrigin])
            .script_src([Source::SelfOrigin, Source::Nonce])
            .style_src([Source::SelfOrigin])
            .img_src([Source::SelfOrigin, Source::Data])
            .font_src([Source::SelfOrigin])
            .connect_src([Source::SelfOrigin])
            .object_src([Source::None])
            .base_uri([Source::SelfOrigin])
            .form_action([Source::SelfOrigin])
            .frame_ancestors([Source::None])
    }

    /// [`Csp::strict`] adjusted for a Leptos app, and the policy [`Cuticle::new`](crate::Cuticle::new) sends.
    ///
    /// Leptos hydration and islands compile a WebAssembly module, which a CSP
    /// blocks unless `script-src` allows `'wasm-unsafe-eval'`. That keyword
    /// allows WebAssembly only; JavaScript `eval()` stays blocked. The nonce
    /// is also allowed in `style-src`, for `leptos_meta`'s `<Style>`.
    ///
    /// ```text
    /// script-src 'self' 'nonce-…' 'wasm-unsafe-eval'; style-src 'self' 'nonce-…'
    /// ```
    ///
    /// During development with `cargo leptos watch`, live reload connects
    /// over a WebSocket; allow it with
    /// `.extend("connect-src", [Source::host("ws://127.0.0.1:3001")])`.
    #[must_use]
    pub fn leptos() -> Self {
        Self::strict()
            .extend("script-src", [Source::WasmUnsafeEval])
            .extend("style-src", [Source::Nonce])
    }

    /// Sets any directive by name, replacing its sources if it is already set.
    /// The escape hatch for directives without a method of their own.
    ///
    /// ```
    /// use cuticle::{Csp, Source};
    ///
    /// let csp = Csp::new().directive("worker-src", [Source::SelfOrigin, Source::Blob]);
    /// assert_eq!(csp.to_string(), "worker-src 'self' blob:");
    /// ```
    #[must_use]
    pub fn directive(
        mut self,
        name: impl Into<Cow<'static, str>>,
        sources: impl IntoIterator<Item = Source>,
    ) -> Self {
        let name = name.into();
        let sources = sources.into_iter().collect();
        match self.directives.iter_mut().find(|d| d.name == name) {
            Some(existing) => existing.sources = sources,
            None => self.directives.push(Directive { name, sources }),
        }
        self
    }

    /// Adds sources to a directive, creating it if needed. Sources it
    /// already has are not repeated.
    ///
    /// ```
    /// use cuticle::{Csp, Source};
    ///
    /// let csp = Csp::strict().extend("connect-src", [Source::host("wss://example.com")]);
    /// assert_eq!(
    ///     csp.get("connect-src").unwrap(),
    ///     [Source::SelfOrigin, Source::host("wss://example.com")]
    /// );
    /// ```
    #[must_use]
    pub fn extend(
        mut self,
        name: impl Into<Cow<'static, str>>,
        sources: impl IntoIterator<Item = Source>,
    ) -> Self {
        let name = name.into();
        let index = if let Some(index) = self.directives.iter().position(|d| d.name == name) {
            index
        } else {
            self.directives.push(Directive {
                name,
                sources: Vec::new(),
            });
            self.directives.len() - 1
        };
        let existing = &mut self.directives[index].sources;
        for source in sources {
            if !existing.contains(&source) {
                existing.push(source);
            }
        }
        self
    }

    /// Removes a directive.
    ///
    /// ```
    /// use cuticle::Csp;
    ///
    /// let csp = Csp::strict().remove("frame-ancestors");
    /// assert!(csp.get("frame-ancestors").is_none());
    /// ```
    #[must_use]
    pub fn remove(mut self, name: &str) -> Self {
        self.directives.retain(|d| d.name != name);
        self
    }

    /// The sources of a directive, if it is set.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&[Source]> {
        self.directives
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.sources.as_slice())
    }

    /// Whether any directive uses [`Source::Nonce`].
    #[must_use]
    pub fn uses_nonce(&self) -> bool {
        self.directives
            .iter()
            .any(|d| d.sources.contains(&Source::Nonce))
    }

    directives! {
        /// `default-src`: the fallback for the other fetch directives.
        default_src => "default-src";
        /// `script-src`: where scripts may load from and which inline scripts may run.
        script_src => "script-src";
        /// `style-src`: stylesheets and inline styles.
        style_src => "style-src";
        /// `img-src`: images and favicons.
        img_src => "img-src";
        /// `connect-src`: `fetch`, XHR, web sockets, `sendBeacon` and `EventSource`.
        connect_src => "connect-src";
        /// `frame-src`: what this page may embed in frames.
        frame_src => "frame-src";
        /// `font-src`: web fonts.
        font_src => "font-src";
        /// `media-src`: audio and video.
        media_src => "media-src";
        /// `worker-src`: web, shared and service workers.
        worker_src => "worker-src";
        /// `manifest-src`: web app manifests.
        manifest_src => "manifest-src";
        /// `object-src`: `<object>` and `<embed>`. Usually `'none'`.
        object_src => "object-src";
        /// `base-uri`: what `<base href>` may point to.
        base_uri => "base-uri";
        /// `form-action`: where forms may submit to.
        form_action => "form-action";
        /// `frame-ancestors`: who may embed this page in a frame (the modern `X-Frame-Options`).
        frame_ancestors => "frame-ancestors";
    }

    /// `upgrade-insecure-requests`: the browser loads `http:` URLs over `https:`.
    #[must_use]
    pub fn upgrade_insecure_requests(self) -> Self {
        self.directive("upgrade-insecure-requests", [])
    }

    /// `report-uri`: where browsers POST violation reports (deprecated in
    /// favour of `report-to`, but still the only one Firefox supports).
    #[must_use]
    pub fn report_uri(self, uri: impl Into<Cow<'static, str>>) -> Self {
        self.directive("report-uri", [Source::Raw(uri.into())])
    }

    /// `report-to`: the reporting group, defined in a `Reporting-Endpoints`
    /// header, to send violation reports to.
    #[must_use]
    pub fn report_to(self, group: impl Into<Cow<'static, str>>) -> Self {
        self.directive("report-to", [Source::Raw(group.into())])
    }

    /// Renders the policy with the given nonce.
    ///
    /// ```
    /// use cuticle::{Csp, CspNonce, Source};
    ///
    /// let nonce = CspNonce::generate();
    /// let csp = Csp::new().script_src([Source::Nonce]);
    /// assert_eq!(csp.render(&nonce), format!("script-src 'nonce-{nonce}'"));
    /// ```
    #[must_use]
    pub fn render(&self, nonce: &CspNonce) -> String {
        self.render_parts(Some(nonce.as_str())).concat()
    }

    /// Checks every directive name and source, so the policy is a valid header value.
    pub(crate) fn validate(&self) -> Result<(), Error> {
        for directive in &self.directives {
            let name_ok = !directive.name.is_empty()
                && directive
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-');
            if !name_ok {
                return Err(Error::new("CSP directive name", &directive.name));
            }
            for source in &directive.sources {
                source.validate()?;
            }
        }
        Ok(())
    }

    /// Renders the policy into the pieces that go between nonces: a policy
    /// with two nonces gives three pieces. With `nonce` set, the nonce is
    /// written in instead and there is one piece.
    pub(crate) fn render_parts(&self, nonce: Option<&str>) -> Vec<String> {
        let mut out = Renderer::new(nonce);
        for (i, directive) in self.directives.iter().enumerate() {
            if i > 0 {
                out.push("; ");
            }
            out.push(&directive.name);
            for source in &directive.sources {
                out.push(" ");
                source.render(&mut out);
            }
        }
        out.finish()
    }
}

impl fmt::Display for Csp {
    /// Renders the policy with `{nonce}` where the nonce goes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_parts(Some("{nonce}")).concat())
    }
}

/// Collects rendered text, splitting it where the nonce goes.
struct Renderer<'a> {
    nonce: Option<&'a str>,
    parts: Vec<String>,
    current: String,
}

impl<'a> Renderer<'a> {
    fn new(nonce: Option<&'a str>) -> Self {
        Self {
            nonce,
            parts: Vec::new(),
            current: String::new(),
        }
    }

    fn push(&mut self, s: &str) {
        self.current.push_str(s);
    }

    fn quoted(&mut self, prefix: &str, value: &str) {
        self.push("'");
        self.push(prefix);
        self.push(value);
        self.push("'");
    }

    fn nonce(&mut self) {
        match self.nonce {
            Some(nonce) => self.current.push_str(nonce),
            None => self.parts.push(std::mem::take(&mut self.current)),
        }
    }

    fn finish(mut self) -> Vec<String> {
        self.parts.push(self.current);
        self.parts
    }
}

/// A single token in a header: visible ASCII without the separators that
/// would end a source (space), a directive (`;`) or a policy (`,`).
fn check_token(what: &'static str, value: &str) -> Result<(), Error> {
    let ok = !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b';' && b != b',' && b != b'\'');
    if ok {
        Ok(())
    } else {
        Err(Error::new(what, value))
    }
}

fn check_scheme(value: &str) -> Result<(), Error> {
    let scheme = value.trim_end_matches(':');
    let ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
    if ok {
        Ok(())
    } else {
        Err(Error::new("CSP scheme source", value))
    }
}

fn check_digest(value: &str) -> Result<(), Error> {
    let ok = !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'-' | b'_' | b'='));
    if ok {
        Ok(())
    } else {
        Err(Error::new("CSP hash source", value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_are_quoted() {
        let cases = [
            (Source::SelfOrigin, "'self'"),
            (Source::None, "'none'"),
            (Source::UnsafeInline, "'unsafe-inline'"),
            (Source::UnsafeEval, "'unsafe-eval'"),
            (Source::UnsafeHashes, "'unsafe-hashes'"),
            (Source::WasmUnsafeEval, "'wasm-unsafe-eval'"),
            (Source::StrictDynamic, "'strict-dynamic'"),
            (Source::ReportSample, "'report-sample'"),
            (Source::InlineSpeculationRules, "'inline-speculation-rules'"),
            (Source::Data, "data:"),
            (Source::Blob, "blob:"),
            (Source::Https, "https:"),
            (Source::scheme("wss"), "wss:"),
            (Source::scheme("wss:"), "wss:"),
            (
                Source::host("https://*.example.com"),
                "https://*.example.com",
            ),
            (Source::Sha384("abc".into()), "'sha384-abc'"),
            (Source::Sha512("abc".into()), "'sha512-abc'"),
        ];
        for (source, expected) in cases {
            assert_eq!(source.to_string(), expected);
        }
    }

    #[test]
    fn splits_around_nonces() {
        let csp = Csp::leptos();
        let parts = csp.render_parts(None);
        assert_eq!(parts.len(), 3);
        assert!(parts[0].ends_with("script-src 'self' 'nonce-"));
        assert!(parts[1].starts_with("' 'wasm-unsafe-eval'; style-src 'self' 'nonce-"));
    }

    #[test]
    fn strict_policy() {
        assert_eq!(
            Csp::strict().to_string(),
            "default-src 'self'; script-src 'self' 'nonce-{nonce}'; style-src 'self'; \
             img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; \
             base-uri 'self'; form-action 'self'; frame-ancestors 'none'"
        );
    }

    #[test]
    fn leptos_policy_allows_wasm() {
        let csp = Csp::leptos();
        assert_eq!(
            csp.get("script-src").unwrap(),
            [Source::SelfOrigin, Source::Nonce, Source::WasmUnsafeEval]
        );
        assert_eq!(
            csp.get("style-src").unwrap(),
            [Source::SelfOrigin, Source::Nonce]
        );
    }

    #[test]
    fn directive_replaces_in_place() {
        let csp = Csp::strict().script_src([Source::SelfOrigin]);
        assert!(
            csp.to_string()
                .starts_with("default-src 'self'; script-src 'self'; style-src")
        );
        assert!(!csp.uses_nonce());
    }

    #[test]
    fn report_directives() {
        let csp = Csp::new()
            .report_uri("/csp-reports")
            .report_to("csp-endpoint");
        assert_eq!(
            csp.to_string(),
            "report-uri /csp-reports; report-to csp-endpoint"
        );
    }

    #[test]
    fn rejects_values_that_break_the_header() {
        for bad in [
            Csp::new().img_src([Source::host("https://a.com; script-src *")]),
            Csp::new().img_src([Source::host("https://a.com https://b.com")]),
            Csp::new().img_src([Source::host("https://a.com\n")]),
            Csp::new().img_src([Source::host("")]),
            Csp::new().img_src([Source::scheme("1http")]),
            Csp::new().img_src([Source::Sha256("a b".into())]),
            Csp::new().directive("script-src;", [Source::SelfOrigin]),
            Csp::new().report_uri("/a, b"),
        ] {
            assert!(bad.validate().is_err(), "{bad}");
        }
        assert!(Csp::leptos().validate().is_ok());
    }
}
