/**
 * E2E mock for @tauri-apps/api/window
 */

const mockWindow = {
  label: 'main',
  setTitle: async () => {},
  close: async () => {},
  show: async () => {},
  hide: async () => {},
  minimize: async () => {},
  maximize: async () => {},
  unmaximize: async () => {},
  setFocus: async () => {},
  isVisible: async () => true,
  isMaximized: async () => false,
  isMinimized: async () => false,
  setSize: async () => {},
  setPosition: async () => {},
  center: async () => {},
  onCloseRequested: async () => () => {},
};

export function getCurrentWindow() {
  return mockWindow;
}

export const appWindow = mockWindow;
