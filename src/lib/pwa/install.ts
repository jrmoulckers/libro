/**
 * Install (Add-to-Home-Screen) UX helpers.
 *
 * The pure {@link shouldShowInstall} decision is unit-tested; the `beforeinstallprompt`
 * capture, the prompt trigger, and standalone detection are the thin DOM shell wired
 * in `App.svelte`. Everything is **feature-detected**: on browsers that never fire
 * `beforeinstallprompt` (notably iOS Safari) the affordance simply stays hidden, and
 * users install via the browser's own "Add to Home Screen" — see the report/UI hint.
 */

/**
 * The (non-standard but widely-shipped) `beforeinstallprompt` event. Chromium fires
 * it when the app is installable; we stash it and replay `prompt()` on user click.
 */
export interface BeforeInstallPromptEvent extends Event {
  readonly platforms: readonly string[];
  prompt(): Promise<void>;
  readonly userChoice: Promise<{ outcome: 'accepted' | 'dismissed'; platform: string }>;
}

/** Inputs to the install-affordance visibility decision. */
export interface InstallState {
  /** A captured, not-yet-consumed `beforeinstallprompt` event (or `null`). */
  promptAvailable: boolean;
  /** The app has been installed this session (`appinstalled` fired). */
  installed: boolean;
  /** The app is already running installed (standalone display mode). */
  standalone: boolean;
}

/**
 * Whether to show the "Install" affordance: only when the browser has offered an
 * install prompt AND the app isn't already installed/standalone. Pure.
 */
export function shouldShowInstall(state: InstallState): boolean {
  return state.promptAvailable && !state.installed && !state.standalone;
}

/**
 * Whether the app is currently running as an installed/standalone PWA. Covers the
 * standard `display-mode: standalone` media query and the iOS Safari
 * `navigator.standalone` flag. Off-DOM this is `false`.
 */
export function isStandalone(): boolean {
  if (typeof matchMedia === 'function' && matchMedia('(display-mode: standalone)').matches) {
    return true;
  }
  return (
    typeof navigator !== 'undefined' &&
    (navigator as Navigator & { standalone?: boolean }).standalone === true
  );
}
