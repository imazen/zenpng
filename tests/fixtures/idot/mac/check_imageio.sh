#!/usr/bin/env bash
# ImageIO compatibility gate for zenpng's iDOT output (run on macOS).
#
# 1. Encodes images with zenpng at 1/2/4/8/16 segments (thresholds lowered so
#    small images qualify) from the RGBA8 fixtures.
# 2. Decodes every output with Apple ImageIO, as written and with iDOT
#    removed (ImageIO's serial path), via imageio_tool.swift.
# 3. Fails unless, for every zenpng-written file, ImageIO decodes it, its iDOT
#    path matches its serial path, and zenpng's pixels match ImageIO's.
#
# Usage (from the repo root): tests/fixtures/idot/mac/check_imageio.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/../../../.." && pwd)
work=${IDOT_WORK:-$root/target/idot-imageio}
rm -rf "$work"
mkdir -p "$work/in/zenpng" "$work/raw"

swiftc -O "$root/tests/fixtures/idot/mac/imageio_tool.swift" -o "$work/imageio_tool"
cargo build --release --features _dev --example idot_encode --example idot_imageio_compare

inputs=("$root"/tests/fixtures/idot/rgba8_n8.png "$root"/tests/fixtures/idot/rgba8_n3_uneven.png)
for crop in "" 24x9; do
  IDOT_CROP=$crop ZENPNG_IDOT_MIN_BYTES=1 "$root/target/release/examples/idot_encode" \
    "$work/in/zenpng" 7 "${inputs[@]}" > /dev/null
done
ls "$work/in/zenpng" | wc -l | xargs echo "zenpng files:"
with_idot=$(grep -l iDOT "$work"/in/zenpng/*.png | wc -l | tr -d ' ')
echo "with iDOT: $with_idot"
if [ "$with_idot" -lt 12 ]; then
  echo "expected at least 12 segmented files; the encoder did not write iDOT"
  exit 1
fi
sw_vers

DUMP_DIR="$work/raw" "$work/imageio_tool" decode "$work"/in/zenpng/*.png > "$work/decode.tsv"
: > "$work/errors.tsv"
"$root/target/release/examples/idot_imageio_compare" "$work/decode.tsv" "$work/raw" "$work/errors.tsv" \
  "zenpng=$work/in/zenpng" > "$work/compare.tsv"
cat "$work/compare.tsv"

# Columns: file, zenpng_serial, parallel==serial, zenpng_vs_imageio_serial,
# imageio_idot_path_vs_its_serial, log
bad=$(awk -F'\t' 'NR>1 && ($2 != "ok" || $3 != "true" || $4 != "IDENTICAL" || $5 != "same")' "$work/compare.tsv")
if [ -n "$bad" ]; then
  echo "ImageIO compatibility FAILED:"
  echo "$bad"
  exit 1
fi
echo "ImageIO compatibility OK"
