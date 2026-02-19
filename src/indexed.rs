//! Indexed (palette) PNG encoding via zenquant.

use alloc::vec::Vec;
use imgref::ImgRef;
use rgb::Rgba;

use zencodec_types::ImageMetadata;
use zenquant::{OutputFormat, QuantizeConfig};

use crate::encode::EncodeConfig;
use crate::error::PngError;
use crate::png_writer::{self, PngWriteMetadata};

/// Encode RGBA8 pixels to indexed PNG using zenquant for palette quantization.
///
/// Quantizes the image to at most 256 colors, then writes an indexed PNG
/// with PLTE and optional tRNS chunks. Uses multi-strategy filter selection
/// and zenflate compression for best file sizes.
pub fn encode_indexed_rgba8(
    img: ImgRef<Rgba<u8>>,
    encode_config: &EncodeConfig,
    quant_config: &QuantizeConfig,
    metadata: Option<&ImageMetadata<'_>>,
) -> Result<Vec<u8>, PngError> {
    let width = img.width() as u32;
    let height = img.height() as u32;
    let (buf, w, h) = img.to_contiguous_buf();

    let rgba_slice: &[zenquant::RGBA<u8>] = bytemuck::cast_slice(buf.as_ref());
    let result = zenquant::quantize_rgba(rgba_slice, w, h, quant_config)?;

    // Build separate RGB and alpha palette arrays
    let palette_rgba = result.palette_rgba();
    let mut palette_rgb = Vec::with_capacity(palette_rgba.len() * 3);
    let mut palette_alpha = Vec::with_capacity(palette_rgba.len());
    let mut has_transparency = false;

    for entry in palette_rgba {
        palette_rgb.push(entry[0]);
        palette_rgb.push(entry[1]);
        palette_rgb.push(entry[2]);
        palette_alpha.push(entry[3]);
        if entry[3] < 255 {
            has_transparency = true;
        }
    }

    let alpha = if has_transparency {
        Some(palette_alpha.as_slice())
    } else {
        None
    };

    let compression_level = encode_config.compression.to_zenflate_level();

    let mut write_meta = PngWriteMetadata::from_metadata(metadata);
    write_meta.source_gamma = encode_config.source_gamma;
    write_meta.srgb_intent = encode_config.srgb_intent;
    write_meta.chromaticities = encode_config.chromaticities;

    png_writer::write_indexed_png(
        result.indices(),
        width,
        height,
        &palette_rgb,
        alpha,
        &write_meta,
        compression_level,
    )
}

/// Create a default [`QuantizeConfig`] tuned for PNG output.
pub fn default_quantize_config() -> QuantizeConfig {
    QuantizeConfig::new(OutputFormat::Png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use imgref::ImgVec;

    fn test_image_4x4() -> ImgVec<Rgba<u8>> {
        let pixels = vec![
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
            Rgba {
                r: 255,
                g: 255,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
            Rgba {
                r: 255,
                g: 255,
                b: 0,
                a: 255,
            },
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 128,
            },
            Rgba {
                r: 0,
                g: 255,
                b: 0,
                a: 128,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 255,
                a: 128,
            },
            Rgba {
                r: 255,
                g: 255,
                b: 0,
                a: 128,
            },
            Rgba {
                r: 128,
                g: 128,
                b: 128,
                a: 255,
            },
            Rgba {
                r: 64,
                g: 64,
                b: 64,
                a: 255,
            },
            Rgba {
                r: 192,
                g: 192,
                b: 192,
                a: 255,
            },
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
        ];
        ImgVec::new(pixels, 4, 4)
    }

    #[test]
    fn roundtrip_indexed_png() {
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quant = default_quantize_config();

        let encoded = encode_indexed_rgba8(img.as_ref(), &config, &quant, None).unwrap();
        assert!(!encoded.is_empty());

        // Verify PNG signature
        assert_eq!(&encoded[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);

        // Full decode roundtrip through zenpng
        let decoded = crate::decode::decode(&encoded, None).unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);
    }

    #[test]
    fn roundtrip_with_metadata() {
        let img = test_image_4x4();
        let config = EncodeConfig::default();
        let quant = default_quantize_config();

        let fake_icc = vec![0x42u8; 200];
        let exif_data = b"Exif\0\0test_exif";
        let xmp_data = b"<x:xmpmeta>test</x:xmpmeta>";

        let meta = ImageMetadata::none()
            .with_icc(&fake_icc)
            .with_exif(exif_data)
            .with_xmp(xmp_data);

        let encoded = encode_indexed_rgba8(img.as_ref(), &config, &quant, Some(&meta)).unwrap();
        let decoded = crate::decode::decode(&encoded, None).unwrap();
        assert_eq!(decoded.info.width, 4);
        assert_eq!(decoded.info.height, 4);

        // ICC profile should roundtrip
        let icc = decoded.info.icc_profile.as_ref().expect("ICC missing");
        assert_eq!(icc.as_slice(), &fake_icc[..]);

        // EXIF should roundtrip
        let exif = decoded.info.exif.as_ref().expect("EXIF missing");
        assert_eq!(exif.as_slice(), exif_data);

        // XMP should roundtrip
        let xmp = decoded.info.xmp.as_ref().expect("XMP missing");
        assert_eq!(xmp.as_slice(), xmp_data);
    }

    #[test]
    fn all_compression_levels_work() {
        let img = test_image_4x4();
        let quant = default_quantize_config();

        for comp in [
            crate::Compression::None,
            crate::Compression::Fastest,
            crate::Compression::Fast,
            crate::Compression::Balanced,
            crate::Compression::High,
        ] {
            let config = EncodeConfig::default().with_compression(comp);
            let encoded = encode_indexed_rgba8(img.as_ref(), &config, &quant, None).unwrap();
            let decoded = crate::decode::decode(&encoded, None).unwrap();
            assert_eq!(decoded.info.width, 4);
            assert_eq!(decoded.info.height, 4);
        }
    }
}
