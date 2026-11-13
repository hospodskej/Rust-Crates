//! The pure arithmetic: target dimensions and the output file name.

/// The dimensions a `width` x `height` picture is drawn at so that its longer
/// side is at most `max_side` pixels.
///
/// The aspect ratio is kept and each side is rounded to the nearest pixel
/// (halves round up, like JavaScript's `Math.round`). A picture that already
/// fits keeps its size; it is never enlarged. No side drops below 1 pixel, and
/// a `max_side` of 0 counts as 1.
///
/// ```
/// use attenuator::target_size;
///
/// assert_eq!(target_size(4000, 3000, 1600), (1600, 1200));
/// assert_eq!(target_size(3000, 4000, 1600), (1200, 1600));
/// assert_eq!(target_size(1200, 800, 1600), (1200, 800));
/// ```
#[must_use]
pub fn target_size(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let max_side = max_side.max(1);
    let longer = width.max(height);
    if longer <= max_side {
        return (width, height);
    }
    // Integer arithmetic, so the longer side lands on `max_side` exactly
    // (`width * (max / longer)` in floating point can be off by one).
    let scale = |side: u32| {
        let (side, max, longer) = (u64::from(side), u64::from(max_side), u64::from(longer));
        let rounded = (2 * side * max + longer) / (2 * longer);
        // `rounded <= max_side`, so it fits in a u32.
        u32::try_from(rounded).unwrap_or(max_side).max(1)
    };
    (scale(width), scale(height))
}

/// `name` with its last extension replaced by `extension` (without the dot):
/// `IMG_1234.JPG` becomes `IMG_1234.webp`, `scan` becomes `scan.webp`.
pub(crate) fn renamed(name: &str, extension: &str) -> String {
    let stem = name.rfind('.').map_or(name, |dot| &name[..dot]);
    format!("{stem}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_and_portrait_fit_the_longer_side() {
        assert_eq!(target_size(4032, 3024, 1600), (1600, 1200));
        assert_eq!(target_size(3024, 4032, 1600), (1200, 1600));
        assert_eq!(target_size(6000, 6000, 1600), (1600, 1600));
    }

    #[test]
    fn small_pictures_are_not_enlarged() {
        assert_eq!(target_size(1600, 900, 1600), (1600, 900));
        assert_eq!(target_size(800, 600, 1600), (800, 600));
        assert_eq!(target_size(1, 1, 1600), (1, 1));
    }

    #[test]
    fn rounds_to_the_nearest_pixel_halves_up() {
        // 1000 * 1600 / 3000 = 533.33 -> 533
        assert_eq!(target_size(3000, 1000, 1600), (1600, 533));
        // 1001 * 1600 / 3000 = 533.87 -> 534
        assert_eq!(target_size(3000, 1001, 1600), (1600, 534));
        // 3 * 2 / 4 = 1.5 -> 2 and 1000 * 1600 / 3200 = 500 exactly
        assert_eq!(target_size(4, 3, 2), (2, 2));
        assert_eq!(target_size(3200, 1000, 1600), (1600, 500));
        // 1 * 1600 / 3200 = 0.5 -> 1
        assert_eq!(target_size(3200, 1, 1600), (1600, 1));
    }

    #[test]
    fn matches_the_javascript_formula() {
        // Math.round(side * Math.min(1, max / Math.max(w, h)))
        for &(w, h) in &[
            (4032, 3024),
            (4000, 2250),
            (2448, 3264),
            (5472, 3648),
            (1700, 1),
        ] {
            let scale = (1600.0 / f64::from(w.max(h))).min(1.0);
            let js = |side: u32| (f64::from(side) * scale).round();
            let (tw, th) = target_size(w, h, 1600);
            assert_eq!(f64::from(tw), js(w).max(1.0), "{w}x{h}");
            assert_eq!(f64::from(th), js(h).max(1.0), "{w}x{h}");
        }
    }

    #[test]
    fn never_drops_below_one_pixel() {
        assert_eq!(target_size(10_000, 1, 1600), (1600, 1));
        assert_eq!(target_size(1, 10_000, 1600), (1, 1600));
        assert_eq!(target_size(100, 50, 0), (1, 1));
    }

    #[test]
    fn huge_sides_do_not_overflow() {
        assert_eq!(target_size(u32::MAX, u32::MAX / 2, 1600), (1600, 800));
    }

    #[test]
    fn renames_the_last_extension() {
        assert_eq!(renamed("IMG_1234.JPG", "webp"), "IMG_1234.webp");
        assert_eq!(renamed("my.photo.png", "jpg"), "my.photo.jpg");
        assert_eq!(renamed("scan", "webp"), "scan.webp");
        assert_eq!(renamed(".jpg", "webp"), ".webp");
        assert_eq!(renamed("dot.", "webp"), "dot.webp");
    }
}
