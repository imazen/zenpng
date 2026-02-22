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
