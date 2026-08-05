import { describe, it, expect } from 'vitest';
import { resolveInitialTheme, nextTheme, isTheme } from './theme';

describe('isTheme', () => {
  it('accepts only the two known themes', () => {
    expect(isTheme('light')).toBe(true);
    expect(isTheme('dark')).toBe(true);
    expect(isTheme('dark-oled')).toBe(false);
    expect(isTheme(null)).toBe(false);
    expect(isTheme('')).toBe(false);
    expect(isTheme(1)).toBe(false);
  });
});

describe('resolveInitialTheme', () => {
  it('prefers an explicit stored choice over the OS preference', () => {
    expect(resolveInitialTheme(true, 'light')).toBe('light');
    expect(resolveInitialTheme(false, 'dark')).toBe('dark');
  });

  it('falls back to the OS preference when nothing is stored', () => {
    expect(resolveInitialTheme(true, null)).toBe('dark');
    expect(resolveInitialTheme(false, null)).toBe('light');
  });

  it('ignores an invalid/garbage stored value and uses the OS preference', () => {
    expect(resolveInitialTheme(true, 'nonsense')).toBe('dark');
    expect(resolveInitialTheme(false, 'sepia')).toBe('light');
  });
});

describe('nextTheme', () => {
  it('toggles between light and dark', () => {
    expect(nextTheme('light')).toBe('dark');
    expect(nextTheme('dark')).toBe('light');
  });
});
