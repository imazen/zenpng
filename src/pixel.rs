//! PNG's defaults at the pixel interchange boundary.

use whereat::{At, at};
use zenpixels::{
    AlphaMode, ColorSpace, ImageSpec, PixelType, PixelView, PixelViewMut, SampleLayout, SampleType,
};

use crate::error::PngError;

pub(crate) const fn default_spec(layout: SampleLayout) -> ImageSpec {
    let alpha = if layout.order.has_alpha_slot() {
        AlphaMode::Straight
    } else if layout.order.has_padding_slot() {
        AlphaMode::Undefined
    } else {
        AlphaMode::None
    };
    let mut color = match (layout.order.is_gray_family(), layout.sample) {
        (true, SampleType::F32) => ColorSpace::GRAY_LINEAR,
        (true, _) => ColorSpace::GRAY_SRGB,
        (false, SampleType::F32) => ColorSpace::LINEAR_SRGB,
        (false, _) => ColorSpace::SRGB,
    };
    match &mut color {
        ColorSpace::Rgb { alpha: mode, .. } | ColorSpace::Gray { alpha: mode, .. } => *mode = alpha,
        _ => unreachable!(),
    }
    ImageSpec::new(layout, color)
}

pub(crate) fn view_from_imgref<P: PixelType>(
    image: imgref::ImgRef<'_, P>,
) -> Result<PixelView<'_>, At<PngError>> {
    let width =
        u32::try_from(image.width()).map_err(|e| at!(PngError::InvalidBuffer(e.to_string())))?;
    let height =
        u32::try_from(image.height()).map_err(|e| at!(PngError::InvalidBuffer(e.to_string())))?;
    let stride = image
        .stride()
        .checked_mul(core::mem::size_of::<P>())
        .ok_or_else(|| {
            at!(PngError::InvalidBuffer(
                "image stride overflows bytes".into()
            ))
        })?;
    PixelView::new_with_spec_cow(
        bytemuck::cast_slice(image.buf()),
        width,
        height,
        stride,
        zenpixels::SpecCow::from_owned(default_spec(P::LAYOUT)),
    )
    .map_err(|e| at!(PngError::InvalidBuffer(e.to_string())))
}

pub(crate) fn view_from_imgref_mut<P: PixelType>(
    image: imgref::ImgRefMut<'_, P>,
) -> Result<PixelViewMut<'_>, At<PngError>> {
    let width =
        u32::try_from(image.width()).map_err(|e| at!(PngError::InvalidBuffer(e.to_string())))?;
    let height =
        u32::try_from(image.height()).map_err(|e| at!(PngError::InvalidBuffer(e.to_string())))?;
    let stride = image
        .stride()
        .checked_mul(core::mem::size_of::<P>())
        .ok_or_else(|| {
            at!(PngError::InvalidBuffer(
                "image stride overflows bytes".into()
            ))
        })?;
    PixelViewMut::new_with_spec_cow(
        bytemuck::cast_slice_mut(image.into_buf()),
        width,
        height,
        stride,
        zenpixels::SpecCow::from_owned(default_spec(P::LAYOUT)),
    )
    .map_err(|e| at!(PngError::InvalidBuffer(e.to_string())))
}
