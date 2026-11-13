---
title: "Settings and diagnostics"
description: "Units, how the viewport feels, how copies behave, theme, the server Open on Phone uses, plus Debug Mode and a bug reporter that bundles everything a developer needs into one file."
order: 22
---

Open Settings with `⌘,` / `Ctrl+,`, the Hew ▸ Settings… menu on macOS, the gear at the right end of the menu bar elsewhere, or the command palette. Every control applies immediately — there's no OK button — and settings sync across windows.

![The Settings window on its Units pane, with System and Format selectors](/docs/settings.webp)

## Units

Choose a **System** (Metric or Imperial) and a **Format**:

- Metric: **Meters**, **Centimeters**, or **Millimeters**
- Imperial: **Architectural** (`5' 3-1/8"`), **Fractional inches** (`60-1/8"`), or **Decimal inches** (`60.125"`)

This controls how every length is displayed and how bare typed numbers are interpreted. Geometry itself is always stored in meters, so switching formats never changes your model, and you can always type any unit explicitly regardless of the display setting ([full input reference](/learn/measurement-and-guides/)). The welcome screen offers the same choice as a single flat dropdown; both set the same persisted default.

### Precision

**Precision** sets how far a length is rounded. Each format remembers its own choice, and the list changes when you change the Format. The defaults are the rounding Hew has always used: Meters 3 decimals, Centimeters 2, Millimeters 1, Decimal inches 3, and the nearest 1/16" for Architectural and Fractional inches.

- **Exact** shows the digits the value needs and no more: `0.5 m`, `5' 3-1/8"`, `9.399 mm`. A length that never ends stops at a sensible limit, so a third of a meter reads `0.333333 m`. In imperial, Exact uses the smallest fraction down to 1/64" that is exactly right, and switches to decimal inches (`1' 2.3333"`) when no fraction is.
- Meters, Centimeters, Millimeters, and Decimal inches round to a number of decimals, from **Whole** (0) up to 6 decimals.
- Architectural and Fractional inches round to the nearest 1', 1", 1/2", 1/4", 1/8", 1/16", 1/32", or 1/64". The 1' choice rounds to the nearest 12".
- When rounding changed the value, the readout starts with `~` (`~0.33 m`, `~3/16"`, `~1' 2-1/8"`). A value that already sits on the grid shows no `~`.

Precision applies to every length Hew shows, including the measurement box, the Tape Measure, Object Info, and dimension text (a dimension whose text you typed over keeps your text). You can type a value with or without the leading `~`. When Hew saves a SketchUp file, it leaves the `~` out.

## View

The View pane is about how the viewport feels. Everything in it starts out the way Hew has always behaved, so nothing changes until you pick.

- **Zoom wheel speed** is a five-stop slider from Slow to Fast. It scales how far one turn of the mouse wheel moves the camera; the middle stop is the usual speed.
- **Smooth camera moves** is on by default. Views, scenes, and other camera moves glide into place, and orbiting coasts to a stop after you let go. Turn it off and the camera jumps straight to where it is going and stops the moment you release. It wins over **View ▸ Scenes ▸ Scene Transitions**: with smooth moves off, scenes always snap.
- **At the cursor** has two switches. **Show tool hints** controls the small labels that follow the pointer, like Endpoint or On Edge. **Show tool pointer** controls the cursor that is shaped like your current tool; turn it off and you get the plain arrow. The hint line in the status bar stays either way.
- **Measurements box** puts the box that shows the number you are typing in the **Top right** (the default) or **Bottom right** of the viewport. The fly speed readout moves with it.

## Editing

**Copies are linked** is on by default. A copy made with Move, Rotate, Paste, or an array shares its shape with the original, so editing one changes all of them ([Linked copies](/learn/groups-and-components/#linked-copies)). Turn it off and copies come out independent; you can still ask for a linked copy each time (the second tap of the copy key in Move and Rotate, or **Edit ▸ Paste linked**). Either way the copy key cycles Copy, the other kind of copy, and back to moving. The choice is yours alone and isn't saved in the file.

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

**Help ▸ Report Bug…** opens a dialog with a description field and a checklist of what it can include: your app version and system details, the session's recorded steps, the files the session took in, your current model, and the recent diagnostic log. Raw input events are listed too, but only when Debug Mode is on. Each row shows its size, and a **Show** link next to it previews exactly what would be sent. Untick anything you'd rather leave out. Recorded steps and the model carry your model's geometry, and imported files are the originals byte for byte: files you imported or opened, such as a `.skp`, and textures and library items you added. If a file wouldn't open or import, a **File that didn't open** row offers that file too, so the developer can open it and see what went wrong. Untick all of these to send only your description, system details, and the log.

**Send report** compresses the report and submits it privately to the Hew developer; nothing is posted publicly. The dialog shows how large the upload will be. Past 1 GB compressed it leaves out the model file, then imported files, then the file that didn't open, and marks each one; **Save to file…** always keeps everything you ticked. You get back a short report ID. From there you can optionally open a public GitHub issue too, prefilled with your description, version, and that ID, but never with any attachment. See [what a report contains and how long it's kept](/privacy/bug-reports/).

With **Let me know what happens with this report** ticked (it is on by default, and Hew remembers your choice), Hew keeps a private key for the report on this computer and checks for a reply when it starts. When the developer answers, a dismissible bar appears at the top of the window, and **Help ▸ Sent Reports…** lists the report with its status and the conversation. There you can reply in plain text, up to 4,000 characters per reply and 20 replies a day per report. **Attach a file…** sends a file to the developer as a follow-up; if a file failed to open in this session, a one-click button attaches that file instead. **Check now** looks for news straight away, and **Forget this report** stops following it. The report itself stays with the developer. No account or email address is needed. See [what is stored and when Hew checks](/privacy/bug-reports/#following-a-report).

If Hew stops on its error screen, click **Report this crash** there instead — it opens the same dialog, already carrying the steps that led to the crash (the model itself isn't available after a crash). A self-hosted web build, or a plain local dev build, has nowhere to send a report to; there the dialog explains that and offers **Save to file…** instead, so you can attach the file to a GitHub issue by hand.

Hew's kernel is deterministic, so a captured session usually reproduces a bug exactly. Reports with recorded steps attached tend to get fixed fast.
