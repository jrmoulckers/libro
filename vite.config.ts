import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { VitePWA } from 'vite-plugin-pwa';

export default defineConfig({
  plugins: [
    svelte(),
    // Installable PWA + offline support. `manifest` colors are provisional neutral
    // values (documented): the @jrm/tokens sync doesn't exist yet, and the manifest
    // lives outside the value-free app.css token system, so these are the one place
    // a fixed color is intentional. `autoUpdate` = Workbox skipWaiting + clientsClaim
    // (no stale-SW trap, no update prompt to dismiss).
    VitePWA({
      registerType: 'autoUpdate',
      includeAssets: ['icon.svg'],
      manifest: {
        name: 'Libro',
        short_name: 'Libro',
        description: 'A pure-client media hub for books, audiobooks, and your personal library.',
        start_url: '/',
        scope: '/',
        display: 'standalone',
        orientation: 'portrait-primary',
        background_color: '#14161c',
        theme_color: '#14161c',
        icons: [
          { src: 'pwa-192.png', sizes: '192x192', type: 'image/png' },
          { src: 'pwa-512.png', sizes: '512x512', type: 'image/png' },
          {
            src: 'pwa-maskable-512.png',
            sizes: '512x512',
            type: 'image/png',
            purpose: 'maskable',
          },
        ],
      },
      workbox: {
        // Precache the whole static build: the app shell AND every lazy chunk
        // (reader / player / enrich / sync / plugins / fflate) + icons + manifest,
        // so opening a book or the player works fully offline after first load.
        globPatterns: ['**/*.{js,css,html,svg,png,webmanifest}'],
        cleanupOutdatedCaches: true,
        runtimeCaching: [
          {
            // Book covers (cross-origin http(s) images) — cache-first with a cap so
            // browsing offline shows covers seen before, without unbounded growth.
            urlPattern: ({ request }) => request.destination === 'image',
            handler: 'CacheFirst',
            options: {
              cacheName: 'libro-covers',
              expiration: { maxEntries: 200, maxAgeSeconds: 60 * 60 * 24 * 30 },
              cacheableResponse: { statuses: [0, 200] },
            },
          },
          {
            // Public metadata APIs (Open Library / Google Books) — inherently online;
            // network-first with a short timeout, falling back to cache so a flaky
            // connection degrades gracefully instead of failing hard.
            urlPattern: ({ url }) =>
              url.hostname === 'openlibrary.org' ||
              url.hostname === 'covers.openlibrary.org' ||
              url.hostname === 'www.googleapis.com',
            handler: 'NetworkFirst',
            options: {
              cacheName: 'libro-metadata-api',
              networkTimeoutSeconds: 5,
              expiration: { maxEntries: 100, maxAgeSeconds: 60 * 60 * 24 * 7 },
              cacheableResponse: { statuses: [0, 200] },
            },
          },
        ],
      },
    }),
  ],
  build: {
    outDir: 'dist',
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.{test,spec}.ts'],
  },
});
