// Which Learn chapters are published.
//
// A chapter about a Preview feature (the compile-time wall in the repo's
// preview-features.json) must not reach the public site until the feature
// graduates. Such a chapter names the feature ids in its frontmatter:
//
//   preview: [fillet]
//
// and stays out of the built site, the chapter list, the previous/next
// links and the sitemap, because every one of those is built from the list
// this module returns.
//
// An id that is not in the registry fails the build. That is how graduation
// is noticed: deleting a feature's registry entry breaks the build at every
// chapter that still names it, and the message says what to do (take the id
// off the chapter, which publishes it; for a `preview-additions/` chapter,
// move its sections into the chapters they name and delete the file).
import { getCollection, type CollectionEntry } from 'astro:content';
import registry from '../../../preview-features.json';

export type LearnEntry = CollectionEntry<'learn'>;

const KNOWN = new Set(registry.features.map((f) => f.id));

/** The registry ids a chapter is held back for; empty means it is published. */
export function heldBackFor(entry: LearnEntry): string[] {
  const ids = entry.data.preview ?? [];
  const unknown = ids.filter((id) => !KNOWN.has(id));
  if (unknown.length > 0) {
    throw new Error(
      `learn/${entry.id}.md: preview feature "${unknown.join('", "')}" is not in preview-features.json. ` +
        `If it graduated, remove it from this chapter's \`preview:\` list` +
        (entry.id.startsWith('preview-additions/')
          ? ' once its sections are moved into the chapters they name (each section says which), and delete this file.'
          : ' (the chapter then publishes).'),
    );
  }
  return ids;
}

/** Every published Learn chapter, in no particular order. */
export async function getPublishedLearn(): Promise<LearnEntry[]> {
  const all = await getCollection('learn');
  return all.filter((entry) => heldBackFor(entry).length === 0);
}
