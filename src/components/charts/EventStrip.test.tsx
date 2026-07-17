import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { EventStrip } from './EventStrip';
import type { EconomicCalendarEvent } from '../local/EconomicCalendarSection';

const event = (overrides: Partial<EconomicCalendarEvent> = {}): EconomicCalendarEvent => ({
  date: '2026-07-17',
  time: '12:30',
  time_unix: Math.floor(Date.now() / 1000) + 2 * 3600,
  currency: 'USD',
  event: 'Core CPI m/m',
  impact: 'high',
  actual: '',
  forecast: '0.3%',
  previous: '0.2%',
  ...overrides,
});

describe('EventStrip', () => {
  it('shows the next upcoming event with a countdown', () => {
    const past = event({ time_unix: Math.floor(Date.now() / 1000) - 3600, event: 'Old News' });
    const next = event();
    render(<EventStrip events={[past, next]} />);
    const strip = screen.getByTestId('chart-event-strip');
    expect(strip).toHaveTextContent('USD Core CPI m/m');
    expect(strip).toHaveTextContent(/in \d+h/);
    expect(strip.textContent).not.toContain('Old News');
  });

  it('renders nothing when every event is in the past', () => {
    const past = event({ time_unix: Math.floor(Date.now() / 1000) - 3600 });
    render(<EventStrip events={[past]} />);
    expect(screen.queryByTestId('chart-event-strip')).toBeNull();
  });
});
