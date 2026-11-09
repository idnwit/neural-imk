// Renders the companion app icon into an .iconset folder (convert with
// `iconutil -c icns`): the menu icon's bold "Vi" in white on a rounded
// blue square, at every size iconutil expects.
// Usage: swift scripts/make-app-icon.swift <out.iconset>
import AppKit

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: make-app-icon.swift <out.iconset>\n".utf8))
    exit(2)
}

func render(pixels: Int) -> Data? {
    guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
                                     bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                     colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
    else { return nil }
    let side = CGFloat(pixels)
    rep.size = NSSize(width: side, height: side)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    // macOS icon grid: the shape fills ~80% of the canvas.
    let inset = side * 0.1
    let rect = NSRect(x: inset, y: inset, width: side - 2 * inset, height: side - 2 * inset)
    let radius = rect.width * 0.225
    NSColor(calibratedRed: 0.13, green: 0.42, blue: 0.86, alpha: 1).setFill()
    NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).fill()
    let attrs: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: rect.height * 0.5, weight: .bold),
        .foregroundColor: NSColor.white,
    ]
    let text = NSAttributedString(string: "Vi", attributes: attrs)
    let s = text.size()
    text.draw(at: NSPoint(x: rect.midX - s.width / 2, y: rect.midY - s.height / 2))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])
}

let dir = URL(fileURLWithPath: CommandLine.arguments[1])
do {
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    for base in [16, 32, 128, 256, 512] {
        for scale in [1, 2] {
            guard let png = render(pixels: base * scale) else { exit(1) }
            let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
            try png.write(to: dir.appendingPathComponent(name))
        }
    }
} catch {
    exit(1)
}
