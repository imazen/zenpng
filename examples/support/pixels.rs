//! Shared packed-pixel conversions for the command-line utility and examples.

use zenpixels::{ImageSpec, PeakSource, PixelBuf, PixelType};
use zenpixels_convert::{ConvertPolicy, convert_buf};

pub fn to_pixels<P: PixelType>(pixels: &PixelBuf, spec: ImageSpec) -> imgref::ImgVec<P> {
    let view = pixels.as_view();
    if spec.layout == view.spec().layout {
        let image = view.try_as_imgref::<P>().expect("packed PNG samples");
        return imgref::ImgVec::new(image.pixels().collect(), image.width(), image.height());
    }
    // These callers inspect or re-encode the input's existing code values.
    let nominal = if view.spec().layout.order.is_gray_family() {
        zenpixels::ColorSpace::GRAY_SRGB
    } else {
        zenpixels::ColorSpace::SRGB
    };
    let nominal = nominal.with_alpha(view.spec().color.alpha());
    let view = zenpixels::PixelView::new(
        view.as_strided_bytes(),
        view.width(),
        view.rows(),
        view.stride(),
        ImageSpec::new(view.spec().layout, nominal),
    )
    .expect("native PNG samples");
    convert_buf(
        &view,
        spec,
        &ConvertPolicy::permissive(),
        PeakSource::Declared,
    )
    .expect("PNG layout conversion")
    .try_into_imgvec()
    .expect("converted PNG sample layout")
}
