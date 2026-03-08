//! IHDR chunk parsing and validation.

use crate::error::PngError;

/// Parsed IHDR chunk.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub(crate) struct Ihdr {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub interlace: u8,
}

impl Ihdr {
    /// Parse IHDR from chunk data (must be exactly 13 bytes).
    pub fn parse(data: &[u8]) -> Result<Self, PngError> {
        if data.len() != 13 {
            return Err(PngError::Decode(alloc::format!(
                "IHDR chunk is {} bytes, expected 13",
                data.len()
            )));
        }

        let width = u32::from_be_bytes(data[0..4].try_into().unwrap());
        let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
        let bit_depth = data[8];
        let color_type = data[9];
        let compression = data[10];
        let filter = data[11];
        let interlace = data[12];

        if width == 0 || height == 0 {
            return Err(PngError::Decode("IHDR: zero dimension".into()));
        }

        if compression != 0 {
            return Err(PngError::Decode(alloc::format!(
                "IHDR: unknown compression method {}",
                compression
            )));
        }
        if filter != 0 {
            return Err(PngError::Decode(alloc::format!(
                "IHDR: unknown filter method {}",
                filter
            )));
        }
        if interlace > 1 {
            return Err(PngError::Decode(alloc::format!(
                "IHDR: unknown interlace method {}",
                interlace
            )));
        }

        let ihdr = Self {
            width,
            height,
            bit_depth,
            color_type,
            interlace,
        };
        ihdr.validate()?;
        Ok(ihdr)
    }

    /// Validate color_type / bit_depth combination per PNG spec.
    fn validate(&self) -> Result<(), PngError> {
        let valid = match self.color_type {
            0 => matches!(self.bit_depth, 1 | 2 | 4 | 8 | 16), // Grayscale
            2 => matches!(self.bit_depth, 8 | 16),             // RGB
            3 => matches!(self.bit_depth, 1 | 2 | 4 | 8),      // Indexed
            4 => matches!(self.bit_depth, 8 | 16),             // GrayAlpha
            6 => matches!(self.bit_depth, 8 | 16),             // RGBA
            _ => false,
        };
        if !valid {
            return Err(PngError::Decode(alloc::format!(
                "invalid color_type={} bit_depth={} combination",
                self.color_type,
                self.bit_depth
            )));
        }
        Ok(())
    }

    /// Number of channels for this color type.
    pub fn channels(&self) -> usize {
        match self.color_type {
            0 => 1, // Grayscale
            2 => 3, // RGB
            3 => 1, // Indexed (palette index)
            4 => 2, // GrayAlpha
            6 => 4, // RGBA
            _ => unreachable!("validated in parse"),
        }
    }

    /// Bytes per pixel for the filter unit (bpp), minimum 1.
    /// For sub-8-bit depths, this is 1.
    pub fn filter_bpp(&self) -> usize {
        let bits_per_pixel = self.channels() * self.bit_depth as usize;
        bits_per_pixel.div_ceil(8)
    }

    /// Raw row bytes (unfiltered row data, not including filter byte).
    /// For sub-8-bit depths, accounts for bit packing.
    pub fn raw_row_bytes(&self) -> usize {
        let bits_per_row = self.width as usize * self.channels() * self.bit_depth as usize;
        bits_per_row.div_ceil(8)
    }

    /// Stride = 1 (filter byte) + raw_row_bytes.
    pub fn stride(&self) -> usize {
        1 + self.raw_row_bytes()
    }

    /// Whether the image uses sub-8-bit depth (1, 2, or 4).
    pub fn is_sub_byte(&self) -> bool {
        self.bit_depth < 8
    }

    /// Whether this is a palette-indexed image.
    pub fn is_indexed(&self) -> bool {
        self.color_type == 3
    }

    /// Whether the source has an alpha channel (color type 4 or 6).
    pub fn has_alpha(&self) -> bool {
        self.color_type == 4 || self.color_type == 6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ihdr(w: u32, h: u32, bit_depth: u8, color_type: u8, interlace: u8) -> Vec<u8> {
        let mut data = Vec::with_capacity(13);
        data.extend_from_slice(&w.to_be_bytes());
        data.extend_from_slice(&h.to_be_bytes());
        data.push(bit_depth);
        data.push(color_type);
        data.push(0); // compression
        data.push(0); // filter
        data.push(interlace);
        data
    }

    #[test]
    fn parse_valid_rgb8() {
        let ihdr = Ihdr::parse(&make_ihdr(100, 200, 8, 2, 0)).unwrap();
        assert_eq!(ihdr.width, 100);
        assert_eq!(ihdr.height, 200);
        assert_eq!(ihdr.bit_depth, 8);
        assert_eq!(ihdr.color_type, 2);
        assert_eq!(ihdr.interlace, 0);
    }

    #[test]
    fn parse_valid_rgba16() {
        let ihdr = Ihdr::parse(&make_ihdr(50, 50, 16, 6, 0)).unwrap();
        assert_eq!(ihdr.channels(), 4);
        assert_eq!(ihdr.filter_bpp(), 8);
    }

    #[test]
    fn parse_valid_indexed_4bit() {
        let ihdr = Ihdr::parse(&make_ihdr(10, 10, 4, 3, 0)).unwrap();
        assert_eq!(ihdr.channels(), 1);
        assert!(ihdr.is_sub_byte());
        assert!(ihdr.is_indexed());
    }

    #[test]
    fn parse_valid_interlaced() {
        let ihdr = Ihdr::parse(&make_ihdr(100, 100, 8, 2, 1)).unwrap();
        assert_eq!(ihdr.interlace, 1);
    }

    #[test]
    fn parse_wrong_length() {
        assert!(Ihdr::parse(&[0; 12]).is_err());
        assert!(Ihdr::parse(&[0; 14]).is_err());
    }

    #[test]
    fn parse_zero_dimensions() {
        assert!(Ihdr::parse(&make_ihdr(0, 100, 8, 2, 0)).is_err());
        assert!(Ihdr::parse(&make_ihdr(100, 0, 8, 2, 0)).is_err());
    }

    #[test]
    fn parse_bad_compression() {
        let mut data = make_ihdr(1, 1, 8, 2, 0);
        data[10] = 1; // invalid compression
        assert!(Ihdr::parse(&data).is_err());
    }

    #[test]
    fn parse_bad_filter() {
        let mut data = make_ihdr(1, 1, 8, 2, 0);
        data[11] = 1; // invalid filter
        assert!(Ihdr::parse(&data).is_err());
    }

    #[test]
    fn parse_bad_interlace() {
        assert!(Ihdr::parse(&make_ihdr(1, 1, 8, 2, 2)).is_err());
    }

    #[test]
    fn parse_invalid_color_bit_depth() {
        // RGB with bit_depth=4 is invalid
        assert!(Ihdr::parse(&make_ihdr(1, 1, 4, 2, 0)).is_err());
        // Indexed with bit_depth=16 is invalid
        assert!(Ihdr::parse(&make_ihdr(1, 1, 16, 3, 0)).is_err());
        // Unknown color type
        assert!(Ihdr::parse(&make_ihdr(1, 1, 8, 5, 0)).is_err());
    }

    #[test]
    fn channels_all_types() {
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 8, 0, 0)).unwrap().channels(),
            1
        );
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 8, 2, 0)).unwrap().channels(),
            3
        );
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 8, 3, 0)).unwrap().channels(),
            1
        );
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 8, 4, 0)).unwrap().channels(),
            2
        );
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 8, 6, 0)).unwrap().channels(),
            4
        );
    }

    #[test]
    fn filter_bpp_values() {
        // Gray 1-bit: 1 bit/pixel → bpp=1
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 1, 0, 0)).unwrap().filter_bpp(),
            1
        );
        // RGB 8-bit: 24 bits/pixel → bpp=3
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 8, 2, 0)).unwrap().filter_bpp(),
            3
        );
        // RGBA 16-bit: 64 bits/pixel → bpp=8
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 16, 6, 0))
                .unwrap()
                .filter_bpp(),
            8
        );
        // GrayAlpha 8-bit: 16 bits/pixel → bpp=2
        assert_eq!(
            Ihdr::parse(&make_ihdr(1, 1, 8, 4, 0)).unwrap().filter_bpp(),
            2
        );
    }

    #[test]
    fn raw_row_bytes_and_stride() {
        // RGB 8-bit, width=10: 30 bytes/row, stride=31
        let ihdr = Ihdr::parse(&make_ihdr(10, 1, 8, 2, 0)).unwrap();
        assert_eq!(ihdr.raw_row_bytes(), 30);
        assert_eq!(ihdr.stride(), 31);

        // Gray 1-bit, width=10: ceil(10/8) = 2 bytes/row
        let ihdr = Ihdr::parse(&make_ihdr(10, 1, 1, 0, 0)).unwrap();
        assert_eq!(ihdr.raw_row_bytes(), 2);
    }

    #[test]
    fn is_sub_byte() {
        assert!(
            Ihdr::parse(&make_ihdr(1, 1, 1, 0, 0))
                .unwrap()
                .is_sub_byte()
        );
        assert!(
            Ihdr::parse(&make_ihdr(1, 1, 2, 0, 0))
                .unwrap()
                .is_sub_byte()
        );
        assert!(
            Ihdr::parse(&make_ihdr(1, 1, 4, 0, 0))
                .unwrap()
                .is_sub_byte()
        );
        assert!(
            !Ihdr::parse(&make_ihdr(1, 1, 8, 0, 0))
                .unwrap()
                .is_sub_byte()
        );
    }

    #[test]
    fn has_alpha_types() {
        assert!(!Ihdr::parse(&make_ihdr(1, 1, 8, 0, 0)).unwrap().has_alpha());
        assert!(!Ihdr::parse(&make_ihdr(1, 1, 8, 2, 0)).unwrap().has_alpha());
        assert!(!Ihdr::parse(&make_ihdr(1, 1, 8, 3, 0)).unwrap().has_alpha());
        assert!(Ihdr::parse(&make_ihdr(1, 1, 8, 4, 0)).unwrap().has_alpha());
        assert!(Ihdr::parse(&make_ihdr(1, 1, 8, 6, 0)).unwrap().has_alpha());
    }

    #[test]
    fn parse_rejects_dimension_exceeding_png_spec_max() {
        // PNG spec maximum dimension is 2^31 - 1
        let max_plus_one = 0x8000_0000u32; // 2^31
        assert!(Ihdr::parse(&make_ihdr(max_plus_one, 1, 8, 0, 0)).is_err());
        assert!(Ihdr::parse(&make_ihdr(1, max_plus_one, 8, 0, 0)).is_err());
        // u32::MAX is also invalid
        assert!(Ihdr::parse(&make_ihdr(u32::MAX, 1, 8, 0, 0)).is_err());
        assert!(Ihdr::parse(&make_ihdr(1, u32::MAX, 8, 0, 0)).is_err());
    }

    #[test]
    fn parse_accepts_png_spec_max_dimension_grayscale() {
        // 2^31 - 1 is the PNG spec maximum; grayscale 8-bit has 1 byte/pixel
        // so row bytes = 2^31 - 1 which fits in u32/usize on all platforms.
        let png_max = 0x7FFF_FFFFu32;
        let result = Ihdr::parse(&make_ihdr(png_max, 1, 8, 0, 0));
        assert!(result.is_ok());
    }

    #[test]
    fn parse_rejects_row_bytes_overflow() {
        // width=536870912 (0x2000_0000), RGBA (4 channels), 16-bit depth
        // bits_per_row = 536870912 * 4 * 16 = 34,359,738,368 which exceeds u32::MAX.
        // This must be rejected to prevent usize overflow on wasm32.
        let width = 536_870_912u32;
        let result = Ihdr::parse(&make_ihdr(width, 1, 16, 6, 0));
        assert!(
            result.is_err(),
            "should reject dimensions that overflow row bytes on 32-bit"
        );
    }

    #[test]
    fn parse_rejects_large_rgba16_width() {
        // Even at PNG spec max width, RGBA 16-bit = 2^31-1 * 4 * 16 / 8 = ~16 GiB per row.
        // This overflows u32 and should be rejected for portability (wasm32).
        let png_max = 0x7FFF_FFFFu32;
        let result = Ihdr::parse(&make_ihdr(png_max, 1, 16, 6, 0));
        assert!(
            result.is_err(),
            "RGBA 16-bit at max width overflows 32-bit row bytes"
        );
    }
}
