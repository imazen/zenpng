//! Shared packed-pixel conversions for the command-line utility and examples.

use zenpixels::{PeakSource, PixelBuf, PixelFormat, PixelType};
use zenpixels_convert::{ConvertPolicy, convert_to_buf_with_peak};

pub fn to_pixels<P: PixelType>(pixels: &PixelBuf, spec: PixelFormat) -> imgref::ImgVec<P> {
    let view = pixels.as_view();
    if spec.layout == view.format().layout {
        let image = view.as_imgref::<P>().expect("packed PNG samples");
        return imgref::ImgVec::new(image.pixels().collect(), image.width(), image.height());
    }
    // These callers inspect or re-encode the input's existing code values.
    let nominal = if view.format().layout.order.is_gray_family() {
        zenpixels::ColorSpace::GRAY_SRGB
    } else {
        zenpixels::ColorSpace::SRGB
    };
    let nominal = nominal.with_alpha(view.format().color.alpha());
    let nominal_spec = PixelFormat::new(view.format().layout, nominal);
    let view = zenpixels::PixelView::new(
        view.as_strided_bytes(),
        view.width(),
        view.rows(),
        view.stride(),
        &nominal_spec,
    )
    .expect("native PNG samples");
    convert_to_buf_with_peak(
        &view,
        spec,
        &ConvertPolicy::permissive(),
        PeakSource::Declared,
    )
    .expect("PNG layout conversion")
    .into_imgvec()
    .expect("converted PNG sample layout")
}
