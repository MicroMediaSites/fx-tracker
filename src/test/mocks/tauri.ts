import { vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export const mockInvoke = vi.mocked(invoke);
export const mockListen = vi.mocked(listen);

/**
 * Reset all Tauri mocks to their initial state
 */
export function resetTauriMocks() {
  mockInvoke.mockReset();
  mockListen.mockReset();
  mockListen.mockImplementation(() => Promise.resolve(() => {}));
}

/**
 * Mock a specific Tauri command with a response
 */
export function mockTauriCommand<T>(command: string, response: T) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === command) return Promise.resolve(response);
    return Promise.reject(new Error(`Unmocked command: ${cmd}`));
  });
}

/**
 * Mock multiple Tauri commands
 */
export function mockTauriCommands(commands: Record<string, unknown>) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd in commands) return Promise.resolve(commands[cmd]);
    return Promise.reject(new Error(`Unmocked command: ${cmd}`));
  });
}

/**
 * Mock a Tauri command that should fail
 */
export function mockTauriCommandError(command: string, error: string) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === command) return Promise.reject(new Error(error));
    return Promise.reject(new Error(`Unmocked command: ${cmd}`));
  });
}
