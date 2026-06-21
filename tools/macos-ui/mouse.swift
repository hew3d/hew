// CGEvent-based mouse driver for UI testing on macOS.
//
// System Events' `click at {x, y}` posts through the Accessibility API and is
// rejected by many apps (notably WKWebView content — Tauri's webview), failing
// with error -25208. Synthesizing low-level HID events with CGEvent instead
// lands clicks anywhere on screen, including inside the webview, as long as the
// driving process (the terminal running this) holds Accessibility permission.
//
// All coordinates are ABSOLUTE screen points (not pixels — points already
// account for Retina scale). The hew-ui.sh wrapper converts window-relative
// coordinates to absolute before calling this.
//
// Subcommands:
//   move  X Y               move the cursor
//   click X Y               single left click
//   dblclick X Y            double left click
//   rclick X Y              single right click
//   drag  X1 Y1 X2 Y2       press at 1, move to 2, release (viewport draws/drags)

import CoreGraphics
import Foundation

let src = CGEventSource(stateID: .hidSystemState)

func post(_ type: CGEventType, _ p: CGPoint, _ button: CGMouseButton = .left) {
    CGEvent(
        mouseEventSource: src, mouseType: type, mouseCursorPosition: p, mouseButton: button
    )?.post(tap: .cghidEventTap)
}

func sleepMs(_ ms: UInt32) { usleep(ms * 1000) }

let a = CommandLine.arguments
guard a.count >= 2 else {
    FileHandle.standardError.write("usage: mouse <move|click|dblclick|rclick|drag> coords...\n".data(using: .utf8)!)
    exit(2)
}
let cmd = a[1]

func pt(_ i: Int) -> CGPoint { CGPoint(x: Double(a[i])!, y: Double(a[i + 1])!) }

switch cmd {
case "move":
    post(.mouseMoved, pt(2))
case "click":
    let p = pt(2)
    post(.mouseMoved, p); sleepMs(40)
    post(.leftMouseDown, p); sleepMs(40)
    post(.leftMouseUp, p)
case "rclick":
    let p = pt(2)
    post(.mouseMoved, p); sleepMs(40)
    post(.rightMouseDown, p, .right); sleepMs(40)
    post(.rightMouseUp, p, .right)
case "dblclick":
    let p = pt(2)
    post(.mouseMoved, p); sleepMs(40)
    // Two down/up pairs; the clickState field tells the OS it's a double click.
    for clickState in [1, 2] {
        let down = CGEvent(mouseEventSource: src, mouseType: .leftMouseDown, mouseCursorPosition: p, mouseButton: .left)!
        down.setIntegerValueField(.mouseEventClickState, value: Int64(clickState))
        down.post(tap: .cghidEventTap)
        sleepMs(20)
        let up = CGEvent(mouseEventSource: src, mouseType: .leftMouseUp, mouseCursorPosition: p, mouseButton: .left)!
        up.setIntegerValueField(.mouseEventClickState, value: Int64(clickState))
        up.post(tap: .cghidEventTap)
        sleepMs(20)
    }
case "drag":
    let p1 = pt(2), p2 = pt(4)
    post(.mouseMoved, p1); sleepMs(40)
    post(.leftMouseDown, p1); sleepMs(60)
    // Interpolate so the webview sees a real drag, not a teleport.
    let steps = 24
    for i in 1...steps {
        let t = Double(i) / Double(steps)
        let p = CGPoint(x: p1.x + (p2.x - p1.x) * t, y: p1.y + (p2.y - p1.y) * t)
        post(.leftMouseDragged, p); sleepMs(12)
    }
    post(.leftMouseUp, p2)
default:
    FileHandle.standardError.write("unknown subcommand: \(cmd)\n".data(using: .utf8)!)
    exit(2)
}
