// Renders the input-menu icon: black "Vi" on transparent (template image),
// 16x16 and 32x32 (@2x) representations in one TIFF.
// Usage: swift scripts/make-icon.swift <out.tiff>
import AppKit

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: make-icon.swift <out.tiff>\n".utf8))
    exit(2)
}

func render(pixels: Int) -> NSBitmapImageRep? {
    guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
                                     bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                     colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
    else { return nil }
    rep.size = NSSize(width: 16, height: 16)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let attrs: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: 11, weight: .bold),
        .foregroundColor: NSColor.black,
    ]
    let text = NSAttributedString(string: "Vi", attributes: attrs)
    let s = text.size()
    text.draw(at: NSPoint(x: (16 - s.width) / 2, y: (16 - s.height) / 2))
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let reps = [16, 32].compactMap(render(pixels:))
guard reps.count == 2,
      let data = NSBitmapImageRep.tiffRepresentationOfImageReps(in: reps, using: .lzw, factor: 0)
else { exit(1) }
do {
    try data.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
} catch {
    exit(1)
}
