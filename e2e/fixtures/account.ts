/**
 * Mock OANDA account data for E2E tests
 */

export const mockAccount = {
  id: '101-001-1234567-001',
  currency: 'USD',
  balance: '10000.00',
  nav: '10250.50',
  unrealized_pl: '250.50',
  open_trade_count: 2,
};

export const mockCredentials = {
  apiKeyPreview: 'abc...xyz',
  accountId: '101-001-1234567-001',
  accountAlias: 'E2E Practice',
  environment: 'practice',
  isConfigured: true,
};

export const mockPositions = [
  {
    instrument: 'EUR_USD',
    long: {
      units: '10000',
      averagePrice: '1.08500',
      unrealizedPL: '150.00',
    },
    short: {
      units: '0',
      averagePrice: '0',
      unrealizedPL: '0',
    },
  },
];

export const mockOrders = [
  {
    id: '2001',
    type: 'LIMIT',
    instrument: 'GBP_USD',
    units: '5000',
    price: '1.26000',
    timeInForce: 'GTC',
    createTime: '2025-12-05T10:00:00Z',
  },
];
