/**
 * Theme resolution + application.
 *
 * The studio rule (AGENTS.md) is that appearance switches **only** via
 * `document.documentElement.dataset.theme` — `dark` sets the attribute, and
 * **light removes it** (light is the default, attribute-absent state). The actual
 * color *values* for each theme arrive later with the `@jrm/tokens` sync; this
 * module never hardcodes a color. `src/app.css` therefore stays value-free: the
 * toggle only flips the attribute, and the vendored token stylesheet will supply
 * the `[data-theme='dark'] { … }` values once it lands.
 *
 * The pure pieces ({@link resolveInitialTheme}, {@link nextTheme}, {@link isTheme})
 * are unit-tested; the `localStorage`/`matchMedia`/DOM helpers are the thin shell.
 */

/** The themes exposed by the minimal P10 toggle. */
export type Theme = 'light' | 'dark';

/** `localStorage` key for the user's explicit choice. */
export const THEME_STORAGE_KEY = 'libro:theme';

/** Narrow an arbitrary value to a {@link Theme}. */
export function isTheme(value: unknown): value is Theme {
  return value === 'light' || value === 'dark';
}

/**
 * Resolve the theme to apply on first load: an explicit stored choice wins;
 * otherwise fall back to the OS `prefers-color-scheme`. Pure.
 */
export function resolveInitialTheme(prefersDark: boolean, stored: string | null): Theme {
  if (isTheme(stored)) return stored;
  return prefersDark ? 'dark' : 'light';
}

/** The other theme — used by the toggle. Pure. */
export function nextTheme(current: Theme): Theme {
  return current === 'dark' ? 'light' : 'dark';
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
 * Apply a theme to the document root. Per the studio rule, `light` **removes** the
 * attribute (default state) and `dark` sets it; the token stylesheet keys off
 * `[data-theme='dark']`.
 */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === 'light') delete root.dataset.theme;
  else root.dataset.theme = theme;
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
