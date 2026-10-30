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
value bit-for-bit. PNG has no diffuse-white carrier. Empty or invalid carriers, ICC profiles over the native decoder's 1 MiB limit,
APP1-prefixed EXIF, nonzero cICP matrix, and conflicting native cICP or HDR
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

The `checked_metadata` integration controls exercise completed-output readback
using the shared testkit, independent chunk/sample observers, one-shot, rows,
pull, animation and erased jobs, replacement/clear and refusals. These sources
are not evidence of a passing test run until a serialized qualification is
recorded. The current lock is a qualification artifact, not evidence that an
unlocked downstream graph selects identical package versions.
