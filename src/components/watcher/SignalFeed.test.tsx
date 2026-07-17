import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueueAlertRow } from './SignalFeed';
import type { QueuedAlert } from '../../hooks/useWatchDaemon';

const strategySignal = (overrides: Record<string, unknown> = {}): QueuedAlert => ({
  id: 'q-1',
  ts: '2026-07-17T00:00:00Z',
  payload: {
    kind: 'strategy-signal',
    instrument: 'EUR_USD',
    signal: 'buy',
    proposal: {
      id: 'match-1',
      ts: '2026-07-17T00:00:00Z',
      instrument: 'EUR_USD',
      side: 'long',
      units: 1000,
      strategy: 'rahagod',
      reason: 'ladder conditions met',
      status: 'pending',
    },
    ...overrides,
  } as QueuedAlert['payload'],
});

describe('QueueAlertRow (issue #8: account + granularity)', () => {
  it('shows granularity and account when the payload carries them', () => {
    render(
      <QueueAlertRow
        alert={strategySignal({ account: 'tf-m5', granularity: 'M5' })}
      />
    );

    expect(screen.getByTestId('signal-granularity')).toHaveTextContent('M5');
    expect(screen.getByTestId('signal-account')).toHaveTextContent('tf-m5');
  });

  it('omits both chips on legacy rows without the fields', () => {
    render(<QueueAlertRow alert={strategySignal()} />);

    expect(screen.queryByTestId('signal-granularity')).toBeNull();
    expect(screen.queryByTestId('signal-account')).toBeNull();
    // The rest of the row still renders
    expect(screen.getByText('EUR_USD')).toBeInTheDocument();
    expect(screen.getByText('rahagod')).toBeInTheDocument();
  });
});
