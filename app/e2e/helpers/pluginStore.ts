import type { Page } from '@playwright/test'

/**
 * What a preview build's plugin store leaves behind in browser storage: a
 * record, a file, a half-written staging entry and a settings key. Specs seed
 * it, load the app with the Plugins switch off (or in a production build), and
 * check that every byte is where it was.
 */

/** Writes the sample store into the page's origin-private file system and local storage. */
export async function seedPluginStore(page: Page): Promise<void> {
  await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory()
    const plugins = await root.getDirectoryHandle('plugins', { create: true })
    const put = async (dir: FileSystemDirectoryHandle, name: string, text: string) => {
      const w = await (await dir.getFileHandle(name, { create: true })).createWritable()
      await w.write(text)
      await w.close()
    }
    const one = await plugins.getDirectoryHandle('com.example.x', { create: true })
    await put(one, 'installed.json', '{"schema":1,"id":"com.example.x","version":"1.0.0"}')
    await put(one, 'hew-plugin.json', '{"manifest":1}')
    const files = await one.getDirectoryHandle('files', { create: true })
    await put(files, 'main.js', '// kept')
    const staging = await plugins.getDirectoryHandle('.staging', { create: true })
    const half = await staging.getDirectoryHandle('com.example.y-0123456789abcdef0123456789abcdef', { create: true })
    await put(half, 'installed.json', 'half written')
    localStorage.setItem('hew.plugin.com.example.x.test', 'kept')
  })
}

/** Every file under `plugins/` as path -> text, plus the plugin settings key. */
export async function dumpPluginStore(page: Page): Promise<Record<string, string>> {
  return page.evaluate(async () => {
    const out: Record<string, string> = {}
    const walk = async (dir: FileSystemDirectoryHandle, prefix: string): Promise<void> => {
      for await (const [name, handle] of dir.entries()) {
        if (handle.kind === 'directory') await walk(handle as FileSystemDirectoryHandle, `${prefix}${name}/`)
        else out[`${prefix}${name}`] = await (await (handle as FileSystemFileHandle).getFile()).text()
      }
    }
    const root = await navigator.storage.getDirectory()
    try {
      await walk(await root.getDirectoryHandle('plugins'), 'plugins/')
    } catch {
      /* no plugins folder */
    }
    out['localStorage:hew.plugin.com.example.x.test'] = localStorage.getItem('hew.plugin.com.example.x.test') ?? '(gone)'
    return out
  })
}
