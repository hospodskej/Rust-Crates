//! Trusted HTML and escaping of interpolated values.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use core::fmt::{self, Display, Write as _};

/// A string of trusted HTML.
///
/// Translations whose key ends in `_html` (the Rails convention) return
/// `Html`: their text is written by the developer and may contain markup.
/// Values interpolated into them are escaped, unless they are `Html`
/// themselves (see [`HtmlArg`]).
///
/// In Leptos, render it with `inner_html`:
///
/// ```ignore
/// view! { <p inner_html=t::auth::register_privacy(locale, link).into_string()></p> }
/// ```
///
/// ```
/// use codon::Html;
///
/// let html = Html::from_static("<b>Hi</b>");
/// assert_eq!(html.as_str(), "<b>Hi</b>");
/// assert_eq!(Html::escape("<b>").as_str(), "&lt;b&gt;");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Html(Cow<'static, str>);

impl Html {
    /// Wraps a static string of trusted HTML.
    #[must_use]
    pub const fn from_static(html: &'static str) -> Self {
        Self(Cow::Borrowed(html))
    }

    /// Wraps a string that the caller vouches is safe HTML.
    ///
    /// Nothing is escaped: never pass user input here, use [`Html::escape`].
    ///
    /// ```
    /// let link = codon::Html::from_trusted(format!("<a href=\"{}\">{}</a>", "/privacy", "Privacy"));
    /// assert_eq!(link.as_str(), "<a href=\"/privacy\">Privacy</a>");
    /// ```
    pub fn from_trusted(html: impl Into<Cow<'static, str>>) -> Self {
        Self(html.into())
    }

    /// Escapes plain text so that it can be used as HTML.
    #[must_use]
    pub fn escape(text: &str) -> Self {
        Self(Cow::Owned(escape_html(text).into_owned()))
    }

    /// The HTML as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Converts into an owned `String`.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0.into_owned()
    }

    /// Converts into the inner `Cow`, without copying a static string.
    #[must_use]
    pub fn into_inner(self) -> Cow<'static, str> {
        self.0
    }
}

impl Display for Html {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Html {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<Html> for String {
    fn from(html: Html) -> Self {
        html.into_string()
    }
}

impl From<Html> for Cow<'static, str> {
    fn from(html: Html) -> Self {
        html.0
    }
}

/// Escapes `&`, `<`, `>`, `"` and `'` for use in HTML text and attribute
/// values. Borrows the input when there is nothing to escape.
///
/// ```
/// assert_eq!(codon::escape_html("Tom & \"Jerry\""), "Tom &amp; &quot;Jerry&quot;");
/// assert!(matches!(codon::escape_html("plain"), std::borrow::Cow::Borrowed(_)));
/// ```
#[must_use]
pub fn escape_html(text: &str) -> Cow<'_, str> {
    if text.bytes().any(needs_escape) {
        let mut out = String::with_capacity(text.len() + 16);
        push_escaped(&mut out, text);
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    }
}

fn needs_escape(b: u8) -> bool {
    matches!(b, b'&' | b'<' | b'>' | b'"' | b'\'')
}

fn push_escaped(out: &mut String, text: &str) {
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        let entity = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            b'\'' => "&#39;",
            _ => continue,
        };
        out.push_str(&text[start..i]);
        out.push_str(entity);
        start = i + 1;
    }
    out.push_str(&text[start..]);
}

/// A `fmt::Write` that escapes everything written through it.
struct EscapingWriter<'a>(&'a mut String);

impl fmt::Write for EscapingWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        push_escaped(self.0, s);
        Ok(())
    }
}

/// A value that can be interpolated into an `_html` translation.
///
/// Plain values (strings, numbers, …) are HTML-escaped; [`Html`] is inserted
/// as is. For any other [`Display`] type, wrap it in [`Escaped`].
///
/// A blanket implementation for every `Display` type is not possible, because
/// `Html` is `Display` too and would then be escaped. That is why other types
/// go through [`Escaped`], which keeps the choice explicit and safe.
///
/// ```
/// use codon::{Escaped, Html, HtmlArg};
///
/// let mut out = String::new();
/// "<script>".push_html(&mut out);
/// Html::from_static("<br>").push_html(&mut out);
/// 42.push_html(&mut out);
/// Escaped(std::net::Ipv4Addr::LOCALHOST).push_html(&mut out);
/// assert_eq!(out, "&lt;script&gt;<br>42127.0.0.1");
/// ```
pub trait HtmlArg {
    /// Appends the value to `out` as HTML.
    fn push_html(&self, out: &mut String);
}

impl HtmlArg for Html {
    fn push_html(&self, out: &mut String) {
        out.push_str(&self.0);
    }
}

impl HtmlArg for str {
    fn push_html(&self, out: &mut String) {
        push_escaped(out, self);
    }
}

impl HtmlArg for String {
    fn push_html(&self, out: &mut String) {
        push_escaped(out, self);
    }
}

impl HtmlArg for Cow<'_, str> {
    fn push_html(&self, out: &mut String) {
        push_escaped(out, self);
    }
}

impl HtmlArg for char {
    fn push_html(&self, out: &mut String) {
        push_escaped(out, self.encode_utf8(&mut [0; 4]));
    }
}

impl<T: HtmlArg + ?Sized> HtmlArg for &T {
    fn push_html(&self, out: &mut String) {
        (**self).push_html(out);
    }
}

impl<T: HtmlArg + ?Sized> HtmlArg for &mut T {
    fn push_html(&self, out: &mut String) {
        (**self).push_html(out);
    }
}

impl<T: HtmlArg + ?Sized> HtmlArg for Box<T> {
    fn push_html(&self, out: &mut String) {
        (**self).push_html(out);
    }
}

impl<T: HtmlArg + ?Sized> HtmlArg for Rc<T> {
    fn push_html(&self, out: &mut String) {
        (**self).push_html(out);
    }
}

impl<T: HtmlArg + ?Sized> HtmlArg for Arc<T> {
    fn push_html(&self, out: &mut String) {
        (**self).push_html(out);
    }
}

/// Numbers and booleans never contain characters that need escaping.
macro_rules! html_arg_display {
    ($($t:ty),*) => {$(
        impl HtmlArg for $t {
            fn push_html(&self, out: &mut String) {
                let _ = write!(out, "{self}");
            }
        }
    )*};
}

html_arg_display!(
    bool, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
);

/// Interpolates any [`Display`] value into an `_html` translation, escaped.
///
/// ```
/// use codon::{Escaped, HtmlArg};
///
/// struct Name;
/// impl std::fmt::Display for Name {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         f.write_str("<Ann & Bob>")
///     }
/// }
///
/// let mut out = String::new();
/// Escaped(Name).push_html(&mut out);
/// assert_eq!(out, "&lt;Ann &amp; Bob&gt;");
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Escaped<T>(pub T);

impl<T: Display> HtmlArg for Escaped<T> {
    fn push_html(&self, out: &mut String) {
        let _ = write!(EscapingWriter(out), "{}", self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn escapes_all_special_characters() {
        assert_eq!(
            escape_html(r#"<a href="x">Tom & Jerry's</a>"#),
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&#39;s&lt;/a&gt;"
        );
        assert_eq!(escape_html(""), "");
        assert_eq!(escape_html("žluťoučký kůň"), "žluťoučký kůň");
        assert_eq!(escape_html("&&"), "&amp;&amp;");
    }

    #[test]
    fn html_args() {
        let mut out = String::new();
        "a<b".push_html(&mut out);
        "x".to_string().push_html(&mut out);
        '>'.push_html(&mut out);
        Html::from_trusted("<i>".to_string()).push_html(&mut out);
        (&&Html::from_static("</i>")).push_html(&mut out);
        Cow::Borrowed("\"").push_html(&mut out);
        (-1.5_f64).push_html(&mut out);
        true.push_html(&mut out);
        assert_eq!(out, "a&lt;bx&gt;<i></i>&quot;-1.5true");
    }

    #[test]
    fn escaped_display() {
        let mut out = String::new();
        Escaped(format_args!("{}&{}", "<", ">")).push_html(&mut out);
        assert_eq!(out, "&lt;&amp;&gt;");
    }

    #[test]
    fn html_conversions() {
        let html = Html::escape("<p>");
        assert_eq!(html.to_string(), "&lt;p&gt;");
        assert_eq!(String::from(html.clone()), "&lt;p&gt;");
        assert_eq!(html.as_ref(), "&lt;p&gt;");
        assert!(matches!(
            Html::from_static("x").into_inner(),
            Cow::Borrowed("x")
        ));
    }
}
