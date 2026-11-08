---
title: "Keyboard shortcuts"
description: "Every default shortcut on one page, including the modifier keys that change what a tool does mid-gesture."
order: 23
---

Tool shortcuts are plain letters, the same on every platform, matching SketchUp's defaults wherever SketchUp has one. Command shortcuts use `⌘` on macOS and `Ctrl` on Windows, Linux, and the web.

## Tools

| Tool | Key |
|---|---|
| Select | `Space` |
| Line | `L` |
| Rectangle | `R` |
| Circle | `C` |
| Polygon | — |
| Arc | `A` |
| Push/Pull | `P` |
| Offset | `F` |
| Move | `M` |
| Rotate | `Q` |
| Scale | `S` |
| Tape Measure | `T` |
| Paint | `B` |
| Orbit | `O` |
| Pan | `H` |
| Zoom | `Z` |

Polygon, Protractor, Slice, Section Plane, and Edit Vertex have no default key, same as in SketchUp — reach them via the Draw/Tools menu or the command palette. On macOS the menus also display `⌘`-combination accelerators beside the tools; those work too, but the letters above are the ones worth learning.

## Commands

| Command | macOS | Windows / Linux / web |
|---|---|---|
| Command palette | `⌘/` | `Ctrl+/` |
| New | `⌘N` | `Ctrl+N` |
| Open | `⌘O` | `Ctrl+O` |
| Save | `⌘S` | `Ctrl+S` |
| Save As | `⇧⌘S` | `Ctrl+Shift+S` |
| Print | `⌘P` | `Ctrl+P` |
| Close window | `⌘W` (desktop) | — |
| Undo | `⌘Z` | `Ctrl+Z` |
| Redo | `⇧⌘Z` | `Ctrl+Shift+Z` |
| Select All | `⌘A` | `Ctrl+A` |
| Select None | `⇧⌘A` | `Ctrl+Shift+A` |
| Invert Selection | — | — |
| Cut | `⌘X` | `Ctrl+X` |
| Copy | `⌘C` | `Ctrl+C` |
| Paste | `⌘V` | `Ctrl+V` |
| Paste In Place | `⇧⌘V` | `Ctrl+Shift+V` |
| Delete selection | `Delete` / `⌫` | `Delete` / `Backspace` |
| Group | `⌘G` | `Ctrl+G` |
| Ungroup | `⇧⌘G` | `Ctrl+Shift+G` |
| Toggle Outliner (Model Info) | `⇧⌘I` | `Ctrl+Shift+I` |
| Toggle Materials | `⇧⌘C` | `Ctrl+Shift+C` |
| Toggle Components | `⇧⌘M` | `Ctrl+Shift+M` |
| Toggle Tags | `⇧⌘T` | `Ctrl+Shift+T` |
| Toggle Object Info | `⇧⌘O` | `Ctrl+Shift+O` |
| Next Scene / Previous Scene | `Page Down` / `Page Up` | `Page Down` / `Page Up` |
| Library | `⇧L` (viewport) / `⇧⌘L` | `Shift+L` (viewport) / `Ctrl+Shift+L` |
| Settings | `⌘,` | `Ctrl+,` |

## During a gesture

These keys change what the *current tool operation* does. Each is covered in more depth in its tool's own chapter:

| Key | Effect |
|---|---|
| `Esc` | Cancel / step back the current gesture; or exit one editing-context level |
| `Enter` | Commit the typed value |
| Type a number/length | Exact value for the current gesture (no input box needed) — also works right after a commit, redoing what you just made at the new value (every draw tool and Push/Pull, Offset, Move, Rotate, Scale) |
| `Shift` | Lock to the dominant axis (Line mid-chain, Move, Tape Measure) · pin the drawing plane under the cursor before the first click (Line, Rectangle, Circle, Polygon, Arc; `Shift` again or `Esc` releases) · lock the axis/plane (Rotate, Protractor, Slice) · pan while orbiting (Orbit tool; can be pressed or released mid-drag, switching orbit and pan back and forth) |
| `→` / `←` / `↑` | Lock to the X / Y / Z axis (Line, Move, Rotate, Protractor, Slice, Tape Measure); `↓` clears |
| `⌘`/`Ctrl` (held) | Measure without dropping a guide (Tape Measure) · turn off inertia for a precise, 1:1 orbit or pan (Orbit/Pan tools and middle-button orbit); pressing it while the camera is still coasting stops it dead |
| `⌘`/`Ctrl` (tap) | Line, Rectangle, Circle, Polygon, and Arc toggle **Detached**: the shape stays apart from the face or drawing under it (the cursor gains a `+`; stays on until tapped again or the tool changes) · Push/Pull toggles extruding a new object instead of changing the clicked one |
| `Option` (Mac) / `Ctrl` (Windows, Linux) (tap) | Move/Rotate cycles moving/rotating, then a copy (linked by default), then the other kind of copy (independent), then back to moving; the cursor badge reads `+`, `+ independent`, or nothing, and it stays on until tapped again |
| `Option`/`Alt` (press) | Arc cycles how the arc closes: open · pie · segment |
| `3x` or `3/` + `Enter` | Right after a copy commits: multiply it into 3 copies / divide the distance into 3 (Move; `x3` and `/3` work too) |
| `4x,3x` + `Enter` | Right after a copy commits: a grid of 4 copies along the move and 3 along a second direction (an arrow key picks that direction) |
| `Shift`-click | Toggle each clicked object in the selection (Select tool); same for a Shift-drag marquee |
| `⌘`/`Option`-click (Mac) · `Ctrl`/`Alt`-click (Windows, Linux) | Add to the selection, never removing (Select tool); same for a marquee drag. On a Mac, `Ctrl`-click is a right-click, so use `⌘` or `Option` there |
| `Shift` + `⌘`/`Option`-click (Mac) · `Shift` + `Ctrl`/`Alt`-click (Windows, Linux) | Subtract from the selection, never adding (Select tool); same for a marquee drag |
| Triple-click | Select a line's whole connected shape (Select tool) |
| `⌘`/`Ctrl`-click | Paint the whole object instead of one face (Paint tool) |
| Double-click | Enter a group/component/object's editing context; end a Line chain |
| `Delete` / `Backspace` | Delete the selection (any tool) |

## Flying

Hold the **right mouse button** in the viewport to fly (see [Viewing your model](/learn/viewing#fly)). These keys mean movement only while the button is held, or while **Camera ▸ Fly** is switched on for a trackpad; the rest of the time they are the tool shortcuts above.

| Key | Motion |
|---|---|
| `W` / `S` | Forward / back along the view direction |
| `A` / `D` | Strafe left / right |
| `E` / `Q` | Up / down |
| `↑` `↓` `←` `→` | Same as `W` `S` `A` `D` |
| `Shift` (held) | Four times faster |
| Scroll wheel | Faster / slower, for the rest of the session |
| Mouse or trackpad finger (Camera ▸ Fly on) | Look around, no button held |
| Pinch or two-finger scroll (Camera ▸ Fly on) | Faster / slower |
| `Esc` (Camera ▸ Fly on) | Switch Fly off |

## Mouse

| Action | Input |
|---|---|
| Orbit | Middle-button drag (hold `⌘`/`Ctrl` to turn off inertia for a precise orbit) |
| Pan | `Shift` + middle-button drag, or the Pan tool (`H`) |
| Zoom (to cursor) | Scroll wheel |
| Fly | Hold the right button; `W` `A` `S` `D` move, the mouse looks, the cursor hides until you let go |
