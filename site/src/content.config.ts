import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { z } from 'astro/zod';

const ctaSchema = z.object({
  label: z.string(),
  href: z.string(),
});

const learn = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/learn' }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    order: z.number(),
  }),
});

const compare = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/compare' }),
  schema: z.object({
    title: z.string(),
    package: z.string(),
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
    reel: z
      .object({
        label: z.string(),
        caption: z.string().optional(),
      })
      .optional(),
    screenshots: z
      .array(
        z.object({
          src: z.string(),
          alt: z.string(),
          caption: z.string().optional(),
        })
      )
      .optional(),
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

export const collections = { learn, compare, faq, pages };
