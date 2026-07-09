/**
 * Mock trade data for E2E tests
 * Matches the tradeTable schema in shared/schema.ts
 */

export const mockTrades = [
  {
    id: '1001',
    user_id: 'e2e-user-001',
    account_id: '101-001-1234567-001',
    instrument: 'EUR_USD',
    units: '10000',
    open_price: '1.08500',
    close_price: '1.08750',
    open_time: 1733047200000, // 2025-12-01T10:00:00Z
    close_time: 1733061600000,
    realized_pl: '25.00',
    state: 'CLOSED',
    synced_at: Date.now(),
    created_at: Date.now(),
    updated_at: Date.now(),
  },
  {
    id: '1002',
    user_id: 'e2e-user-001',
    account_id: '101-001-1234567-001',
    instrument: 'GBP_USD',
    units: '-5000',
    open_price: '1.27500',
    close_price: '1.27200',
    open_time: 1733126400000, // 2025-12-02T08:00:00Z
    close_time: 1733155200000,
    realized_pl: '15.00',
    state: 'CLOSED',
    synced_at: Date.now(),
    created_at: Date.now(),
    updated_at: Date.now(),
  },
  {
    id: '1003',
    user_id: 'e2e-user-001',
    account_id: '101-001-1234567-001',
    instrument: 'USD_JPY',
    units: '10000',
    open_price: '150.500',
    close_price: '150.200',
    open_time: 1733216400000, // 2025-12-03T09:00:00Z
    close_time: 1733238000000,
    realized_pl: '-20.00',
    state: 'CLOSED',
    synced_at: Date.now(),
    created_at: Date.now(),
    updated_at: Date.now(),
  },
];

/**
 * Mock historical trades as returned by get_trade_history (Rust format)
 */
export const mockHistoricalTrades = [
  {
    id: '1001',
    instrument: 'EUR_USD',
    units: '10000',
    openPrice: '1.08500',
    closePrice: '1.08750',
    openTime: '2025-12-01T10:00:00Z',
    closeTime: '2025-12-01T14:00:00Z',
    realizedPl: '25.00',
    isWinner: true,
  },
  {
    id: '1002',
    instrument: 'GBP_USD',
    units: '-5000',
    openPrice: '1.27500',
    closePrice: '1.27200',
    openTime: '2025-12-02T08:00:00Z',
    closeTime: '2025-12-02T16:00:00Z',
    realizedPl: '15.00',
    isWinner: true,
  },
  {
    id: '1003',
    instrument: 'USD_JPY',
    units: '10000',
    openPrice: '150.500',
    closePrice: '150.200',
    openTime: '2025-12-03T09:00:00Z',
    closeTime: '2025-12-03T15:00:00Z',
    realizedPl: '-20.00',
    isWinner: false,
  },
];
