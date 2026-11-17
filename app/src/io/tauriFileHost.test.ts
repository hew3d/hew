/**
 * io/tauriFileHost.ts — the desktop host's picture pick: a cancel, a read,
 * and a pick whose bytes cannot be read, which must reject with
 * FileReadError (the FileHost contract) so both shells say so instead of
 * treating it as a cancel.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { FileReadError, UnsupportedFileError } from './fileHost'
import { TauriFileHost, resolveImportPickFromPath } from './tauriFileHost'

const mockInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))

beforeEach(() => mockInvoke.mockReset())

describe('TauriFileHost.openImage', () => {
  it('answers null when the picker is canceled, reading nothing', async () => {
    mockInvoke.mockResolvedValueOnce(null)
    await expect(new TauriFileHost().openImage()).resolves.toBeNull()
    expect(mockInvoke).toHaveBeenCalledTimes(1)
  })

  it('reads the picked picture, named after its file', async () => {
    mockInvoke.mockResolvedValueOnce('/Users/someone/Pictures/wall.png')
    mockInvoke.mockResolvedValueOnce(new Uint8Array([1, 2, 3]).buffer)
    const got = await new TauriFileHost().openImage()
    expect(got?.name).toBe('wall.png')
    expect(Array.from(got?.bytes ?? [])).toEqual([1, 2, 3])
    expect(mockInvoke).toHaveBeenLastCalledWith('read_file', { path: '/Users/someone/Pictures/wall.png' })
  })

  it('a picked picture whose bytes cannot be read rejects with FileReadError, not a cancel', async () => {
    const reason = new Error('No such file or directory')
    mockInvoke.mockResolvedValueOnce('/Users/someone/Pictures/wall.png')
    mockInvoke.mockRejectedValueOnce(reason)
    const err = await new TauriFileHost().openImage().catch((e: unknown) => e)
    expect(err).toBeInstanceOf(FileReadError)
    expect((err as FileReadError).fileName).toBe('wall.png')
    expect((err as FileReadError).reason).toBe(reason)
  })
})

describe('TauriFileHost.saveAs', () => {
  it('writes a doubled extension from the dialog back as a single one', async () => {
    mockInvoke.mockResolvedValueOnce('/d/kb10test.hew.hew') // pick_save_path
    mockInvoke.mockResolvedValueOnce(undefined) // write_file_bytes
    const ref = await new TauriFileHost().saveAs(new Uint8Array([1]), 'Untitled.hew', 'hew')
    expect(ref).toMatchObject({ name: 'kb10test.hew', handle: '/d/kb10test.hew' })
    expect(mockInvoke).toHaveBeenLastCalledWith('write_file_bytes', new Uint8Array([1]), {
      headers: { path: encodeURIComponent('/d/kb10test.hew') },
    })
  })
})

describe('Tauri import dispatch by extension', () => {
  const bytes = new Uint8Array([1, 2, 3])

  it.each(['/d/notes.txt', '/d/tool.hewplugin', '/d/README', '/d/model.dae.bak'])(
    'refuses %s before any directory scan',
    async (path) => {
      const err = await resolveImportPickFromPath(path, bytes).catch((e: unknown) => e)
      expect(err).toBeInstanceOf(UnsupportedFileError)
      expect(mockInvoke).not.toHaveBeenCalledWith('list_dir', expect.anything())
    },
  )

  it('an upper-case .DAE is still COLLADA', async () => {
    mockInvoke.mockResolvedValue([]) // list_dir: no textures
    const pick = await resolveImportPickFromPath('/d/MODEL.DAE', bytes)
    expect(pick).toMatchObject({ kind: 'dae', name: 'MODEL.DAE', path: '/d/MODEL.DAE' })
  })

  it('openAny() refuses a picked file of an unknown kind', async () => {
    mockInvoke.mockResolvedValueOnce('/d/notes.txt') // pick_open_path
    mockInvoke.mockResolvedValueOnce(bytes.buffer) // read_file
    await expect(new TauriFileHost().openAny()).rejects.toBeInstanceOf(UnsupportedFileError)
  })

  it('openForImport() refuses a picked file of an unknown kind', async () => {
    mockInvoke.mockResolvedValueOnce('/d/notes.txt')
    mockInvoke.mockResolvedValueOnce(bytes.buffer)
    await expect(new TauriFileHost().openForImport()).rejects.toBeInstanceOf(UnsupportedFileError)
  })
})
