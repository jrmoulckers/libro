/**
 * Theme resolution + application.
 *
 * The studio token model (see `vendor/@jrm/tokens`): `:root` in `tokens.css` is the LIGHT
 * base, and `[data-theme="dark"]`, `[data-theme="dark-oled"]`, `[data-theme="high-contrast"]`,
 * `[data-theme="high-contrast-dark"]` each restate only their semantic colors.
 *
 * `index.css` additionally carries three preference-driven fallbacks, every one of them
 * scoped to `:root:not([data-theme])` — i.e. they apply only while the consumer has *not*
 * pinned an explicit theme:
 *
 * - `@media (prefers-color-scheme: dark)` → dark palette
 * - `@media (prefers-contrast: more)` → high-contrast palette
 * - `@media (prefers-color-scheme: dark) and (prefers-contrast: more)` → high-contrast-dark
 *
 * Two consequences follow, and they pull in opposite directions:
 *
 * 1. We must always pin an **explicit** `data-theme`, including for light. If light removed
 *    the attribute, a user who deliberately chose light would still be shown dark on a
 *    dark-OS device, because the `:not([data-theme])` fallback would win. So `applyTheme`
 *    never deletes the attribute.
 * 2. Because pinning suppresses all three fallbacks, {@link resolveInitialTheme} has to
 *    reproduce them itself for a user who has expressed no stored choice. Otherwise pinning
 *    would silently discard an OS-level `prefers-contrast: more` request — an accessibility
 *    regression. Its branches mirror the three media blocks above exactly.
 *
 * Note this is a deliberate departure from the `AGENTS.md` shorthand that "light = attribute
 * removed": that rule predates the preference fallbacks in the vendored stylesheet and is
 * unsafe against the current token build.
 *
 * The pure pieces ({@link resolveInitialTheme}, {@link nextTheme}, {@link isTheme},
 * {@link themeLabel}) are unit-tested; the `localStorage`/`matchMedia`/DOM helpers are the
 * thin shell.
 */

/** The themes libro exposes, in the order the toggle cycles through them. */
export const THEMES = [
  'light',
  'dark',
  'dark-oled',
  'high-contrast',
  'high-contrast-dark',
] as const;

/** One of the studio appearance modes. */
export type Theme = (typeof THEMES)[number];

/** The OS-level appearance preferences that seed a first-run theme. */
export interface ThemePreferences {
  /** `prefers-color-scheme: dark` */
  prefersDark: boolean;
  /** `prefers-contrast: more` */
  prefersMoreContrast: boolean;
}

/** `localStorage` key for the user's explicit choice. */
export const THEME_STORAGE_KEY = 'libro:theme';

/** Human-facing labels for each theme (used by the toggle control). */
const THEME_LABELS: Record<Theme, string> = {
  light: 'Light',
  dark: 'Dark',
  'dark-oled': 'Dark OLED',
  'high-contrast': 'High contrast',
  'high-contrast-dark': 'High contrast dark',
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
 * Resolve the theme to apply on first load: an explicit stored choice always wins, otherwise
 * fall back to the OS preferences. The fallback branches mirror the three
 * `:root:not([data-theme])` media blocks in the vendored token stylesheet, because pinning an
 * explicit attribute suppresses them. Pure.
 */
export function resolveInitialTheme(prefs: ThemePreferences, stored: string | null): Theme {
  if (isTheme(stored)) return stored;
  if (prefs.prefersMoreContrast) return prefs.prefersDark ? 'high-contrast-dark' : 'high-contrast';
  return prefs.prefersDark ? 'dark' : 'light';
}

/** The next theme in the cycle — used by the toggle. Wraps around. Pure. */
export function nextTheme(current: Theme): Theme {
  const index = THEMES.indexOf(current);
  // The modulo keeps this in range; the fallback only satisfies noUncheckedIndexedAccess.
  return THEMES[(index + 1) % THEMES.length] ?? THEMES[0];
}

/** Read a media query, defaulting to `false` off-DOM. */
function matches(query: string): boolean {
  return typeof matchMedia === 'function' && matchMedia(query).matches;
}

/** Read the OS appearance preferences (all default to `false` off-DOM). */
export function readThemePreferences(): ThemePreferences {
  return {
    prefersDark: matches('(prefers-color-scheme: dark)'),
    prefersMoreContrast: matches('(prefers-contrast: more)'),
  };
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
 * the token stylesheet's preference fallbacks can't override an explicit choice.
 */
export function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}

/**
 * Resolve + apply the initial theme before first paint (called from `main.ts`).
 * Returns the resolved theme so the shell can seed its reactive state.
 */
export function applyInitialTheme(): Theme {
  const theme = resolveInitialTheme(readThemePreferences(), readStoredTheme());
  applyTheme(theme);
  return theme;
}
