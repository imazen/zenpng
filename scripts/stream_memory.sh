#!/bin/bash
# Peak RSS, heap peak and wall time for one-shot vs streaming encode and
# decode (examples/heaptrack_streaming.rs), one process per measurement.
#
#   scripts/stream_memory.sh <heaptrack_streaming binary> <out.tsv> <png>...
#
# Each PNG must be RGB8 or RGBA8. Per input: `load` (the input decode every
# encode mode pays), encode modes oneshot/stream (default downcasts) and
# soneshot/sstream (DowncastFlags::none(), strip streaming at efforts 1-15),
# single-threaded and with PARALLEL=1, at EFFORTS (default 1,2,7,13,15,16,19),
# then the decode modes. CORES (default 16-19) pins every run. Columns:
# image mode effort parallel max_rss_kib wall_s heap_peak bytes.
set -u
bin=$1
out=$2
shift 2
cores=${CORES:-16-19}
efforts=${EFFORTS:-1,2,7,13,15,16,19}
tmp=$(mktemp -d "${TMPDIR:-$HOME/tmp}/stream_memory.XXXXXX")
printf 'image\tmode\teffort\tparallel\tmax_rss_kib\twall_s\theap_peak\tbytes\n' >"$out"

run() { # image mode effort parallel args...
  local img=$1 mode=$2 e=$3 par=$4
  shift 4
  local tv
  tv=$({ PARALLEL=$par /usr/bin/time -v taskset -c "$cores" "$bin" "$@"; } 2>&1)
  local rss wall bytes peak
  rss=$(grep 'Maximum resident' <<<"$tv" | grep -o '[0-9]*')
  wall=$(grep 'Elapsed (wall' <<<"$tv" | awk '{print $NF}' | awk -F: '{print ($1*60+$2)}')
  bytes=$(grep -o '[0-9]* bytes' <<<"$tv" | tail -1 | grep -o '[0-9]*')
  rm -f "$tmp"/ht.*
  PARALLEL=$par taskset -c "$cores" heaptrack -o "$tmp/ht" "$bin" "$@" >/dev/null 2>&1
  peak=$(heaptrack_print "$tmp"/ht.zst 2>/dev/null | grep -m1 'peak heap memory consumption' | grep -o '[0-9.]*[KMG]$')
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$img" "$mode" "$e" "$par" "$rss" "$wall" "$peak" "${bytes:-}" >>"$out"
}

for png in "$@"; do
  img=$(basename "$png" .png)
  run "$img" load - 0 load "$png"
  for e in ${efforts//,/ }; do
    for par in 0 1; do
      for m in oneshot stream soneshot sstream; do
        run "$img" "$m" "$e" "$par" "$m$e" "$png"
      done
    done
  done
  for m in dec_whole dec_whole_mt dec_push dec_stream; do
    run "$img" "$m" - 0 "$m" "$png"
  done
done
rm -rf "$tmp"
