//! Whole-source PNG qualification. Metadata discovery precedes native pixel
//! reconstruction. The shared allowance covers encoded input, not native
//! metadata, inflate, interlace-canvas or pipeline-worker storage.
use super::*;
use alloc::boxed::Box;
use core::fmt;
use zencodec::decode::{
    BatchMeta, BoxedError, ControlError, Decode, DecodeJob, DynControlledDecoder,
    InterpretationFacts, InterpretationState, InterpretationUnavailable, MetadataUpdate,
    StreamingDecode, erase_controlled,
};
use zencodec::source::AccessPattern;
use zencodec::storage::{StorageBudget, StorageError};
use zencodec_coroutine::{CoroutineDecoder, CoroutineError};

#[derive(Debug)]
enum ControlledError {
    Native(At<CodecError>),
    Coroutine(CoroutineError),
}
impl fmt::Display for ControlledError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native(e) => e.fmt(f),
            Self::Coroutine(e) => e.fmt(f),
        }
    }
}
impl core::error::Error for ControlledError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(match self {
            Self::Native(e) => e,
            Self::Coroutine(e) => e,
        })
    }
}
impl From<CoroutineError> for ControlledError {
    fn from(error: CoroutineError) -> Self {
        Self::Coroutine(error)
    }
}
impl From<StorageError> for ControlledError {
    fn from(error: StorageError) -> Self {
        Self::Coroutine(error.into())
    }
}
impl From<At<CodecError>> for ControlledError {
    fn from(error: At<CodecError>) -> Self {
        Self::Native(error)
    }
}
impl From<At<PngError>> for ControlledError {
    fn from(error: At<PngError>) -> Self {
        Self::Native(CodecError::of(error))
    }
}

pub(super) fn start(
    job: PngDecodeJob,
    _preferred: &[PixelDescriptor],
    storage: StorageBudget,
) -> Result<Box<DynControlledDecoder>, BoxedError> {
    if job.start_frame_index != 0 {
        return Err(Box::new(ControlError::NotControlled));
    }
    let source_storage = storage.clone();
    let decoder = CoroutineDecoder::controlled_with_storage(
        4096,
        AccessPattern::RandomAccess,
        storage,
        move |mut ctx| async move {
            let local_stop = job.stop.clone();
            let check_stop = || {
                if let Some(stop) = &local_stop {
                    enough::Stop::check(stop).map_err(|reason| at!(PngError::from(reason)))?;
                }
                Ok::<(), At<PngError>>(())
            };
            check_stop()?;
            let total = ctx.len().await?;
            let limits = *job.limits.as_ref().unwrap_or(&job.config.limits);
            limits
                .check_input_size(total)
                .map_err(|e| at!(PngError::LimitExceeded(e)))?;
            let len = usize::try_from(total).map_err(|_| StorageError::SizeOverflow)?;
            let mut input = source_storage.bytes(len)?;
            let header_len = len.min(33);
            ctx.read_exact_at(0, &mut input.as_mut_slice()[..header_len])
                .await?;
            let header = job.probe(&input.as_slice()[..header_len])?;
            limits
                .check_dimensions(header.width, header.height)
                .map_err(|e| at!(PngError::LimitExceeded(e)))?;
            ctx.set_info(header.clone());
            ctx.announce(MetadataUpdate::HeaderReady).await?;
            ctx.interpretation(InterpretationFacts::new(
                header.width,
                header.height,
                None,
                InterpretationState::Provisional,
            ))
            .await?;
            for (i, chunk) in input.as_mut_slice()[header_len..]
                .chunks_mut(4096)
                .enumerate()
            {
                check_stop()?;
                ctx.read_exact_at((header_len + i * 4096) as u64, chunk)
                    .await?;
            }
            let data = input.as_slice();
            check_progressive_policy(data, job.policy.as_ref())?;
            let full = job.probe(data)?;
            let output = job.output_info(data)?;
            let output_bytes = u64::from(output.width)
                .checked_mul(u64::from(output.height))
                .and_then(|n| n.checked_mul(output.native_format.bytes_per_pixel() as u64))
                .ok_or(StorageError::SizeOverflow)?;
            limits
                .check_output_size(output_bytes)
                .map_err(|e| at!(PngError::LimitExceeded(e)))?;
            let unavailable = inspect_color(data, &full, job.policy.as_ref(), &check_stop)?;
            let interlaced = full.is_progressive;
            ctx.set_info(full);
            ctx.set_output_descriptor(output.native_format);
            let mut facts = InterpretationFacts::new(
                output.width,
                output.height,
                Some(output.native_format),
                InterpretationState::Final,
            );
            if let Some(reason) = unavailable {
                facts = facts.with_unavailable(reason);
            }
            ctx.interpretation(facts).await?;
            // Worker creation and Adam7 full-canvas allocation both remain
            // beyond final adoption. Preferred output is a native fallback;
            // required descriptors are checked by the shared request adapter.
            check_stop()?;
            if interlaced {
                let image = job.decoder(Cow::Borrowed(data), &[])?.decode()?;
                if image.pixels().descriptor() != output.native_format {
                    return Err(at!(PngError::InvalidInput(
                        "controlled PNG output descriptor changed".into()
                    ))
                    .into());
                }
                for y in (0..output.height).step_by(64) {
                    check_stop()?;
                    let pixels = image.pixels();
                    let rows = pixels.sub_rows(y, 64.min(output.height - y));
                    ctx.write_rows(BatchMeta::final_rows(y), &rows).await?;
                }
            } else {
                let mut native = job.streaming_decoder(Cow::Borrowed(data), &[])?;
                while let Some((y, rows)) = native.next_batch()? {
                    if rows.descriptor() != output.native_format {
                        return Err(at!(PngError::InvalidInput(
                            "controlled PNG row descriptor changed".into()
                        ))
                        .into());
                    }
                    ctx.write_rows(BatchMeta::final_rows(y), &rows).await?;
                }
            }
            Ok::<_, ControlledError>(output)
        },
    )?;
    Ok(erase_controlled(decoder))
}

fn inspect_color(
    data: &[u8],
    full: &ImageInfo,
    policy: Option<&zencodec::decode::DecodePolicy>,
    stop: &impl Fn() -> Result<(), At<PngError>>,
) -> Result<Option<InterpretationUnavailable>, At<PngError>> {
    use InterpretationUnavailable as Why;
    let (mut idat, mut ended, mut icc, mut cicp, mut srgb, mut legacy) =
        (false, false, false, false, false, false);
    let mut reason = None;
    // Inspect physical carriers, including ones the lenient native probe skips.
    let mut chunks = crate::chunk::ChunkIter::new_with_config(data, true);
    while let Some(chunk) = chunks.next() {
        stop()?;
        let chunk = chunk?;
        let color = matches!(
            &chunk.chunk_type,
            b"iCCP" | b"cICP" | b"sRGB" | b"gAMA" | b"cHRM" | b"mDCV" | b"cLLI"
        );
        if color {
            let end = chunks.pos();
            let stored = u32::from_be_bytes(data[end - 4..end].try_into().unwrap());
            let actual = zenflate::crc32(zenflate::crc32(0, &chunk.chunk_type), chunk.data);
            if stored != actual {
                reason.get_or_insert(Why::IncompleteColorMetadata);
            }
        }
        if idat && color {
            reason = Some(Why::LateColorMetadata);
        }
        match &chunk.chunk_type {
            b"IDAT" => idat = true,
            b"IEND" => {
                ended = true;
                break;
            }
            b"iCCP" => {
                if icc {
                    reason.get_or_insert(Why::IncompleteColorMetadata);
                }
                icc = true;
            }
            b"cICP" => {
                if cicp
                    || chunk.data.len() != 4
                    || chunk.data.get(2) != Some(&0)
                    || chunk.data.get(3) != Some(&1)
                {
                    reason.get_or_insert(Why::IncompleteColorMetadata);
                }
                if chunk.data.len() == 4
                    && (zenpixels::TransferFunction::from_cicp(chunk.data[1]).is_none()
                        || zenpixels::ColorPrimaries::from_cicp(chunk.data[0]).is_none())
                {
                    reason.get_or_insert(Why::UnsupportedColor);
                }
                cicp = true;
            }
            b"sRGB" => {
                if srgb || chunk.data.len() != 1 || chunk.data[0] > 3 {
                    reason.get_or_insert(Why::IncompleteColorMetadata);
                }
                srgb = true;
            }
            b"gAMA" | b"cHRM" => legacy = true,
            _ => {}
        }
    }
    if !ended || !idat {
        return Err(at!(PngError::Truncated(
            "controlled PNG needs complete IDAT and IEND".into()
        )));
    }
    if icc && policy.is_some_and(|p| p.allow_icc == Some(false)) {
        reason.get_or_insert(Why::ForbiddenColorMetadata);
    } else if icc && full.source_color.icc_profile.is_none() {
        reason.get_or_insert(Why::IncompleteColorMetadata);
    }
    if legacy && !icc && !cicp && !srgb {
        reason.get_or_insert(Why::UnsupportedColor);
    }
    if !matches!(full.sequence, zencodec::ImageSequence::Single) {
        reason.get_or_insert(Why::UnqualifiedOutput);
    }
    Ok(reason)
}
