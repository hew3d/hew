import { defineCollection } from 'astro:content';
import { glob, file } from 'astro/loaders';
import { z } from 'astro/zod';

const ctaSchema = z.object({
  label: z.string(),
  href: z.string(),
});

const blog = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/blog' }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    pubDate: z.coerce.date(),
    updatedDate: z.coerce.date().optional(),
    draft: z.boolean().default(false),
  }),
});

const learn = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/learn' }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    order: z.number(),
  }),
});

const faq = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/faq' }),
  schema: z.object({
    question: z.string(),
    order: z.number(),
  }),
});

const pages = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/pages' }),
  schema: z.object({
    title: z.string(),
    description: z.string().optional(),
    hero: z
      .object({
        eyebrow: z.string().optional(),
        title: z.string(),
        tagline: z.string(),
        primaryCta: ctaSchema,
        secondaryCta: ctaSchema.optional(),
      })
      .optional(),
    screenshot: z.string().optional(),
    features: z
      .array(
        z.object({
          title: z.string(),
          body: z.string(),
          icon: z.string().optional(),
        })
      )
      .optional(),
    comparisonTitle: z.string().optional(),
    comparisonIntro: z.string().optional(),
    comparison: z
      .array(
        z.object({
          title: z.string(),
          pain: z.string(),
          fix: z.string(),
        })
      )
      .optional(),
    closingCta: z
      .object({
        title: z.string(),
        body: z.string().optional(),
        primaryCta: ctaSchema,
        secondaryCta: ctaSchema.optional(),
      })
      .optional(),
    intro: z.string().optional(),
    webCta: z
      .object({
        label: z.string(),
        href: z.string(),
        blurb: z.string().optional(),
      })
      .optional(),
  }),
});

const releases = defineCollection({
  loader: file('./src/data/releases.json'),
  schema: z.object({
    id: z.enum(['macos', 'windows', 'linux']),
    label: z.string(),
    available: z.boolean(),
    version: z.string().optional(),
    downloadUrl: z.string().optional(),
    formats: z.array(z.string()).optional(),
    note: z.string().optional(),
  }),
});

export const collections = { blog, learn, faq, pages, releases };
