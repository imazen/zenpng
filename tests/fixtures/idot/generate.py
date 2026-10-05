#!/usr/bin/env python3
"""Tiny PNGs carrying Apple's `iDOT` parallel-decode chunk (stdlib only).

Writes `*.png`, `manifest.json` and `manifest.tsv` (same data, for the
Rust test) next to this script. Every image is a few
hundred bytes. The parallel path only engages on images this small when a
test lowers the size threshold (`zenpng::__set_idot_min_bytes`, `_dev`).

`iDOT` layout (big-endian u32s): N, then N x {first_row, row_count, offset},
offset = byte offset of the segment's first IDAT chunk header, measured from
the first byte of the iDOT chunk's length field.

manifest.json, per file:
  color_type, bit_depth, width, height
  raw_sha256       sha256 of the unfiltered scanlines (packed, no filter
                   bytes) - ground truth computed here, independent of any
                   decoder; null when the image data is not a complete
                   serial stream
  serial_ok        a decoder ignoring iDOT decodes the whole image
  parallel_usable  a correct decoder can decode the segments independently
  note
"""

import hashlib
import json
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))


def chunk(kind, data):
    return (struct.pack(">I", len(data)) + kind + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))


CHANNELS = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}


def row_bytes(w, ct, bd):
    return (w * CHANNELS[ct] * bd + 7) // 8


def filter_bpp(ct, bd):
    return max(1, CHANNELS[ct] * bd // 8)


def make_rows(w, h, ct, bd, seed):
    """Deterministic packed scanlines with structure (every filter has work)."""
    rb = row_bytes(w, ct, bd)
    rows = []
    x = seed * 2654435761 & 0xFFFFFFFF
    for y in range(h):
        r = bytearray(rb)
        for i in range(rb):
            x ^= (x << 13) & 0xFFFFFFFF
            x ^= x >> 17
            x ^= (x << 5) & 0xFFFFFFFF
            smooth = (i * 3 + y * 5) & 0xFF
            r[i] = smooth if (i // 7 + y) % 3 else (x & 0xFF)
        if ct == 3:  # palette indices must stay below the palette size (16)
            r = bytearray(b & 0xFF if bd < 8 else b & 0x0F for b in r)
        rows.append(bytes(r))
    return rows


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def filter_row(ft, row, prev, bpp):
    out = bytearray([ft])
    for i, v in enumerate(row):
        a = row[i - bpp] if i >= bpp else 0
        b = prev[i]
        c = prev[i - bpp] if i >= bpp else 0
        p = [0, a, b, (a + b) >> 1, paeth(a, b, c)][ft]
        out.append((v - p) & 0xFF)
    return bytes(out)


def filtered(rows, filters, bpp):
    prev = bytes(len(rows[0]))
    out = []
    for row, ft in zip(rows, filters):
        out.append(filter_row(ft, row, prev, bpp))
        prev = row
    return out


def default_filters(h, boundaries):
    return [1 if (y == 0 or y in boundaries) else [1, 2, 3, 4, 2][y % 5] for y in range(h)]


def bounds_of(rows_per_seg):
    acc, b = 0, set()
    for n in rows_per_seg[:-1]:
        acc += n
        b.add(acc)
    return b


def segments(frows, rows_per_seg, mode="full"):
    parts, start = [], 0
    co = zlib.compressobj(9, zlib.DEFLATED, -15)
    for k, n in enumerate(rows_per_seg):
        data = b"".join(frows[start:start + n])
        start += n
        last = k == len(rows_per_seg) - 1
        if mode == "full":
            co = zlib.compressobj(9, zlib.DEFLATED, -15)
            parts.append(co.compress(data) + co.flush(zlib.Z_FINISH if last else zlib.Z_FULL_FLUSH))
        else:
            parts.append(co.compress(data) + co.flush(zlib.Z_FINISH if last else zlib.Z_SYNC_FLUSH))
    return parts


def table(entries):
    out = struct.pack(">I", len(entries))
    for f, n, o in entries:
        out += struct.pack(">III", f, n, o)
    return out


def png(ihdr_fields, pre_idat, parts, rows_per_seg, adler, header=b"\x78\xda",
        idat_chunk=None, override=None, idot_after=False, extra_pre=b""):
    parts = list(parts)
    parts[0] = header + parts[0]
    parts[-1] = parts[-1] + struct.pack(">I", adler)
    seg_chunks = []
    for p in parts:
        if idat_chunk:
            seg_chunks.append([p[i:i + idat_chunk] for i in range(0, len(p), idat_chunk)] or [b""])
        else:
            seg_chunks.append([p])
    n = len(parts)
    pos = 12 + 4 + 12 * n
    entries, first = [], 0
    for k, chunks in enumerate(seg_chunks):
        entries.append((first, rows_per_seg[k], pos))
        first += rows_per_seg[k]
        pos += sum(12 + len(c) for c in chunks)
    tbl = override(entries) if override else table(entries)
    idat = b"".join(chunk(b"IDAT", c) for chunks in seg_chunks for c in chunks)
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", *ihdr_fields)) + pre_idat + extra_pre
    out += (idat + chunk(b"iDOT", tbl)) if idot_after else (chunk(b"iDOT", tbl) + idat)
    return out + chunk(b"IEND", b"")


def main():
    manifest = {}

    def emit(name, data, meta, raw_sha, serial_ok, usable, note):
        with open(os.path.join(HERE, name), "wb") as f:
            f.write(data)
        manifest[name] = dict(meta, raw_sha256=raw_sha, serial_ok=serial_ok,
                              parallel_usable=usable, note=note)

    def image(ct, bd, w, h, seed):
        rows = make_rows(w, h, ct, bd, seed)
        pre = b""
        if ct == 3:
            pal = bytes(v for i in range(16) for v in (i * 16, 255 - i * 16, (i * 37) & 0xFF))
            pre = chunk(b"PLTE", pal) + chunk(b"tRNS", bytes(range(0, 256, 32)))
        meta = {"color_type": ct, "bit_depth": bd, "width": w, "height": h}
        return rows, pre, meta, (w, h, bd, ct, 0, 0, 0), hashlib.sha256(b"".join(rows)).hexdigest()

    def build(ct, bd, w, h, rows_per_seg, seed=1, filters=None, mode="full"):
        rows, pre, meta, ih, sha = image(ct, bd, w, h, seed)
        bpp = filter_bpp(ct, bd)
        fs = filters or default_filters(h, bounds_of(rows_per_seg))
        frows = filtered(rows, fs, bpp)
        adler = zlib.adler32(b"".join(frows)) & 0xFFFFFFFF
        return rows, pre, meta, ih, sha, frows, segments(frows, rows_per_seg, mode), adler

    # --- Valid layouts across color types (both decoder output paths) -------
    formats = [
        (6, 8, "rgba8"), (2, 8, "rgb8"), (0, 8, "gray8"), (4, 8, "ga8"),
        (0, 16, "gray16"), (2, 16, "rgb16"), (6, 16, "rgba16"),
        (3, 8, "pal8_trns"), (3, 4, "pal4_trns"), (0, 1, "gray1"), (0, 2, "gray2"),
    ]
    for k, (ct, bd, tag) in enumerate(formats):
        segs = [8, 8]
        _, pre, meta, ih, sha, _, parts, adler = build(ct, bd, 24, 16, segs, seed=k + 1)
        emit(f"{tag}_n2.png", png(ih, pre, parts, segs, adler), meta, sha, True, True,
             f"{tag}, two 8-row segments")

    for segs, nm, chunk_size in [([3, 9, 4], "n3_uneven", None), ([2] * 8, "n8", 7),
                                 ([1, 15], "n2_one_row_first", None)]:
        _, pre, meta, ih, sha, _, parts, adler = build(6, 8, 24, 16, segs, seed=50)
        emit(f"rgba8_{nm}.png", png(ih, pre, parts, segs, adler, idat_chunk=chunk_size),
             meta, sha, True, True, f"segments {segs}" + (f", {chunk_size}-byte IDAT chunks" if chunk_size else ""))

    # --- Boundary rows that read the previous segment ------------------------
    for ft, nm in [(2, "up"), (3, "average"), (4, "paeth")]:
        fs = default_filters(16, {8})
        fs[8] = ft
        _, pre, meta, ih, sha, _, parts, adler = build(6, 8, 24, 16, [8, 8], seed=60, filters=fs)
        emit(f"boundary_row_{nm}.png", png(ih, pre, parts, [8, 8], adler), meta, sha, True, False,
             f"segment 1 starts with filter {nm}")

    # --- Streams that are not segmentable ------------------------------------
    _, pre, meta, ih, sha, _, parts, adler = build(6, 8, 24, 16, [8, 8], seed=70, mode="sync")
    emit("sync_flush_backrefs.png", png(ih, pre, parts, [8, 8], adler), meta, sha, True, False,
         "sync flush keeps the window; segment 1 may back-reference segment 0")

    rows, pre, meta, ih, sha, frows, _, adler = build(6, 8, 24, 16, [8, 8], seed=71)
    body = zlib.compress(b"".join(frows), 9)[2:-4]
    cut = len(body) // 2
    emit("no_flush_at_offset.png", png(ih, pre, [body[:cut], body[cut:]], [8, 8], adler),
         meta, sha, True, False, "one deflate stream, iDOT offset at an arbitrary chunk boundary")

    # Buchanan-style: the split falls inside a stored block, after its header.
    stored = b""
    data = b"".join(frows)
    half = len(data) // 2
    stored0 = b"\x00" + struct.pack("<HH", half, half ^ 0xFFFF)
    rest = data[half:]
    seg0 = stored0  # header only; its payload lives in segment 1
    seg1 = data[:half] + b"\x01" + struct.pack("<HH", len(rest), len(rest) ^ 0xFFFF) + rest
    emit("stored_block_split.png", png(ih, pre, [seg0, seg1], [8, 8], adler), meta, sha, True, False,
         "segment 0 ends inside a stored block header (the ambiguous-PNG shape)")

    full0 = zlib.compress(b"".join(frows[:8]), 9)
    c1 = zlib.compressobj(9, zlib.DEFLATED, -15)
    s1 = c1.compress(b"".join(frows[8:])) + c1.flush(zlib.Z_FINISH)
    emit("complete_stream_in_segment0.png", png(ih, pre, [full0, s1], [8, 8], adler, header=b""),
         meta, None, False, False, "segment 0 is a whole zlib stream; serial decode stops at row 8")

    c0 = zlib.compressobj(9, zlib.DEFLATED, -15)
    s0 = c0.compress(b"".join(frows[:8])) + c0.flush(zlib.Z_FINISH)
    emit("bfinal_in_segment0.png", png(ih, pre, [s0, s1], [8, 8], adler), meta, None, False, False,
         "segment 0 ends with BFINAL")

    _, pre, meta, ih, sha, frows, parts, adler = build(6, 8, 24, 16, [8, 8], seed=72)
    emit("bad_adler.png", png(ih, pre, parts, [8, 8], adler ^ 1), meta, sha, True, False,
         "correct segments, wrong Adler-32 trailer")

    def fake(entries):
        return table([(0, 7, entries[0][2]), (7, 9, entries[1][2])])
    emit("row_counts_disagree_with_stream.png", png(ih, pre, parts, [8, 8], adler, override=fake),
         meta, sha, True, False, "table says 7/9 rows, flush is after row 8")

    # --- Malformed tables over a valid stream --------------------------------
    variants = [
        ("table_offset_mid_chunk.png", lambda e: table([e[0], (e[1][0], e[1][1], e[1][2] + 3)]),
         "segment 1 offset inside an IDAT chunk", {}),
        ("table_offset_past_eof.png", lambda e: table([e[0], (e[1][0], e[1][1], 0x7FFFFFFF)]),
         "segment 1 offset past end of file", {}),
        ("table_offset_wraps.png", lambda e: table([e[0], (e[1][0], e[1][1], 0xFFFFFFFF)]),
         "offset 2^32-1 (pointer arithmetic overflow on 32-bit)", {}),
        ("table_first_not_first_idat.png", lambda e: table([(0, 8, e[1][2]), (8, 8, e[1][2])]),
         "segment 0 not at the first IDAT; offsets not increasing", {}),
        ("table_overlapping_rows.png", lambda e: table([(0, 16, e[0][2]), (0, 16, e[1][2])]),
         "both segments claim every row", {}),
        ("table_gap_rows.png", lambda e: table([(0, 7, e[0][2]), (8, 8, e[1][2])]),
         "row 7 belongs to no segment", {}),
        ("table_rows_exceed_height.png", lambda e: table([(0, 8, e[0][2]), (8, 9, e[1][2])]),
         "rows sum past the height", {}),
        ("table_zero_rows.png", lambda e: table([(0, 0, e[0][2]), (0, 16, e[1][2])]),
         "a segment with zero rows", {}),
        ("table_n1.png", lambda e: table([(0, 16, e[0][2])]), "N = 1", {}),
        ("table_n0.png", lambda e: struct.pack(">I", 0), "N = 0", {}),
        ("table_truncated.png", lambda e: table(e)[:-4], "length not 4 + 12N", {}),
        ("table_huge_n.png", lambda e: struct.pack(">I", 0xFFFFFFFF) + table(e)[4:],
         "N = 2^32-1 with a 28-byte body", {}),
        ("table_empty.png", lambda e: b"", "zero-length iDOT", {}),
        ("idot_after_idat.png", None, "iDOT after the image data", {"idot_after": True}),
        ("two_idot_chunks.png", None, "a second iDOT before the real one",
         {"extra_pre": chunk(b"iDOT", table([(0, 16, 0)]))}),
    ]
    for name, ov, note, kw in variants:
        emit(name, png(ih, pre, parts, [8, 8], adler, override=ov, **kw), meta, sha, True, False, note)

    # Interlaced with iDOT: the table must be ignored.
    rows, pre, meta, ih, sha = image(6, 8, 16, 16, 80)
    passes = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
    raw = b""
    for x0, y0, dx, dy in passes:
        for y in range(y0, 16, dy):
            px = b"".join(rows[y][4 * x:4 * x + 4] for x in range(x0, 16, dx))
            if px:
                raw += b"\x00" + px
    c = zlib.compressobj(9, zlib.DEFLATED, -15)
    h2 = len(raw) // 2
    p0 = c.compress(raw[:h2]) + c.flush(zlib.Z_FULL_FLUSH)
    p1 = c.compress(raw[h2:]) + c.flush(zlib.Z_FINISH)
    emit("interlaced_with_idot.png",
         png((16, 16, 8, 6, 0, 0, 1), pre, [p0, p1], [8, 8], zlib.adler32(raw) & 0xFFFFFFFF),
         meta, sha, True, False, "Adam7 image with an iDOT chunk")

    with open(os.path.join(HERE, "manifest.json"), "w") as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
        f.write("\n")
    with open(os.path.join(HERE, "manifest.tsv"), "w") as f:
        f.write("file\tcolor_type\tbit_depth\twidth\theight\traw_sha256\tserial_ok\tparallel_usable\n")
        for n in sorted(manifest):
            m = manifest[n]
            f.write(f"{n}\t{m['color_type']}\t{m['bit_depth']}\t{m['width']}\t{m['height']}\t"
                    f"{m['raw_sha256'] or '-'}\t{int(m['serial_ok'])}\t{int(m['parallel_usable'])}\n")
    total = sum(os.path.getsize(os.path.join(HERE, n)) for n in manifest)
    print(f"wrote {len(manifest)} files, {total} bytes")


if __name__ == "__main__":
    main()
