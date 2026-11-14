//! Bounds-checked reads from the uploaded bytes.
//!
//! Every read of the file goes through [`Reader`], so a value inside the
//! file can never index past its end: a read that does not fit is reported
//! as [`Error::Truncated`] instead.

use crate::{Error, Format};

/// The uploaded bytes of one format, read with bounds checks.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    format: Format,
}

impl<'a> Reader<'a> {
    pub(crate) const fn new(bytes: &'a [u8], format: Format) -> Self {
        Self { bytes, format }
    }

    pub(crate) const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// `length` bytes from `offset`, or [`Error::Truncated`] when the file
    /// ends before them (or the numbers overflow).
    pub(crate) fn slice(&self, offset: usize, length: usize) -> Result<&'a [u8], Error> {
        offset
            .checked_add(length)
            .and_then(|end| self.bytes.get(offset..end))
            .ok_or_else(|| self.truncated())
    }

    /// Everything from `offset` to the end of the file.
    pub(crate) fn rest(&self, offset: usize) -> Result<&'a [u8], Error> {
        self.bytes.get(offset..).ok_or_else(|| self.truncated())
    }

    fn array<const N: usize>(&self, offset: usize) -> Result<[u8; N], Error> {
        self.slice(offset, N)?
            .try_into()
            .map_err(|_| self.truncated())
    }

    pub(crate) fn u8(&self, offset: usize) -> Result<u8, Error> {
        self.bytes
            .get(offset)
            .copied()
            .ok_or_else(|| self.truncated())
    }

    pub(crate) fn u16_be(&self, offset: usize) -> Result<u16, Error> {
        self.array(offset).map(u16::from_be_bytes)
    }

    pub(crate) fn u32_be(&self, offset: usize) -> Result<u32, Error> {
        self.array(offset).map(u32::from_be_bytes)
    }

    pub(crate) fn u32_le(&self, offset: usize) -> Result<u32, Error> {
        self.array(offset).map(u32::from_le_bytes)
    }

    /// A 32-bit length field as a `usize`.
    ///
    /// On a target whose `usize` is narrower than 32 bits a length that does
    /// not fit could not fit in memory either, so it counts as truncated.
    pub(crate) fn length_be(&self, offset: usize) -> Result<usize, Error> {
        usize::try_from(self.u32_be(offset)?).map_err(|_| self.truncated())
    }

    /// Like [`Reader::length_be`], little-endian.
    pub(crate) fn length_le(&self, offset: usize) -> Result<usize, Error> {
        usize::try_from(self.u32_le(offset)?).map_err(|_| self.truncated())
    }

    pub(crate) const fn truncated(&self) -> Error {
        Error::Truncated {
            format: self.format,
        }
    }

    pub(crate) const fn malformed(&self, reason: &'static str) -> Error {
        Error::Malformed {
            format: self.format,
            reason,
        }
    }
}

/// A fixed-size array from the start of `bytes`, if it is long enough.
pub(crate) fn array_at<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    bytes.get(offset..offset.checked_add(N)?)?.try_into().ok()
}
