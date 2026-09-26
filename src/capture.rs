//! Renderer-independent normalization for captured frames.

/// Converts a four-channel GPU readback into tightly packed, top-left-origin
/// RGBA pixels. Returns `None` when the dimensions do not fit the supplied
/// buffer.
pub fn rgba_pixels(
    width: usize,
    height: usize,
    stride: usize,
    pixels: &[u8],
    bgra: bool,
    bottom_left: bool,
) -> Option<Vec<u8>> {
    let row_bytes = width.checked_mul(4)?;
    if stride < row_bytes || stride.checked_mul(height)? > pixels.len() {
        return None;
    }
    let len = row_bytes.checked_mul(height)?;
    let mut rgba = vec![0; len];
    for y in 0..height {
        let src_y = if bottom_left { height - 1 - y } else { y };
        let src = &pixels[src_y * stride..src_y * stride + row_bytes];
        let dst = &mut rgba[y * row_bytes..(y + 1) * row_bytes];
        dst.copy_from_slice(src);
        if bgra {
            for pixel in dst.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
    }
    Some(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_rows_are_swapped_and_tightened() {
        assert_eq!(
            rgba_pixels(
                2,
                2,
                12,
                &[3, 2, 1, 4, 7, 6, 5, 8, 0, 0, 0, 0, 11, 10, 9, 12, 15, 14, 13, 16, 0, 0, 0, 0,],
                true,
                false,
            ),
            Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
        );
    }

    #[test]
    fn bottom_left_rows_are_flipped() {
        assert_eq!(
            rgba_pixels(1, 2, 4, &[1, 2, 3, 4, 5, 6, 7, 8], false, true),
            Some(vec![5, 6, 7, 8, 1, 2, 3, 4])
        );
    }

    #[test]
    fn rejects_short_or_narrow_buffers() {
        assert_eq!(rgba_pixels(2, 1, 7, &[0; 8], false, false), None);
        assert_eq!(rgba_pixels(2, 2, 8, &[0; 15], false, false), None);
    }
}
