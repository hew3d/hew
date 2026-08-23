---
title: "Settings and diagnostics"
description: "Units and theme, the server Open on Phone uses, plus Debug Mode and a bug reporter that bundles everything a developer needs into one file."
order: 22
---

Open Settings with `⌘,` / `Ctrl+,`, the Hew ▸ Settings… menu on macOS, the gear at the right end of the menu bar elsewhere, or the command palette. Every control applies immediately — there's no OK button — and settings sync across windows.

![The Settings window on its Units pane, with System and Format selectors](/docs/settings.webp)

## Units

Choose a **System** (Metric or Imperial) and a **Format**:

- Metric: **Meters**, **Centimeters**, or **Millimeters**
- Imperial: **Architectural** (`5' 3-1/8"`), **Fractional inches** (`60-1/8"`), or **Decimal inches** (`60.125"`)

This controls how every length is displayed and how bare typed numbers are interpreted. Geometry itself is always stored in meters, so switching formats never changes your model, and you can always type any unit explicitly regardless of the display setting ([full input reference](/learn/measurement-and-guides/)). The welcome screen offers the same choice as a single flat dropdown; both set the same persisted default.

## Theme

**Auto** follows your operating system's light/dark appearance, live. Pick **Light** or **Dark** to override.

## Advanced: your own server

The Advanced pane (desktop only) has one job: choosing where **Open on Phone** sends a model. **Hew cloud** is the default. **Self-hosted** takes the address of a server you run yourself, plus an optional **Upload key** if that server requires one. **Test connection** confirms the address answers as a Hew relay before you rely on it, and the QR code you get from File ▸ Open on Phone… then points at that server, never at app.hew3d.com. What your phone has to do differently, and how to stand up a server, is covered in [Using your own server](/learn/hew-on-your-phone/#using-your-own-server).

## Debug Mode

The Debug pane's **Enable Debug Mode** checkbox turns on deeper diagnostics for chasing a problem or helping report one:

- A rolling **diagnostic log** — on desktop it's written to the app's log directory as `diagnostic.log`; on the web there's a *Download Diagnostic Log…* button.
- **Input recording**, capturing low-level interaction events.
- Kernel **torture mode**, extra internal validation after every operation. Noticeably slower; leave it off for normal modeling.

## Reporting a bug

**Help ▸ Report Bug…** assembles everything a developer needs into a single file: app version and system info, your current model, the recent diagnostic log, and the recorded input events. On desktop it's saved to the app's log directory (the confirmation toast shows the exact path); on the web, it downloads. Attach that file to your bug report.

Hew's kernel is deterministic, so a captured session usually reproduces a bug exactly. Reports with a bundle attached tend to get fixed fast.
