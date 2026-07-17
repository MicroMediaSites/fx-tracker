import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/plugin-updater', () => ({
  check: vi.fn(),
  Update: class {},
}));
vi.mock('@tauri-apps/plugin-process', () => ({
  relaunch: vi.fn(),
}));

// Vite-injected globals the hook reads at module scope
(globalThis as Record<string, unknown>).__APP_VERSION__ = '0.0.0-test';
(globalThis as Record<string, unknown>).__BUILD_MODE__ = 'production';

import { useAppUpdater } from './useAppUpdater';
import { check } from '@tauri-apps/plugin-updater';

const mockInvoke = vi.mocked(invoke);
const mockCheck = vi.mocked(check);

describe('useAppUpdater — local builds (placeholder endpoint)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The hook early-returns in dev mode; these tests exercise the
    // production paths.
    vi.stubEnv('DEV', false);
  });

  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it('reports local-build instead of erroring, and never calls check()', async () => {
    mockInvoke.mockResolvedValue(true); // updater_is_placeholder → true

    const { result } = renderHook(() => useAppUpdater());
    let outcome: boolean | undefined;
    await act(async () => {
      outcome = await result.current.checkForUpdates();
    });

    expect(outcome).toBe(false);
    expect(result.current.status).toBe('local-build');
    expect(result.current.error).toBeNull();
    expect(mockCheck).not.toHaveBeenCalled();
  });

  it('release builds (real endpoint) proceed to check()', async () => {
    mockInvoke.mockResolvedValue(false);
    mockCheck.mockResolvedValue(null); // up to date

    const { result } = renderHook(() => useAppUpdater());
    await act(async () => {
      await result.current.checkForUpdates();
    });

    expect(mockCheck).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe('up-to-date');
  });

  it('fails open if the probe command is unavailable', async () => {
    mockInvoke.mockRejectedValue(new Error('unknown command'));
    mockCheck.mockResolvedValue(null);

    const { result } = renderHook(() => useAppUpdater());
    await act(async () => {
      await result.current.checkForUpdates();
    });

    // Probe failure must not block the real check
    expect(mockCheck).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe('up-to-date');
  });
});
