/** True when File > Open of `path` should drop the opened document's remote
 * note (design 7.2): the shell's `library_name_for_path` (via the store's
 * `nameForPath`) says the path is a Library item. The shell canonicalizes
 * both sides, so symlinks, `..`, case-insensitive volumes and odd names
 * agree with where the Library really is. A rejected ask counts as "not a
 * Library item", so the bytes are kept unchanged. */
export async function noteDroppedFor(
  path: string,
  askShell: (path: string) => Promise<string | null>,
): Promise<boolean> {
  try {
    return (await askShell(path)) !== null
  } catch {
    return false
  }
}
