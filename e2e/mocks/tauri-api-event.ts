/**
 * E2E mock for @tauri-apps/api/event
 *
 * Stores registered listeners so tests can fire events via:
 *   window.__E2E_EMIT_EVENT__('event-name', payload)
 */

type UnlistenFn = () => void;
type EventHandler = (event: { payload: unknown }) => void;

declare global {
  interface Window {
    __E2E_EVENT_LISTENERS__?: Map<string, Set<EventHandler>>;
    __E2E_EMIT_EVENT__?: (event: string, payload?: unknown) => void;
  }
}

// Initialize global listener storage
if (typeof window !== 'undefined') {
  if (!window.__E2E_EVENT_LISTENERS__) {
    window.__E2E_EVENT_LISTENERS__ = new Map();
  }
  if (!window.__E2E_EMIT_EVENT__) {
    window.__E2E_EMIT_EVENT__ = (event: string, payload?: unknown) => {
      const listeners = window.__E2E_EVENT_LISTENERS__?.get(event);
      if (listeners) {
        listeners.forEach(handler => handler({ payload }));
      }
    };
  }
}

export async function listen<T>(_event: string, _handler: (event: { payload: T }) => void): Promise<UnlistenFn> {
  const listeners = window.__E2E_EVENT_LISTENERS__;
  if (listeners) {
    if (!listeners.has(_event)) {
      listeners.set(_event, new Set());
    }
    listeners.get(_event)!.add(_handler as EventHandler);
  }

  return () => {
    const set = listeners?.get(_event);
    if (set) {
      set.delete(_handler as EventHandler);
    }
  };
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
export async function emit(_event: string, _payload?: unknown): Promise<void> {
  // Also fire to local listeners for consistency
  window.__E2E_EMIT_EVENT__?.(_event, _payload);
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
export async function once<T>(_event: string, _handler: (event: { payload: T }) => void): Promise<UnlistenFn> {
  const wrappedHandler = ((event: { payload: unknown }) => {
    (_handler as EventHandler)(event);
    // Remove after first call
    const set = window.__E2E_EVENT_LISTENERS__?.get(_event);
    if (set) {
      set.delete(wrappedHandler);
    }
  }) as EventHandler;

  const listeners = window.__E2E_EVENT_LISTENERS__;
  if (listeners) {
    if (!listeners.has(_event)) {
      listeners.set(_event, new Set());
    }
    listeners.get(_event)!.add(wrappedHandler);
  }

  return () => {
    const set = listeners?.get(_event);
    if (set) {
      set.delete(wrappedHandler);
    }
  };
}
