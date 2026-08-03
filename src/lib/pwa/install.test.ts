import { describe, it, expect } from 'vitest';
import { shouldShowInstall } from './install';

describe('shouldShowInstall', () => {
  it('shows only when a prompt is available and not already installed/standalone', () => {
    expect(shouldShowInstall({ promptAvailable: true, installed: false, standalone: false })).toBe(
      true,
    );
  });

  it('hides when no install prompt has been captured', () => {
    expect(shouldShowInstall({ promptAvailable: false, installed: false, standalone: false })).toBe(
      false,
    );
  });

  it('hides once the app has been installed this session', () => {
    expect(shouldShowInstall({ promptAvailable: true, installed: true, standalone: false })).toBe(
      false,
    );
  });

  it('hides when already running standalone', () => {
    expect(shouldShowInstall({ promptAvailable: true, installed: false, standalone: true })).toBe(
      false,
    );
  });
});
