import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import {
  EventRow,
  countdown,
  localDateKey,
  type EconomicCalendarEvent,
} from './EconomicCalendarSection';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

const event = (overrides: Partial<EconomicCalendarEvent> = {}): EconomicCalendarEvent => ({
  date: '2026-07-17',
  time: '12:30',
  time_unix: 1784291400,
  currency: 'USD',
  event: 'Core CPI m/m',
  impact: 'high',
  actual: '',
  forecast: '0.3%',
  previous: '0.2%',
  ...overrides,
});

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue([]);
});

describe('EventRow', () => {
  it('renders a readable impact badge, not just a dot', () => {
    render(<EventRow ev={event()} />);
    expect(screen.getByTestId('calendar-impact')).toHaveTextContent('high');
  });

  it('abbreviates medium to med', () => {
    render(<EventRow ev={event({ impact: 'medium' })} />);
    expect(screen.getByTestId('calendar-impact')).toHaveTextContent('med');
  });

  it('expands into a detail panel with labeled fields and UTC time', () => {
    render(<EventRow ev={event()} />);
    expect(screen.queryByTestId('calendar-event-detail')).toBeNull();

    fireEvent.click(screen.getByTestId('calendar-event-toggle'));

    const detail = screen.getByTestId('calendar-event-detail');
    expect(detail).toHaveTextContent('12:30 UTC');
    expect(detail).toHaveTextContent('Forecast 0.3%');
    expect(detail).toHaveTextContent('Previous 0.2%');
    expect(detail).toHaveTextContent('Actual pending');
  });

  it('loads release history for the series on expand', async () => {
    mockInvoke.mockResolvedValue([
      event({ date: '2026-06-10', actual: '0.3%', forecast: '0.3%' }),
      event({ date: '2026-05-13', actual: '0.2%', forecast: '0.3%' }),
    ]);
    render(<EventRow ev={event()} />);
    fireEvent.click(screen.getByTestId('calendar-event-toggle'));

    await waitFor(() => expect(screen.getByTestId('calendar-history')).toBeInTheDocument());
    expect(mockInvoke).toHaveBeenCalledWith('get_economic_event_history', {
      currency: 'USD',
      event: 'Core CPI m/m',
      limit: 8,
    });
    expect(screen.getByTestId('calendar-history')).toHaveTextContent('2026-06-10');
    expect(screen.getByTestId('calendar-history')).toHaveTextContent('2026-05-13');
  });
});

describe('localDateKey', () => {
  it('keys an instant by the LOCAL date, not the UTC date', () => {
    // 2026-07-17T04:00:00Z is still 2026-07-16 anywhere west of UTC-4 —
    // the exact shape of the "Today at 8am (it's 10pm)" grouping bug.
    // Assert against the environment's own timezone math so the test is
    // correct on Matt's machine (Mountain) and in CI (UTC) alike.
    const unix = Date.UTC(2026, 6, 17, 4, 0, 0) / 1000;
    const expected = new Date(unix * 1000);
    const pad = (n: number) => String(n).padStart(2, '0');
    expect(localDateKey(unix)).toBe(
      `${expected.getFullYear()}-${pad(expected.getMonth() + 1)}-${pad(expected.getDate())}`
    );
    if (expected.getTimezoneOffset() > 240) {
      expect(localDateKey(unix)).not.toBe('2026-07-17');
    }
  });
});

describe('countdown', () => {
  it('formats the release distance for a human', () => {
    expect(countdown(30)).toBe('now');
    expect(countdown(5 * 60)).toBe('in 5m');
    expect(countdown(2 * 3600 + 14 * 60)).toBe('in 2h 14m');
    expect(countdown(3 * 86400)).toBe('in 3d');
  });
});
