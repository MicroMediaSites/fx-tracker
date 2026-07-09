import { vi } from 'vitest';

/**
 * Create a mock for Zero's useQuery hook
 */
export function createMockUseQuery<T>(data: T[], isLoading = false) {
  return vi.fn(() => [data, { isLoading }] as const);
}

/**
 * Default empty query result
 */
export const mockEmptyQuery = vi.fn(() => [[], { isLoading: false }] as const);

/**
 * Loading query result
 */
export const mockLoadingQuery = vi.fn(() => [[], { isLoading: true }] as const);

/**
 * Create mock Zero mutate functions for a table
 */
export function createMockMutate() {
  return {
    insert: vi.fn(),
    update: vi.fn(),
    delete: vi.fn(),
  };
}

/**
 * Mock Zero context with common tables
 */
export const mockZero = {
  mutate: {
    strategy: createMockMutate(),
    note: createMockMutate(),
    journal_entry: createMockMutate(),
    playbook: createMockMutate(),
    backtest_result: createMockMutate(),
    economic_event: createMockMutate(),
  },
};
