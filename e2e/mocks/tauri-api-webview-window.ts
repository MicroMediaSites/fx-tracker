/**
 * E2E mock for @tauri-apps/api/webviewWindow
 */

const mockWebviewWindow = {
  label: 'main',
  close: async () => {},
  show: async () => {},
  hide: async () => {},
  setTitle: async () => {},
  listen: async () => () => {},
  emit: async () => {},
  once: async () => () => {},
};

export function getCurrentWebviewWindow() {
  return mockWebviewWindow;
}

export class WebviewWindow {
  label: string;
  constructor(label: string) {
    this.label = label;
  }
  async close() {}
  async show() {}
  async hide() {}
  async setTitle() {}
  async listen() { return () => {}; }
  async emit() {}
  async once() { return () => {}; }
}
