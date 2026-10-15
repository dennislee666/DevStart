// Reproducible native renderer for the original [play] application mark.
import AppKit
let root = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "."
for (size, name) in [(1024, "devstart-icon.png"), (256, "devstart-window-icon.png")] {
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    let scale = CGFloat(size) / 1024
    let transform = NSAffineTransform()
    transform.scale(by: scale)
    transform.concat()
    NSColor(srgbRed: 0.12, green: 0.14, blue: 0.17, alpha: 1).setFill()
    NSBezierPath(roundedRect: NSRect(x: 48, y: 48, width: 928, height: 928), xRadius: 208, yRadius: 208).fill()
    NSColor(srgbRed: 126/255, green: 214/255, blue: 176/255, alpha: 1).setStroke()
    let brackets = NSBezierPath()
    brackets.lineWidth = 54
    brackets.lineCapStyle = .round
    brackets.lineJoinStyle = .round
    brackets.move(to: NSPoint(x: 338, y: 744))
    brackets.line(to: NSPoint(x: 248, y: 744))
    brackets.line(to: NSPoint(x: 248, y: 280))
    brackets.line(to: NSPoint(x: 338, y: 280))
    brackets.move(to: NSPoint(x: 686, y: 744))
    brackets.line(to: NSPoint(x: 776, y: 744))
    brackets.line(to: NSPoint(x: 776, y: 280))
    brackets.line(to: NSPoint(x: 686, y: 280))
    brackets.stroke()
    NSColor(srgbRed: 126/255, green: 214/255, blue: 176/255, alpha: 1).setFill()
    let play = NSBezierPath()
    play.move(to: NSPoint(x: 430, y: 670))
    play.line(to: NSPoint(x: 658, y: 512))
    play.line(to: NSPoint(x: 430, y: 354))
    play.close()
    play.fill()
    NSGraphicsContext.restoreGraphicsState()
    try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(root)/assets/\(name)"))
}
