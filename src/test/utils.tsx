import { render, RenderOptions } from '@testing-library/react';
import { ReactElement, ReactNode } from 'react';

/**
 * Minimal context wrapper for tests.
 * Add providers here as needed (e.g., theme, routing).
 */
function TestProviders({ children }: { children: ReactNode }) {
  return <>{children}</>;
}

/**
 * Custom render function that wraps components in test providers.
 * Use this instead of RTL's render for consistent test setup.
 */
export function renderWithProviders(
  ui: ReactElement,
  options?: Omit<RenderOptions, 'wrapper'>
) {
  return render(ui, { wrapper: TestProviders, ...options });
}

// Re-export everything from testing-library
export * from '@testing-library/react';
export { default as userEvent } from '@testing-library/user-event';

// Export custom render as default render
export { renderWithProviders as render };
