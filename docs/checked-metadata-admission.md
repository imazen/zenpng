# Checked PNG metadata admission

The PNG zencodec job accepts policy-filtered metadata as a retention obligation
through the checked setter. Later metadata and policy setters must not remove or
replace required fields. The adapter checks the effective carriers before a
still or row header is produced, and before an animation header is produced.
An explicit checked replacement can replace or clear the generic record. A later
best-effort metadata replacement must match that record, so it cannot reinsert
fields discarded by the checked policy. Pixel color binding can still add
format-required color signaling.

PNG carries ICC, TIFF-form EXIF, nonempty UTF-8 XMP, cICP, whole-nit content
light level, and mastering display volume alongside cICP. A mastering value is
accepted only when the actual mDCV writer and reader round trip every floating
value bit-for-bit. PNG has no diffuse-white carrier. Empty or invalid carriers, ICC profiles over the native decoder's 1 MiB
limit, APP1-prefixed EXIF, nonzero cICP matrix, and conflicting native cICP or HDR
settings are refused. Raw HDR overrides are conservatively refused when the
corresponding generic field is required. A policy that discards cICP or HDR is
refused when an earlier native configuration would still write that field.
Required ICC and cICP default to Verbatim when color policy is unspecified;
explicit conflicting policies refuse. An f32 input transformed to sRGB cannot
retain a conflicting linear color signal.

This contract covers the generic retained record. Native ancillary settings and
opaque text or unknown chunks can contain additional data: clearing generic
metadata does not certify the entire PNG as private or empty. It does not bake
orientation, and it does not promise a change to requested pixel conversion,
quantization or transparent-sample settings.

## Qualification and limits

On aarch64 Linux, source `f1f2cd62` passed the 12 checked-metadata controls.
They exercise the shared testkit's completed-output readback and independent
chunk/sample checks, one-shot, rows, the shared pull-row driver, animation and
erased jobs, replacement/clear and refusals. Native `encode_from` remains
unsupported and has an explicit refusal control.

Source `59c377f3` differs from `f1f2cd62` only in two lint compatibility edits.
It passed strict all-target Clippy with `inventory,metadata-rewrite,controlled-decode`.
Five complete integration targets then passed serially: `integration` 66/66,
`orientation_carriage` 3/3, `row_planning` 7/7, `shared_contracts` 4/4 and
`source_color_binding` 5/5. Cargo reused Fresh artifacts from the same 59
source path; the earlier verbose broad run recorded all five targets compiling
from that path. Probe parity reported 68 PNGs checked.

The broad run including library tests ended before a final result after the
3 GiB scope was OOM-killed. Its APNG comparison logged differences, but the
test compares composited zenpng canvases with raw image-png subframes and is
not a valid composed-frame oracle. Those diagnostics do not establish whether
the native pixels are correct. The run also logged missing discover, Kodak and
image-png corpus roots; its staged generic corpus root contained only the CLIC
fixture. Library, APNG corpus and full external corpus coverage therefore
remain unqualified. No release qualification or performance claim follows.

The tested lock is a qualification artifact; it does not establish that an
unlocked downstream graph selects identical package versions. The shared core
and pixel family patches are review pins, and release still depends on the
unpublished family crates. The standalone `fuzz/Cargo.toml` still patches
older core and pixel revisions; `cargo metadata --locked` fails its zenpixels
version selection against this source. That separate fuzz graph must be aligned
and qualified before landing.
