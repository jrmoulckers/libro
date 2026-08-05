/**
 * Theme resolution + application.
 *
 * The studio token model (see vendor/@jrm/tokens): `:root` in tokens.css is the LIGHT
 * base, and `[data-theme="dark"]` / `[data-theme="dark-oled"]` / `[data-theme="high-contrast"]`
 * restate only their semantic colors. `index.css` also carries a
 * `@media (prefers-color-scheme: dark) :root:not([data-theme])` fallback that flips an
 * *unpinned* root to the dark palette on a dark OS.
 *
 * Because of that fallback we must always pin an **explicit** `data-theme` — including for
 * light. If light removed the attribute, a user who deliberately chose light would still see
 * dark on a dark-OS device (the `:not([data-theme])` fallback would win). So `applyTheme`
 * never deletes the attribute; `light` is set as literally `data-theme="light"`, which
 * suppresses the fallback and lets the `:root` light base show through.
 *
 * The pure pieces ({@link resolveInitialTheme}, {@link nextTheme}, {@link isTheme},
 * {@link themeLabel}) are unit-tested; the `localStorage`/`matchMedia`/DOM helpers are the
 * thin shell.
 */

/** The themes libro exposes, in the order the toggle cycles through them. */
export const THEMES = ['light', 'dark', 'dark-oled', 'high-contrast'] as const;

/** One of the studio appearance modes. */
export type Theme = (typeof THEMES)[number];

/** `localStorage` key for the user's explicit choice. */
export const THEME_STORAGE_KEY = 'libro:theme';

/** Human-facing labels for each theme (used by the toggle control). */
const THEME_LABELS: Record<Theme, string> = {
  light: 'Light',
  dark: 'Dark',
  'dark-oled': 'Dark OLED',
  'high-contrast': 'High contrast',
};

/** Narrow an arbitrary value to a {@link Theme}. */
export function isTheme(value: unknown): value is Theme {
  return typeof value === 'string' && (THEMES as readonly string[]).includes(value);
}

/** A short, human label for a theme — e.g. for the toggle button. Pure. */
export function themeLabel(theme: Theme): string {
  return THEME_LABELS[theme];
}

/**
 * Resolve the theme to apply on first load: an explicit stored choice wins;
 * otherwise fall back to the OS `prefers-color-scheme`. Pure.
 */
export function resolveInitialTheme(prefersDark: boolean, stored: string | null): Theme {
  if (isTheme(stored)) return stored;
  return prefersDark ? 'dark' : 'light';
}

/** The next theme in the cycle — used by the toggle. Wraps around. Pure. */
export function nextTheme(current: Theme): Theme {
  const index = THEMES.indexOf(current);
  return THEMES[(index + 1) % THEMES.length];
}

/** Read the OS dark-mode preference (defaults to `false` off-DOM). */
export function prefersDark(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches;
}

/** Read the persisted theme choice, tolerating unavailable storage. */
export function readStoredTheme(): string | null {
  try {
    return localStorage.getItem(THEME_STORAGE_KEY);
  } catch {
    return null;
  }
}

/** Persist the theme choice, tolerating unavailable storage. */
export function persistTheme(theme: Theme): void {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // Private mode / disabled storage — the choice just won't survive reloads.
  }
}

/**
 * Apply a theme to the document root. Every theme — including `light` — is set as an
 * **explicit** `data-theme` value; the attribute is never removed. This pins the palette so
 * the token stylesheet's `prefers-color-scheme` fallback can't override an explicit choice.
 */
export function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}

/**
 * Resolve + apply the initial theme before first paint (called from `main.ts`).
 * Returns the resolved theme so the shell can seed its reactive state.
 */
export function applyInitialTheme(): Theme {
  const theme = resolveInitialTheme(prefersDark(), readStoredTheme());
  applyTheme(theme);
  return theme;
}
