import { describe, it, expect, beforeEach } from 'vitest';
import { resolveInitialTheme, nextTheme, isTheme, themeLabel, applyTheme, THEMES } from './theme';
import type { Theme } from './theme';

describe('isTheme', () => {
  it('accepts every studio theme in the cycle', () => {
    expect(isTheme('light')).toBe(true);
    expect(isTheme('dark')).toBe(true);
    expect(isTheme('dark-oled')).toBe(true);
    expect(isTheme('high-contrast')).toBe(true);
    expect(isTheme('high-contrast-dark')).toBe(true);
  });

  it('rejects unknown or non-string values', () => {
    expect(isTheme('sepia')).toBe(false);
    expect(isTheme(null)).toBe(false);
    expect(isTheme('')).toBe(false);
    expect(isTheme(1)).toBe(false);
  });
});

describe('themeLabel', () => {
  it('gives a human label for each theme', () => {
    expect(themeLabel('light')).toBe('Light');
    expect(themeLabel('dark')).toBe('Dark');
    expect(themeLabel('dark-oled')).toBe('Dark OLED');
    expect(themeLabel('high-contrast')).toBe('High contrast');
    expect(themeLabel('high-contrast-dark')).toBe('High contrast dark');
  });
});

describe('resolveInitialTheme', () => {
  const os = (prefersDark: boolean, prefersMoreContrast = false) => ({
    prefersDark,
    prefersMoreContrast,
  });

  it('prefers an explicit stored choice over the OS preference', () => {
    expect(resolveInitialTheme(os(true), 'light')).toBe('light');
    expect(resolveInitialTheme(os(false), 'dark')).toBe('dark');
    expect(resolveInitialTheme(os(false), 'dark-oled')).toBe('dark-oled');
    expect(resolveInitialTheme(os(true), 'high-contrast')).toBe('high-contrast');
    expect(resolveInitialTheme(os(false), 'high-contrast-dark')).toBe('high-contrast-dark');
  });

  it('a stored choice also wins over a contrast preference', () => {
    expect(resolveInitialTheme(os(false, true), 'light')).toBe('light');
    expect(resolveInitialTheme(os(true, true), 'dark')).toBe('dark');
  });

  it('falls back to the OS preference when nothing is stored', () => {
    expect(resolveInitialTheme(os(true), null)).toBe('dark');
    expect(resolveInitialTheme(os(false), null)).toBe('light');
  });

  // Pinning an explicit data-theme suppresses the token stylesheet's
  // :root:not([data-theme]) media fallbacks, so these branches must reproduce them
  // or an OS-level contrast request would be silently dropped.
  it('honors prefers-contrast: more, pairing it with the color scheme', () => {
    expect(resolveInitialTheme(os(false, true), null)).toBe('high-contrast');
    expect(resolveInitialTheme(os(true, true), null)).toBe('high-contrast-dark');
  });

  it('ignores an invalid/garbage stored value and uses the OS preference', () => {
    expect(resolveInitialTheme(os(true), 'nonsense')).toBe('dark');
    expect(resolveInitialTheme(os(false), 'sepia')).toBe('light');
    expect(resolveInitialTheme(os(false, true), 'sepia')).toBe('high-contrast');
  });
});

describe('nextTheme', () => {
  it('cycles through every theme and wraps around', () => {
    expect(nextTheme('light')).toBe('dark');
    expect(nextTheme('dark')).toBe('dark-oled');
    expect(nextTheme('dark-oled')).toBe('high-contrast');
    expect(nextTheme('high-contrast')).toBe('high-contrast-dark');
    expect(nextTheme('high-contrast-dark')).toBe('light');
  });

  it('visits each theme exactly once per full cycle', () => {
    const visited: string[] = [];
    let theme: Theme = THEMES[0];
    for (let i = 0; i < THEMES.length; i++) {
      visited.push(theme);
      theme = nextTheme(theme);
    }
    expect(new Set(visited).size).toBe(THEMES.length);
    expect(theme).toBe(THEMES[0]);
  });
});

describe('applyTheme', () => {
  beforeEach(() => {
    delete document.documentElement.dataset.theme;
  });

  it('sets an explicit data-theme for light (never removes the attribute)', () => {
    applyTheme('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('sets an explicit data-theme for dark', () => {
    applyTheme('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('always leaves an explicit attribute when switching dark -> light', () => {
    applyTheme('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    applyTheme('light');
    // The key fix: light pins data-theme="light" rather than clearing it, so the
    // token stylesheet's prefers-color-scheme fallback can't override the choice.
    expect(document.documentElement.hasAttribute('data-theme')).toBe(true);
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
