// ImageIO probe for zenpng's iDOT fixtures. Run on macOS:
//
//   swiftc -O imageio_tool.swift -o imageio_tool
//   ./imageio_tool encode OUT_DIR W H [W H ...]   # write RGBA8 PNGs via ImageIO
//   ./imageio_tool decode FILE...                 # decode via ImageIO, print TSV
//
// encode: deterministic RGBA8 pixels (same formula as the zenpng generator's
// make_rows for RGBA8, seed 99) written with CGImageDestination; prints
// whether ImageIO emitted an iDOT chunk and the raw-pixel SHA-256.
//
// decode: CGImageSource -> CGImage -> the decoded buffer as ImageIO produced
// it (dataProvider, no redraw / color conversion), twice: as given, and with
// every iDOT chunk removed (ImageIO's own serial decode). Prints one TSV row:
//   path, ok, width, height, bitsPerComponent, bitsPerPixel, alphaInfo,
//   byteOrder, sha256 (as given), sha256 (iDOT removed), 1 if they match
// sha256 covers the packed rows (row padding stripped).
// With DUMP_DIR set, the packed rows are also written to
// DUMP_DIR/<parent dir>__<file>.raw and .noidot.raw for byte comparison.
// Anything ImageIO logs (e.g. "iDOT doesn't point to valid IDAT chunk") goes
// to stderr from the framework itself.

import CoreGraphics
import CryptoKit
import Foundation
import ImageIO
import UniformTypeIdentifiers
import os

func hex(_ d: some Sequence<UInt8>) -> String { d.map { String(format: "%02x", $0) }.joined() }

func makeRGBA(_ w: Int, _ h: Int, seed: UInt32) -> [UInt8] {
    var x: UInt32 = seed &* 2_654_435_761
    var out = [UInt8](repeating: 0, count: w * h * 4)
    let rb = w * 4
    for y in 0..<h {
        for i in 0..<rb {
            x ^= x << 13; x ^= x >> 17; x ^= x << 5
            let smooth = UInt8(truncatingIfNeeded: i * 3 + y * 5)
            out[y * rb + i] = ((i / 7 + y) % 3 != 0) ? smooth : UInt8(truncatingIfNeeded: x)
        }
    }
    // Keep alpha opaque-ish but varied, so ImageIO keeps an alpha channel.
    for p in stride(from: 3, to: out.count, by: 4) { out[p] = out[p] | 0x80 }
    return out
}

func encode(_ dir: String, _ w: Int, _ h: Int) {
    var px = makeRGBA(w, h, seed: 99)
    let raw = Data(px)
    let cs = CGColorSpace(name: CGColorSpace.sRGB)!
    let info = CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue)
    let provider = CGDataProvider(data: Data(bytes: &px, count: px.count) as CFData)!
    let img = CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32,
                      bytesPerRow: w * 4, space: cs, bitmapInfo: info, provider: provider,
                      decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    let path = "\(dir)/imageio_rgba8_\(w)x\(h).png"
    let url = URL(fileURLWithPath: path)
    let dst = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dst, img, nil)
    guard CGImageDestinationFinalize(dst) else { print("\(path)\tFAILED"); return }
    let bytes = try! Data(contentsOf: url)
    let hasIdot = bytes.range(of: Data("iDOT".utf8)) != nil
    print("\(path)\t\(w)x\(h)\tbytes=\(bytes.count)\tidot=\(hasIdot)\traw_sha256=\(hex(SHA256.hash(data: raw)))")
}

let marker = OSLog(subsystem: "com.imazen.zenpng.idot", category: "file")

/// The file with every `iDOT` chunk removed: what ImageIO decodes when it
/// cannot take its parallel path (its own serial interpretation).
func stripIdot(_ d: Data) -> Data {
    var out = d.prefix(8)
    var p = 8
    while p + 12 <= d.count {
        let n = Int(d[p]) << 24 | Int(d[p + 1]) << 16 | Int(d[p + 2]) << 8 | Int(d[p + 3])
        let end = min(d.count, p + 12 + n)
        if d[(p + 4)..<(p + 8)] != Data("iDOT".utf8) { out.append(d[p..<end]) }
        p = end
    }
    return out
}

struct Decoded {
    var width = 0, height = 0, bpc = 0, bpp = 0
    var alpha: UInt32 = 0, order: UInt32 = 0
    var packed: Data? = nil
}

func decodeData(_ fileBytes: Data) -> Decoded? {
    guard let src = CGImageSourceCreateWithData(fileBytes as CFData, nil),
          let decoded = CGImageSourceCreateImageAtIndex(src, 0, [kCGImageSourceShouldCache: false] as CFDictionary)
    else { return nil }
    guard let data = decoded.dataProvider?.data as Data? else { return nil }
    let img = decoded
    let rowLen = (img.width * img.bitsPerPixel + 7) / 8
    var packed = Data(capacity: rowLen * img.height)
    for y in 0..<img.height {
        let start = y * img.bytesPerRow
        packed.append(data[start..<(start + rowLen)])
    }
    let order = img.bitmapInfo.rawValue & CGBitmapInfo.byteOrderMask.rawValue
    return Decoded(width: img.width, height: img.height, bpc: img.bitsPerComponent,
                   bpp: img.bitsPerPixel, alpha: img.alphaInfo.rawValue, order: order, packed: packed)
}

func decode(_ path: String) {
    let url = URL(fileURLWithPath: path)
    let name = url.lastPathComponent
    let group = url.deletingLastPathComponent().lastPathComponent
    // Marks the start of each file in the unified log, so ImageIO's own
    // messages (e.g. "iDOT doesn't point to valid IDAT chunk") can be
    // attributed: log stream --level debug --predicate 'process == "imageio_tool"'
    os_log("ZENPNG-FILE %{public}@", log: marker, type: .error, path)
    let bytes = (try? Data(contentsOf: url)) ?? Data()
    let withIdot = decodeData(bytes)
    os_log("ZENPNG-STRIPPED %{public}@", log: marker, type: .error, path)
    let without = decodeData(stripIdot(bytes))
    let dump = ProcessInfo.processInfo.environment["DUMP_DIR"]
    func sha(_ d: Decoded?) -> String { d?.packed.map { hex(SHA256.hash(data: $0)) } ?? "-" }
    if let dump, let p = withIdot?.packed {
        try? p.write(to: URL(fileURLWithPath: "\(dump)/\(group)__\(name).raw"))
    }
    if let dump, let p = without?.packed {
        try? p.write(to: URL(fileURLWithPath: "\(dump)/\(group)__\(name).noidot.raw"))
    }
    let d = withIdot ?? without ?? Decoded()
    let same = (withIdot?.packed != nil && withIdot?.packed == without?.packed) ? 1 : 0
    print("\(path)\t\(withIdot == nil ? 0 : 1)\t\(d.width)\t\(d.height)\t\(d.bpc)\t\(d.bpp)\t\(d.alpha)\t\(d.order)\t\(sha(withIdot))\t\(sha(without))\t\(same)")
}

let args = CommandLine.arguments
switch args.count > 1 ? args[1] : "" {
case "encode":
    var i = 3
    while i + 1 < args.count {
        encode(args[2], Int(args[i])!, Int(args[i + 1])!)
        i += 2
    }
case "decode":
    print("file\tok\twidth\theight\tbpc\tbpp\talpha_info\tbyte_order\tsha256\tsha256_without_idot\tidot_path_matches_serial")
    for p in args.dropFirst(2) { decode(p) }
default:
    print("usage: imageio_tool encode DIR W H [W H...] | decode FILE...")
}
