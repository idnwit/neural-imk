// Renders the DMG window background: a light 640x400 canvas with a large
// arrow from the app icon slot (left) to the Applications slot (right) and a
// Vietnamese caption at the bottom.
// Usage: swift scripts/make-dmg-background.swift <out.png>
import AppKit

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: make-dmg-background.swift <out.png>\n".utf8))
    exit(2)
}

let width = 640, height = 400
guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
                                 bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                 colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
else {
    FileHandle.standardError.write(Data("cannot allocate bitmap\n".utf8))
    exit(1)
}
rep.size = NSSize(width: width, height: height)
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

NSColor(calibratedWhite: 0.96, alpha: 1).setFill()
NSRect(x: 0, y: 0, width: width, height: height).fill()

// Arrow, y = 200 (AppKit origin is bottom-left; the canvas is symmetric here).
let blue = NSColor(calibratedRed: 0.13, green: 0.42, blue: 0.86, alpha: 1)
blue.setFill()
let shaft = NSBezierPath(rect: NSRect(x: 160, y: 188, width: 250, height: 24))
shaft.fill()
let head = NSBezierPath()
head.move(to: NSPoint(x: 400, y: 245))
head.line(to: NSPoint(x: 480, y: 200))
head.line(to: NSPoint(x: 400, y: 155))
head.close()
head.fill()

let attrs: [NSAttributedString.Key: Any] = [
    .font: NSFont.systemFont(ofSize: 18, weight: .medium),
    .foregroundColor: NSColor(calibratedWhite: 0.3, alpha: 1),
]
let caption = NSAttributedString(string: "Kéo NeuralIMK vào Applications", attributes: attrs)
let size = caption.size()
caption.draw(at: NSPoint(x: (CGFloat(width) - size.width) / 2, y: 40))

NSGraphicsContext.restoreGraphicsState()
guard let png = rep.representation(using: .png, properties: [:]) else {
    FileHandle.standardError.write(Data("png encode failed\n".utf8))
    exit(1)
}
do {
    try png.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
} catch {
    FileHandle.standardError.write(Data("write failed: \(error)\n".utf8))
    exit(1)
}
