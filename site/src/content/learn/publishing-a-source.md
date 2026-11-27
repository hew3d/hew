---
title: "Publishing a source"
description: "Share your Library items from GitHub, GitLab, Codeberg, or your own website, so other people can add them from inside Hew."
order: 15.1
---

A source is a folder of Library items plus one file, `hew.json`, that lists them. Put that folder on GitHub, GitLab, Codeberg, or any website, and anyone can add it in Hew's Library with **Add a source…** and its address. [The Library](/learn/library/#sources) chapter covers what your users see.

You don't need permission from anyone to publish. The Hew catalog is an optional extra step that makes a source show up under **Browse the catalog**.

The steps use `hew-cli`, a command-line program that comes with the desktop app. [The API guide](https://github.com/hew3d/hew/blob/main/docs/API_GUIDE.md#install-hew-cli) says where it is on each platform.

## 1. Collect the items

Save each thing you want to share to your Library, then copy those `.hew` files out of your Library folder into a new folder. Arrange them in subfolders any way you like. Files and folders whose names start with a dot are skipped.

Every file must be a Library item. A model you saved with **File ▸ Save** is not one until you save it to the Library.

An item keeps its identity when you publish it and when someone adds it. That is how Hew knows, later, that your new version replaces the one a person already has. Keep editing the same items; don't re-create them.

## 2. Write `hew.json`

```sh
hew-cli library index ./parts --name "Shop Hardware" --license CC-BY-4.0
```

The command writes two things into the folder: `hew.json`, which lists every item with its size and a checksum, and `thumbnails/`, with one picture per item.

| Option | What it sets |
|---|---|
| `--name` | The source's name. Required the first time. |
| `--description` | A sentence about what the source holds. |
| `--author` | Your name, or your shop's. |
| `--homepage` | An `https` address for more about the source. |
| `--license` | The license the items are shared under. |
| `--license-url` | An `https` address for the license text. |

Later runs keep what you set before, and an option replaces only its own field.

State a license. Hew shows it to people before they take an item, and a source with none reads **No license stated**. Write it as an [SPDX identifier](https://spdx.org/licenses/) such as `CC-BY-4.0`, `CC0-1.0`, or `MIT`. For terms of your own, use `LicenseRef-` followed by a short name, and give `--license-url`.

If a file in the folder is not a Library item, the command names it, stops, and writes nothing. Save that file to the Library or move it out of the folder. An item whose picture can't be drawn is listed without one, with a warning.

## 3. Run it again after every change

`hew.json` records a checksum of each item, and Hew refuses an item that doesn't match its checksum. Whenever you add, change, or remove an item, run `hew-cli library index ./parts` again before you publish.

To find out whether `hew.json` is current without writing anything:

```sh
hew-cli library index ./parts --check
```

It prints `hew.json is up to date` when it is. Otherwise it lists each file that is missing, added, or changed, and exits with status 1, so you can run it in a script before every upload.

## 4. Put the folder online

Publish the whole folder: the items, `hew.json`, and `thumbnails/`.

**On GitHub, GitLab, or Codeberg**, commit the folder to a public repository. The address people paste into Hew is the repository's page, or the short form:

| Host | Short form |
|---|---|
| GitHub | `github:owner/repo` |
| GitLab | `gitlab:group/project` |
| Codeberg | `codeberg:owner/repo` |
| Another Forgejo or Gitea site | `forgejo:host/owner/repo` |

Two additions work on every short form. `//folder` names a folder inside the repository when `hew.json` isn't at the top: `github:owner/repo//parts`. `@name` reads a branch or a tag instead of the default branch: `github:owner/repo@stable`, `codeberg:owner/repo@v2`. Both work on GitHub, GitLab, Codeberg, and other Forgejo or Gitea sites. The name can't contain a `/`.

Private repositories don't work. Hew reads a source without signing in.

**On any other website**, upload the folder and give people the address of `hew.json`, such as `https://example.org/parts/hew.json`. The site has to meet three conditions:

- It serves the files over `https`.
- It doesn't ask for a login.
- It sends the header `Access-Control-Allow-Origin: *` with `hew.json`, with every item, and with every thumbnail. The web version of Hew runs in a browser, and a browser reads another site only when that site allows it. Without the header your source works in the desktop app and fails in the browser. GitHub, GitLab.com, Codeberg, and gitea.com already send it.

## 5. Check it

```sh
hew-cli catalog check github:owner/repo
```

The command reads your source the way Hew does and prints one line for each problem. No output means it found none. Each line starts with a code:

| Code | What is wrong |
|---|---|
| `reference` | The address isn't in its standard short form. The message gives the form to use. |
| `index_invalid` | `hew.json` is missing, damaged, or breaks a rule of the format. |
| `license_missing` | `hew.json` states no license. |
| `license_format` | A license isn't an SPDX identifier or expression such as `CC-BY-4.0` or `MIT OR Apache-2.0`, or it is a `LicenseRef-` name with no `https` `license-url`. |
| `file_integrity` | An item doesn't match its checksum or size, or a thumbnail isn't a PNG picture. Run `library index` again and publish again. |
| `item_mismatch` | An item doesn't open in Hew, or `hew.json` disagrees with the item about what it is. |
| `ids_unique` | Two entries are the same item. This happens when a file was copied in a file manager and not saved as a new item. |
| `public_https` | An address isn't `https`, or leads somewhere that isn't on the public internet. |
| `cors` | The `Access-Control-Allow-Origin` header is missing. |
| `rate_limited` | The host is limiting requests. Wait and run it again. |
| `unreachable` | The host didn't answer. |

GitHub limits how often a program may ask it questions without signing in. If you hit `rate_limited` there, set the environment variable `GITHUB_TOKEN` to a GitHub token and the check uses it for GitHub alone.

## 6. Tell people, or ask to be listed

Your source works now. Give people its address and they add it with **Add a source…**. Hew tells them it isn't in the catalog and that nobody has checked it.

To have it appear under **Browse the catalog**, open a pull request on [`hew3d/catalog`](https://github.com/hew3d/catalog) that adds your source's short form to `library.json`. A listed source needs a license, has to be readable from the browser, and must pass `hew-cli catalog check` with no output. A maintainer reviews each request. The catalog's own [publishing notes](https://github.com/hew3d/catalog/blob/main/PUBLISHING.md) have the details.

Being listed changes nothing about where your items live or how they update. Hew always reads items from your source as it is now, so a new version reaches people as soon as you publish it, with no step at the catalog.

## Limits

- One item can be up to 64 MB, and a source can hold up to 20,000 items. `hew.json` itself can be up to 4 MB, and a thumbnail up to 1 MB.
- An item's name can be up to 200 characters. It can have up to 32 keywords of up to 64 characters each.
- Hew checks each item against its checksum, which catches a damaged or half-uploaded file. Items aren't signed, so the check does not prove who published them.
- People who took an item keep it if you remove it from your source, or take the source down.
