import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { EventRow, countdown, type EconomicCalendarEvent } from './EconomicCalendarSection';

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

describe('EventRow', () => {
  it('renders currency, title, forecast and previous', () => {
    render(<EventRow ev={event()} />);
    expect(screen.getByText('USD')).toBeInTheDocument();
    expect(screen.getByText('Core CPI m/m')).toBeInTheDocument();
    expect(screen.getByTitle('Forecast')).toHaveTextContent('0.3%');
    expect(screen.getByTitle('Previous')).toHaveTextContent('0.2%');
  });

  it('shows the actual once released and marks impact on the dot', () => {
    render(<EventRow ev={event({ actual: '0.4%', impact: 'medium' })} />);
    expect(screen.getByTitle('Actual')).toHaveTextContent('0.4%');
    expect(screen.getByTestId('calendar-impact')).toHaveAttribute('title', 'medium impact');
  });

  it('omits empty forecast/previous/actual cells entirely', () => {
    render(<EventRow ev={event({ forecast: '', previous: '', actual: '' })} />);
    expect(screen.queryByTitle('Forecast')).toBeNull();
    expect(screen.queryByTitle('Previous')).toBeNull();
    expect(screen.queryByTitle('Actual')).toBeNull();
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
