import '@testing-library/jest-dom/vitest';
import { vi } from 'vitest';

// Mock Tauri APIs globally
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
  emit: vi.fn(),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn(() => ({
    setTitle: vi.fn(),
    close: vi.fn(),
    show: vi.fn(),
    hide: vi.fn(),
  })),
}));
