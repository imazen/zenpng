# zenpng

PNG encoding and decoding with `zencodec-types` trait integration.

Wraps the `png` crate (0.18) with typed pixel buffers (`imgref` + `rgb`), metadata roundtrip (ICC/EXIF/XMP, gAMA/sRGB/cHRM/cICP/mDCV/cLLI chunks), and optional palette quantization via `zenquant`.

## Features

- 8-bit and 16-bit PNG support (truecolor and indexed)
- gAMA/sRGB/cHRM/cICP chunk roundtrip
- Optional indexed PNG with `zenquant` palette quantization
- Custom indexed PNG writer using `zenflate` compression
- `zencodec-types` trait integration for the unified codec pipeline

## License

AGPL-3.0-or-later
