use std::fmt;

/// A value that cannot go into a header, found by
/// [`Cuticle::try_into_layer`](crate::Cuticle::try_into_layer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    what: &'static str,
    value: String,
}

impl Error {
    pub(crate) fn new(what: &'static str, value: &str) -> Self {
        Self {
            what,
            value: value.to_owned(),
        }
    }

    /// The offending value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: {:?}", self.what, self.value)
    }
}

impl std::error::Error for Error {}
