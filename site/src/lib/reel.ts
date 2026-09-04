// Where the landing reel comes from. The published manifest
// (src/data/videos.json, key "reel") wins; HEW_REEL_LOCAL=1 previews the
// files assemble-reel.py wrote to public/videos/ before they are uploaded.
// Null means "not published yet" — the landing page then renders no reel
// (the screenshot rotator further down is unaffected).
import manifest from '../data/videos.json';

export interface ReelMedia {
  webm: string;
  mp4: string;
  poster: string;
}

interface Published {
  reel?: { webm: { url: string }; mp4: { url: string }; poster: { url: string } };
}

export function resolveReel(): ReelMedia | null {
  const published = (manifest as Published).reel;
  if (published) {
    return { webm: published.webm.url, mp4: published.mp4.url, poster: published.poster.url };
  }
  if (process.env.HEW_REEL_LOCAL) {
    return { webm: '/videos/reel.webm', mp4: '/videos/reel.mp4', poster: '/videos/reel.jpg' };
  }
  return null;
}
