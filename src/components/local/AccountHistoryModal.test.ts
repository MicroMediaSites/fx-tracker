/**
 * formatDuration — trade hold time for the drill-down rows.
 */
import { describe, expect, it } from 'vitest';
import { formatDuration } from './AccountHistoryModal';

describe('formatDuration', () => {
  it('renders sub-hour holds in minutes', () => {
    expect(formatDuration(3 * 60)).toBe('3m');
    expect(formatDuration(59 * 60)).toBe('59m');
  });

  it('renders hours and minutes', () => {
    expect(formatDuration(2 * 3600 + 14 * 60)).toBe('2h 14m');
  });

  it('omits the minutes on a whole hour', () => {
    expect(formatDuration(3 * 3600)).toBe('3h');
  });

  it('renders multi-day holds', () => {
    expect(formatDuration(27 * 3600)).toBe('1d 3h');
    expect(formatDuration(48 * 3600)).toBe('2d');
  });

  it('shows a dash for missing or negative durations', () => {
    // An open trade or a clock-skew artifact — never render a bogus number.
    expect(formatDuration(null)).toBe('—');
    expect(formatDuration(-10)).toBe('—');
  });

  it('renders a sub-minute hold as 0m rather than a dash', () => {
    expect(formatDuration(30)).toBe('0m');
  });
});
