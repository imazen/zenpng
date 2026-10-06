#!/usr/bin/env bash
# Build the inputs for benches/vs_png.rs from 20 imazen-26 renders (one per
# category, png-v3 variant), downloaded over HTTPS (no git-lfs needed).
#
#   scripts/vs_png_inputs.sh OUT_DIR
#   ZENPNG_BENCH_DIR=OUT_DIR cargo bench --bench vs_png --features _dev
#
# Every source is resized (Mitchell) to each long edge in SIZES (default
# "64 256 1024 2560") as RGB8 (PNG24, so ImageMagick can't pick gray or
# palette); a size larger than the source is skipped, never upscaled. Five
# sources also get RGBA8 at each size in RGBA_SIZES (default 1024) and gray8,
# palette, RGB16 and Adam7 variants at 1024; the 1-bit patent scan gets a
# gray1 variant. Requires ImageMagick 7 (`magick`) and curl.
#
# benches/pareto.rs inputs: SIZES="64 256 1024 4096" RGBA_SIZES="256 1024"
set -euo pipefail
out=${1:?usage: $0 OUT_DIR}
src=$out/src
mkdir -p "$src"
base=https://codec-corpus.r2.imazen.org/imazen-26-png-v3
while read -r rel; do
  f=$src/$(basename "$rel")
  [ -s "$f" ] || curl -fsSL "$base/${rel#png-v3/}" -o "$f"
done <<'LIST'
png-v3/1200-lilith-interiors/1207_interiors_bedroom-with-bed_hotel-zentik-project-valladolid_s23u_iso1600-f1p7_20230917-183439_4000x3000.sdr.png
png-v3/1400-lilith-nature/1407_nature_rocky-coastline-ocean_20210608-132404-2_8160x6120.sdr.png
png-v3/1600-lilith-food/1609_food_tray-of-food_kurama-kibune-cho-kyoto-japan_s23u_iso100-f2p2_20230711-123756_4000x3000.sdr.png
png-v3/2000-unsplash-people/2007_people_by-dwayne-joe-6-rwiq8vbks-unsplash_2624x3936.sdr.png
png-v3/2200-unsplash-renders/2207_renders_abstract-contour-pattern_by-matt-str-l5btvo8eeyq-unsplash_11514x8635.sdr.png
png-v3/2400-unsplash-textures/2407_textures_blue-wabring-threads_by-pawel-czerwinski-kra9zndplra-unsplash_4000x6000.sdr.png
png-v3/3000-art-institute-of-chicago-photos/3007_aic_rounded-jar-depicting-abstract-fish-or-sharks_4715_2848x2250.sdr.png
png-v3/3300-met-museum-photos/3307_met_marine_337532_3795x2667.sdr.png
png-v3/5000-national-park-service-brochures/color/5007_nps_choh-great-falls-hiking-map_color_p01_3300x2550.sdr.png
png-v3/5200-epa-climate-impact-2021-report/5207_epa_climate-impact-2021_approach-4-step-methodology_p011_2968x3841.sdr.png
png-v3/5300-noaa-hurricane-documents/5307_noaa_nhc-milton-al142024_text-body_p19_2550x3300.sdr.png
png-v3/6000-lilith-scans-public-patents/lynn_conway_us5046022_1bitoriginal/6007_scans-patents_lynn-conway-us5046022-1bit_p008_2320x3408.sdr.png
png-v3/6600-ia-scans-manuscript-illustrations/6607_scans-illustrations_haeckel-radiolaria-spheres_plate0090_4918x7246.sdr.png
png-v3/6800-ia-scans-manuscript-text/6807_scans-text_hokusai-ja-woodblock_p0029_6166x4857.sdr.png
png-v3/7000-lilith-plots/aliased-lines/7007_plots_line-00020-s1aac7045_1024x1024.sdr.png
png-v3/8000-lilith-mobile-screenshots/8007_mobile-screenshots_imageflow-site-layout_screenshot-20260526-070339-brave_1080x2520.sdr.png
png-v3/8100-lilith-web-screenshots/1440x900/8107_web-screenshots_climate-news_dpr1_page2_1440x900.sdr.png
png-v3/9000-lilith-ai-clipart/9007_gen_clipart_boba-tea-cup_1024x1024.sdr.png
png-v3/9094-lilith-ai-illustrations/9097_gen_illustrations_autumn-deciduous-path_1024x1536.sdr.png
png-v3/9226-lilith-ai-products/accessories/9227_gen_products-accessories_bucket-hat-checkerboard_p0002_1024x1536.sdr.png
LIST
cd "$src"
SIZES=${SIZES:-64 256 1024 2560}
RGBA_SIZES=${RGBA_SIZES:-1024}
for f in *.png; do
  s=${f:0:4}
  edge=$(magick identify -format '%[fx:max(w,h)]' "$f")
  for L in $SIZES; do
    [ "$L" -le "$edge" ] || continue
    nice -n 19 magick "$f" -filter Mitchell -resize "${L}x${L}>" -strip "PNG24:$out/${s}_rgb8_${L}.png"
  done
done
cd "$out"
for s in 1207 8107 9007 5207 6807; do
  for L in $RGBA_SIZES; do
    [ -f "${s}_rgb8_${L}.png" ] || continue
    nice -n 19 magick "${s}_rgb8_${L}.png" -alpha set -channel A -fx '0.5+0.5*sin(i/37)*cos(j/53)' +channel -strip "PNG32:${s}_rgba8_${L}.png"
  done
  in=${s}_rgb8_1024.png
  nice -n 19 magick "$in" -colorspace Gray -depth 8 -define png:color-type=0 -strip "${s}_gray8_1024.png"
  nice -n 19 magick "$in" -colors 256 -strip "PNG8:${s}_pal8_1024.png"
  nice -n 19 magick "$in" -depth 16 -strip "PNG48:${s}_rgb16_1024.png"
  nice -n 19 magick "$in" -interlace PNG -strip "${s}_interlaced_1024.png"
done
nice -n 19 magick "$src"/6007*.png -resize "1024x1024>" -threshold 50% -type bilevel -strip 6007_gray1_1024.png
ls "$out"/*.png | wc -l
