/**
 * AccountHistoryModal — the drill-down header (AGT-1133/D8).
 *
 * The header must repeat the section's active window label so a reader never
 * has to guess which span the trades below belong to (`windowLabel`, not a
 * re-implemented copy of its strings).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { AccountHistoryModal } from './AccountHistoryModal';
import type { AccountHistory } from '../../hooks/useAccountHistory';
import { localDateRangeToInstants, type GlanceWindow } from '../../hooks/useAccountsGlance';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

const HISTORY: AccountHistory = {
  account: 'tf-m1',
  account_id: '101-001-26151603-002',
  environment: 'practice',
  baseline: null,
  since: null,
  count: 0,
  realized: '0',
  blended_exits: 0,
  truncated: false,
  trades: [],
};

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue(HISTORY);
});

afterEach(() => {
  vi.clearAllMocks();
});

// Built via localDateRangeToInstants from local calendar dates (not hardcoded
// UTC instants) so the expected label holds regardless of the machine's
// timezone — same approach useAccountsGlance's own windowLabel test uses.
const rangeInstants = localDateRangeToInstants(
  new Date(2026, 7, 1, 0, 0, 0),
  new Date(2026, 7, 24, 0, 0, 0)
);

const cases: { window: GlanceWindow; label: string }[] = [
  { window: { kind: 'baseline' }, label: 'Since baseline' },
  { window: { kind: 'today' }, label: 'Today' },
  { window: { kind: 'days', days: 7 }, label: 'Last 7d' },
  { window: { kind: 'days', days: 30 }, label: 'Last 30d' },
  { window: { kind: 'range', ...rangeInstants }, label: 'Aug 1 – Aug 24' },
];

describe('AccountHistoryModal — header repeats the active window label', () => {
  for (const { window, label } of cases) {
    it(`shows "${label}" for ${window.kind}`, async () => {
      render(
        <AccountHistoryModal account="tf-m1" glanceWindow={window} onClose={() => {}} />
      );
      await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
      expect(screen.getByTestId('history-window-label')).toHaveTextContent(label);
    });
  }

  it('renders nothing when the account is null (modal closed)', () => {
    render(
      <AccountHistoryModal account={null} glanceWindow={{ kind: 'today' }} onClose={() => {}} />
    );
    expect(screen.queryByTestId('account-history-modal')).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
