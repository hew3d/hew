// hew3d.com — static marketing site. `site` is required by @astrojs/sitemap;
// the passthrough image service avoids a sharp native dep (all images are
// served verbatim from public/).
import { readFileSync, readdirSync } from 'node:fs';
import { defineConfig, passthroughImageService } from 'astro/config';
import sitemap from '@astrojs/sitemap';

// Fail the build when a <docs-video slug="..."> in a Learn chapter has no
// entry in src/data/videos.json. Without this the mismatch is silent: the
// custom element resolves slugs at runtime in the browser, so astro
// check/build stay green and the page just shows a gap where the clip
// should be. Runs in every build path (verify.sh and the Cloudflare Pages
// deploy both go through `astro build`).
const docsVideoCheck = () => ({
  name: 'docs-video-check',
  hooks: {
    'astro:build:start': () => {
      const manifest = JSON.parse(
        readFileSync(new URL('./src/data/videos.json', import.meta.url), 'utf8'),
      );
      const learnDir = new URL('./src/content/learn/', import.meta.url);
      const missing = [];
      for (const file of readdirSync(learnDir)) {
        if (!file.endsWith('.md')) continue;
        const body = readFileSync(new URL(file, learnDir), 'utf8');
        for (const m of body.matchAll(/<docs-video\b[^>]*\bslug="([^"]+)"/g)) {
          if (!(m[1] in manifest.videos)) missing.push(`${file}: slug "${m[1]}"`);
        }
      }
      if (missing.length > 0) {
        throw new Error(
          'docs-video slugs missing from src/data/videos.json:\n  ' +
            missing.join('\n  '),
        );
      }
    },
  },
});

export default defineConfig({
  site: 'https://hew3d.com',
  integrations: [sitemap(), docsVideoCheck()],
  image: { service: passthroughImageService() },
});
