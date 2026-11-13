//! The CRC-32 (ISO-HDLC, as used by PNG and zlib) that guards every PNG chunk.

#[allow(clippy::indexing_slicing)] // n < 256
const TABLE: [u32; 256] = {
    let mut table = [0_u32; 256];
    let mut n = 0;
    while n < 256 {
        #[allow(clippy::cast_possible_truncation)] // n < 256
        let mut crc = n as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[n] = crc;
        n += 1;
    }
    table
};

/// The CRC-32 of `bytes`.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0_u32, |crc, &byte| {
        let [low, ..] = crc.to_le_bytes();
        #[allow(clippy::indexing_slicing)] // a u8 always indexes a 256-entry table
        let entry = TABLE[usize::from(low ^ byte)];
        entry ^ (crc >> 8)
    })
}

#[cfg(test)]
mod tests {
    use super::crc32;

    #[test]
    fn matches_the_reference_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }
}
