//! zennode node definitions for PNG encoding.
//!
//! Defines [`EncodePng`] with RIAPI-compatible querystring keys matching
//! imageflow's established PNG encoding parameters.

extern crate alloc;

use zennode::*;

/// PNG encoding with quality, lossless mode, and compression options.
///
/// Matches imageflow's RIAPI keys: `png.quality`, `png.min_quality`,
/// `png.lossless`, `png.max_deflate`.
///
/// JSON API: `{ "quality": 85, "lossless": true, "max_deflate": false }`
/// RIAPI: `?png.quality=85&png.lossless=true&png.max_deflate=true`
#[derive(Node, Clone, Debug)]
#[node(id = "zenpng.encode", group = Encode, role = Encode)]
#[node(tags("codec", "png", "lossless", "encode"))]
pub struct EncodePng {
    /// Generic quality 0-100 (mapped via with_generic_quality at execution time).
    ///
    /// When set (>= 0), this value is passed through zencodec's
    /// `with_generic_quality()` which maps it to the codec's native
    /// quality scale. Use this for uniform quality across all codecs.
    #[param(range(0..=100), default = -1, step = 1)]
    #[param(unit = "", section = "Main", label = "Quality")]
    #[kv("quality")]
    pub quality: i32,

    /// Codec-specific PNG quality override (0-100).
    ///
    /// Controls quantization quality when `lossless` is false.
    /// Higher values produce better quality but larger files.
    /// When set (>= 0), takes precedence over the generic `quality` field.
    /// When `lossless` is true, this is ignored.
    #[param(range(0..=100), default = -1, step = 1)]
    #[param(unit = "", section = "Main", label = "PNG Quality")]
    #[kv("png.quality")]
    pub png_quality: i32,

    /// Minimum acceptable quality for lossy encoding (0-100).
    ///
    /// When non-zero, the encoder will not quantize below this
    /// quality level, falling back to lossless if the target
    /// quality would be too low. Used as a quality floor.
    #[param(range(0..=100), default = 0, step = 1)]
    #[param(unit = "", section = "Main", label = "Min Quality")]
    #[kv("png.min_quality")]
    pub min_quality: i32,

    /// Use lossless PNG encoding (no quantization).
    ///
    /// When true, pixels are encoded without any lossy
    /// transformation. When false, palette quantization may
    /// be applied to reduce file size.
    #[param(default = true)]
    #[param(section = "Main")]
    #[kv("png.lossless")]
    pub lossless: bool,

    /// Use maximum DEFLATE compression effort.
    ///
    /// When true, uses the highest compression effort for
    /// smallest possible file size at the cost of much slower
    /// encoding. Maps to zenpng's `Compression::Crush` level.
    #[param(default = false)]
    #[param(section = "Advanced")]
    #[kv("png.max_deflate")]
    pub max_deflate: bool,
}

impl Default for EncodePng {
    fn default() -> Self {
        Self {
            quality: -1,
            png_quality: -1,
            min_quality: 0,
            lossless: true,
            max_deflate: false,
        }
    }
}

impl EncodePng {
    /// Apply this node's explicitly-set params on top of an existing config.
    ///
    /// Fields at their default/sentinel value are skipped:
    /// - `quality` and `png_quality`: `-1` means not set
    /// - `min_quality`: `0` means not set
    /// - `lossless`: `true` is the default (PNG is lossless by default)
    /// - `max_deflate`: `false` means not set
    ///
    /// Codec-specific `png_quality` is applied AFTER generic `quality`,
    /// so it takes precedence when both are set.
    pub fn apply(
        &self,
        mut config: crate::codec::PngEncoderConfig,
    ) -> crate::codec::PngEncoderConfig {
        use zencodec::encode::EncoderConfig as _;

        // Generic quality first
        if self.quality >= 0 {
            config = config.with_generic_quality(self.quality as f32);
        }
        // Codec-specific quality override
        if self.png_quality >= 0 {
            config = config.with_generic_quality(self.png_quality as f32);
        }
        // Lossless mode (PNG default is true; only apply when explicitly false)
        if !self.lossless {
            config = config.with_lossless(false);
        }
        // Max deflate -> Crush compression
        if self.max_deflate {
            config = config.with_compression(crate::Compression::Crush);
        }
        // min_quality is stored on the node for pipeline use but does not
        // map to a PngEncoderConfig setter — it is used at execution time
        // to decide whether to fall back to lossless when quantization
        // quality would be too low.
        config
    }

    /// Build a config from scratch using only this node's params.
    pub fn to_encoder_config(&self) -> crate::codec::PngEncoderConfig {
        self.apply(crate::codec::PngEncoderConfig::new())
    }
}

/// Registration function for aggregating crates.
pub fn register(registry: &mut NodeRegistry) {
    registry.register(&ENCODE_PNG_NODE);
}

/// All PNG zennode definitions.
pub static ALL: &[&dyn NodeDef] = &[&ENCODE_PNG_NODE];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_metadata() {
        let schema = ENCODE_PNG_NODE.schema();
        assert_eq!(schema.id, "zenpng.encode");
        assert_eq!(schema.group, NodeGroup::Encode);
        assert_eq!(schema.role, NodeRole::Encode);
        assert!(schema.tags.contains(&"png"));
        assert!(schema.tags.contains(&"lossless"));
        assert!(schema.tags.contains(&"codec"));
        assert!(schema.tags.contains(&"encode"));
    }

    #[test]
    fn param_count_and_names() {
        let schema = ENCODE_PNG_NODE.schema();
        let names: Vec<&str> = schema.params.iter().map(|p| p.name).collect();
        assert!(names.contains(&"quality"));
        assert!(names.contains(&"png_quality"));
        assert!(names.contains(&"min_quality"));
        assert!(names.contains(&"lossless"));
        assert!(names.contains(&"max_deflate"));
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn defaults() {
        let node = ENCODE_PNG_NODE.create_default().unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::I32(-1)));
        assert_eq!(node.get_param("png_quality"), Some(ParamValue::I32(-1)));
        assert_eq!(node.get_param("min_quality"), Some(ParamValue::I32(0)));
        assert_eq!(node.get_param("lossless"), Some(ParamValue::Bool(true)));
        assert_eq!(node.get_param("max_deflate"), Some(ParamValue::Bool(false)));
    }

    #[test]
    fn from_kv_png_quality() {
        let mut kv = KvPairs::from_querystring("png.quality=92&png.lossless=false");
        let node = ENCODE_PNG_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("png_quality"), Some(ParamValue::I32(92)));
        assert_eq!(node.get_param("lossless"), Some(ParamValue::Bool(false)));
        assert_eq!(kv.unconsumed().count(), 0);
    }

    #[test]
    fn from_kv_generic_quality() {
        // "quality" sets the generic quality field
        let mut kv = KvPairs::from_querystring("quality=75");
        let node = ENCODE_PNG_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::I32(75)));
        // png_quality remains unset
        assert_eq!(node.get_param("png_quality"), Some(ParamValue::I32(-1)));
    }

    #[test]
    fn from_kv_both_qualities() {
        // Both generic and codec-specific can be set independently
        let mut kv = KvPairs::from_querystring("quality=80&png.quality=92");
        let node = ENCODE_PNG_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::I32(80)));
        assert_eq!(node.get_param("png_quality"), Some(ParamValue::I32(92)));
        assert_eq!(kv.unconsumed().count(), 0);
    }

    #[test]
    fn from_kv_min_quality() {
        let mut kv = KvPairs::from_querystring("png.quality=90&png.min_quality=40");
        let node = ENCODE_PNG_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("min_quality"), Some(ParamValue::I32(40)));
    }

    #[test]
    fn from_kv_max_deflate() {
        let mut kv = KvPairs::from_querystring("png.max_deflate=true&png.lossless=true");
        let node = ENCODE_PNG_NODE.from_kv(&mut kv).unwrap().unwrap();
        assert_eq!(node.get_param("max_deflate"), Some(ParamValue::Bool(true)));
        assert_eq!(node.get_param("lossless"), Some(ParamValue::Bool(true)));
    }

    #[test]
    fn from_kv_no_match() {
        let mut kv = KvPairs::from_querystring("w=800&h=600");
        let result = ENCODE_PNG_NODE.from_kv(&mut kv).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn json_round_trip() {
        let mut params = ParamMap::new();
        params.insert("quality".into(), ParamValue::I32(80));
        params.insert("png_quality".into(), ParamValue::I32(92));
        params.insert("lossless".into(), ParamValue::Bool(false));
        params.insert("min_quality".into(), ParamValue::I32(40));
        params.insert("max_deflate".into(), ParamValue::Bool(true));

        let node = ENCODE_PNG_NODE.create(&params).unwrap();
        assert_eq!(node.get_param("quality"), Some(ParamValue::I32(80)));
        assert_eq!(node.get_param("png_quality"), Some(ParamValue::I32(92)));
        assert_eq!(node.get_param("lossless"), Some(ParamValue::Bool(false)));
        assert_eq!(node.get_param("min_quality"), Some(ParamValue::I32(40)));
        assert_eq!(node.get_param("max_deflate"), Some(ParamValue::Bool(true)));

        // Round-trip
        let exported = node.to_params();
        let node2 = ENCODE_PNG_NODE.create(&exported).unwrap();
        assert_eq!(node2.get_param("quality"), Some(ParamValue::I32(80)));
        assert_eq!(node2.get_param("png_quality"), Some(ParamValue::I32(92)));
        assert_eq!(node2.get_param("lossless"), Some(ParamValue::Bool(false)));
    }

    #[test]
    fn downcast_to_concrete() {
        let node = ENCODE_PNG_NODE.create_default().unwrap();
        let enc = node.as_any().downcast_ref::<EncodePng>().unwrap();
        assert_eq!(enc.quality, -1);
        assert_eq!(enc.png_quality, -1);
        assert!(enc.lossless);
        assert!(!enc.max_deflate);
        assert_eq!(enc.min_quality, 0);
    }

    #[test]
    fn to_encoder_config_defaults() {
        let node = EncodePng::default();
        let _config = node.to_encoder_config();
    }

    #[test]
    fn apply_generic_quality_only() {
        let mut node = EncodePng::default();
        node.quality = 80;
        node.lossless = false;
        let config = node.to_encoder_config();
        let q = zencodec::encode::EncoderConfig::generic_quality(&config);
        assert!(q.is_some());
    }

    #[test]
    fn apply_codec_specific_overrides_generic() {
        let mut node = EncodePng::default();
        node.quality = 50;
        node.png_quality = 90;
        node.lossless = false;
        let config = node.to_encoder_config();
        let q = zencodec::encode::EncoderConfig::generic_quality(&config);
        assert_eq!(q, Some(90.0));
    }

    #[test]
    fn apply_preserves_existing_config() {
        let base = crate::codec::PngEncoderConfig::new()
            .with_compression(crate::Compression::Fast);
        let node = EncodePng::default();
        let _config = node.apply(base);
    }

    #[test]
    fn apply_max_deflate() {
        let mut node = EncodePng::default();
        node.max_deflate = true;
        let _config = node.to_encoder_config();
    }

    #[test]
    fn apply_lossy_mode() {
        let mut node = EncodePng::default();
        node.lossless = false;
        node.quality = 85;
        let config = node.to_encoder_config();
        let lossless = zencodec::encode::EncoderConfig::is_lossless(&config);
        assert_eq!(lossless, Some(false));
    }

    #[test]
    fn registry_integration() {
        let mut registry = NodeRegistry::new();
        register(&mut registry);
        assert!(registry.get("zenpng.encode").is_some());

        // png.quality triggers codec-specific path
        let result = registry.from_querystring("png.quality=80&png.lossless=false");
        assert_eq!(result.instances.len(), 1);
        assert_eq!(result.instances[0].schema().id, "zenpng.encode");

        // generic quality also triggers the node
        let result2 = registry.from_querystring("quality=80");
        assert_eq!(result2.instances.len(), 1);
        assert_eq!(result2.instances[0].schema().id, "zenpng.encode");
    }
}
