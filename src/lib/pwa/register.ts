/// <reference types="vite-plugin-pwa/client" />
/**
 * Service-worker registration — the thin runtime shell over vite-plugin-pwa's
 * generated Workbox worker.
 *
 * The worker is built with `registerType: 'autoUpdate'` (Workbox `skipWaiting` +
 * `clientsClaim`), so a freshly deployed shell activates and takes control without
 * trapping the user on a stale service worker and without an update prompt to
 * dismiss. `registerSW` is a no-op in browsers without service-worker support and
 * is never imported by tests (jsdom has no SW), so the pure logic stays testable
 * while this wiring is covered by build + typecheck.
 */
import { registerSW } from 'virtual:pwa-register';

/** Register the generated service worker (call once, from `main.ts`). */
export function registerServiceWorker(): void {
  if (typeof window === 'undefined' || !('serviceWorker' in navigator)) return;
  registerSW({ immediate: true });
}
