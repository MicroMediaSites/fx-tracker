/**
 * accountSuffix — the last-3-digits sub-account label on each tile.
 */
import { describe, expect, it } from 'vitest';
import { accountSuffix } from './AccountsSection';

describe('accountSuffix', () => {
  it('takes the last three digits of the final hyphen group', () => {
    expect(accountSuffix('101-001-26151603-005')).toBe('005');
    expect(accountSuffix('101-001-26151603-001')).toBe('001');
  });

  it('returns null for a missing account id', () => {
    expect(accountSuffix(null)).toBeNull();
  });

  it('falls back to the tail of an unexpected shape rather than guessing', () => {
    expect(accountSuffix('abc')).toBe('abc');
    expect(accountSuffix('12345')).toBe('345');
  });
});
