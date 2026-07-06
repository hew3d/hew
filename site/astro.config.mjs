// hew3d.com — static marketing site. `site` is required by @astrojs/rss and
// @astrojs/sitemap; the passthrough image service avoids a sharp native dep
// (all images are served verbatim from public/).
import { defineConfig, passthroughImageService } from 'astro/config';
import sitemap from '@astrojs/sitemap';

export default defineConfig({
  site: 'https://hew3d.com',
  integrations: [sitemap()],
  image: { service: passthroughImageService() },
});
