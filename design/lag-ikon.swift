// Tegner app-ikonet. Bruk: swift design/lag-ikon.swift <utfil.png>
// Følger macOS-rutenettet: 1024 px lerret med et 824 px hovedfelt.
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let canvas = 1024
let ctx = CGContext(
    data: nil, width: canvas, height: canvas, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.displayP3)!,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 255) / 255, green: CGFloat((hex >> 8) & 255) / 255,
        blue: CGFloat(hex & 255) / 255, alpha: alpha)
}

func gradient(_ top: UInt32, _ bottom: UInt32) -> CGGradient {
    CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!,
        colors: [color(top), color(bottom)] as CFArray, locations: [0, 1])!
}

// Superellipse gives the continuous corners of a Mac icon rather than plain circular arcs.
func squircle(_ r: CGRect, exponent n: CGFloat = 5) -> CGPath {
    let path = CGMutablePath()
    let a = r.width / 2, b = r.height / 2
    for i in 0..<720 {
        let t = CGFloat(i) / 720 * 2 * .pi
        let c = cos(t), s = sin(t)
        let x = pow(abs(c), 2 / n) * a * (c < 0 ? -1 : 1)
        let y = pow(abs(s), 2 / n) * b * (s < 0 ? -1 : 1)
        let p = CGPoint(x: r.midX + x, y: r.midY + y)
        if i == 0 { path.move(to: p) } else { path.addLine(to: p) }
    }
    path.closeSubpath()
    return path
}

func rounded(_ r: CGRect, _ radius: CGFloat) -> CGPath {
    CGPath(roundedRect: r, cornerWidth: radius, cornerHeight: radius, transform: nil)
}

func fillVertical(_ g: CGGradient, in r: CGRect) {
    ctx.drawLinearGradient(
        g, start: CGPoint(x: r.midX, y: r.maxY), end: CGPoint(x: r.midX, y: r.minY), options: [])
}

let body = CGRect(x: 100, y: 100, width: 824, height: 824)
let tile = squircle(body)

ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: color(0x000000, 0.28))
ctx.addPath(tile)
ctx.setFillColor(color(0x1F6FE5))
ctx.fillPath()
ctx.restoreGState()

ctx.saveGState()
ctx.addPath(tile)
ctx.clip()
fillVertical(gradient(0x4BA3FF, 0x1558D6), in: body)

let cardSize = CGSize(width: 452, height: 548)
let center = CGPoint(x: 512, y: 506)

func placed(dx: CGFloat, dy: CGFloat, degrees: CGFloat, _ draw: (CGRect) -> Void) {
    ctx.saveGState()
    ctx.translateBy(x: center.x + dx, y: center.y + dy)
    ctx.rotate(by: -degrees * .pi / 180)
    draw(CGRect(
        x: -cardSize.width / 2, y: -cardSize.height / 2,
        width: cardSize.width, height: cardSize.height))
    ctx.restoreGState()
}

// Motion trail: fainter copies of the card behind it.
for (dx, degrees, alpha) in [(CGFloat(-132), CGFloat(-5), CGFloat(0.20)), (-52, 3, 0.36)] {
    placed(dx: dx, dy: 0, degrees: degrees) { r in
        ctx.addPath(rounded(r, 44))
        ctx.setFillColor(color(0xFFFFFF, alpha))
        ctx.fillPath()
    }
}

placed(dx: 62, dy: 0, degrees: 11) { r in
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: -6, height: -18), blur: 40, color: color(0x062A6B, 0.42))
    ctx.addPath(rounded(r, 44))
    ctx.setFillColor(color(0xFFFFFF))
    ctx.fillPath()
    ctx.restoreGState()

    let photo = r.insetBy(dx: 30, dy: 30)
    ctx.saveGState()
    ctx.addPath(rounded(photo, 20))
    ctx.clip()
    fillVertical(gradient(0xFFC56B, 0xFF6A5A), in: photo)

    ctx.setFillColor(color(0xFFF1C9))
    ctx.fillEllipse(in: CGRect(x: photo.midX + 12, y: photo.midY + 62, width: 128, height: 128))

    func ridge(_ points: [(CGFloat, CGFloat)], _ fill: UInt32) {
        let p = CGMutablePath()
        p.move(to: CGPoint(x: photo.minX, y: photo.minY))
        for (x, y) in points {
            p.addLine(to: CGPoint(x: photo.minX + photo.width * x, y: photo.minY + photo.height * y))
        }
        p.addLine(to: CGPoint(x: photo.maxX, y: photo.minY))
        p.closeSubpath()
        ctx.addPath(p)
        ctx.setFillColor(color(fill))
        ctx.fillPath()
    }
    ridge([(0, 0.42), (0.30, 0.66), (0.52, 0.44), (0.76, 0.60), (1, 0.38)], 0x2C7C8C)
    ridge([(0, 0.20), (0.26, 0.38), (0.58, 0.16), (0.82, 0.32), (1, 0.18)], 0x124E60)
    ctx.restoreGState()
}
ctx.restoreGState()

let url = URL(fileURLWithPath: CommandLine.arguments[1])
let out = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(out, ctx.makeImage()!, nil)
guard CGImageDestinationFinalize(out) else { fatalError("Kunne ikke skrive ikonet") }
