import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '../../test/utils';
import { BacktestResultsPanel, BacktestResult, TradeData } from './BacktestResultsPanel';
import { Strategy } from '../../types/strategy';

// Mock child components
vi.mock('../charts', () => ({
  EquityCurveChart: ({ data }: { data: unknown[] }) => (
    <div data-testid="equity-curve-chart">Equity Chart ({data.length} points)</div>
  ),
}));

vi.mock('../ui/InfoTooltip', () => ({
  InfoTooltip: ({ text }: { text: string }) => <span title={text}>ℹ️</span>,
}));

vi.mock('./TradeSubsetModal', () => ({
  TradeSubsetModal: () => <div data-testid="trade-subset-modal">Modal</div>,
}));

const mockStrategy: Strategy = {
  id: 'test-strategy-1',
  name: 'Test Strategy',
  user_id: 'user-1',
  description: 'A test strategy for unit tests',
  schema_version: 2,
  indicators: [],
  parameters: [],
  entry_rules: [],
  exit_rules: [],
  risk_settings: {
    risk_method: 'percent',
    risk_value: 1,
    rr_ratio: 2,
    spread_buffer_pips: 1,
  },
  version: 1,
  is_active: true,
  is_promoted: false,
  is_locked: false,
  is_archived: false,
  created_at: Date.now(),
  updated_at: Date.now(),
};

const createMockTrade = (overrides: Partial<TradeData> = {}): TradeData => ({
  tradeNum: 1,
  direction: 'LONG',
  entryTime: '2024-01-01T00:00:00Z',
  exitTime: '2024-01-02T00:00:00Z',
  entryPrice: '1.1000',
  exitPrice: '1.1050',
  units: '1000',
  pnl: '50.00',
  pnlPct: '0.45',
  cumulativePnl: '50.00',
  ...overrides,
});

const createMockResult = (overrides: Partial<BacktestResult> = {}): BacktestResult => ({
  metrics: {
    totalPnl: '1234.56',
    totalReturnPct: '12.35',
    annualizedReturnPct: '24.70',
    winningTrades: 30,
    losingTrades: 20,
    winRate: '60.00',
    avgWin: '82.00',
    avgLoss: '41.00',
    profitFactor: '2.00',
    maxDrawdownPct: '8.50',
    sharpeRatio: '1.85',
    totalTrades: 50,
    finalBalance: '11234.56',
  },
  trades: [
    createMockTrade({ tradeNum: 1, pnl: '100.00', direction: 'LONG' }),
    createMockTrade({ tradeNum: 2, pnl: '-50.00', direction: 'SHORT' }),
    createMockTrade({ tradeNum: 3, pnl: '75.00', direction: 'LONG' }),
  ],
  equityCurve: [
    { time: '2024-01-01', balance: '10000' },
    { time: '2024-01-15', balance: '10500' },
    { time: '2024-01-30', balance: '11234.56' },
  ],
  dataRange: {
    startTime: '2024-01-01T00:00:00Z',
    endTime: '2024-03-31T00:00:00Z',
    totalCandles: 540,
  },
  ...overrides,
});

describe('BacktestResultsPanel', () => {
  const defaultProps = {
    result: null,
    running: false,
    selectedStrategy: null,
    instrument: 'EUR_USD',
    granularity: 'H4',
    initialBalance: 10000,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('empty states', () => {
    it('shows prompt to select strategy when none selected', () => {
      render(<BacktestResultsPanel {...defaultProps} />);

      expect(screen.getByText('Select a strategy from the list to run a backtest.')).toBeInTheDocument();
    });

    it('shows no results message when strategy selected but no results', () => {
      render(<BacktestResultsPanel {...defaultProps} selectedStrategy={mockStrategy} />);

      expect(screen.getByText(/No backtest results for/)).toBeInTheDocument();
      expect(screen.getByText('Test Strategy')).toBeInTheDocument();
    });
  });

  describe('loading state', () => {
    it('shows loading overlay when running', () => {
      render(<BacktestResultsPanel {...defaultProps} running={true} selectedStrategy={mockStrategy} />);

      expect(screen.getByText('Running backtest...')).toBeInTheDocument();
    });
  });

  describe('results display', () => {
    it('displays final balance', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('$11234.56')).toBeInTheDocument();
    });

    it('displays starting balance', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('$10,000')).toBeInTheDocument();
    });

    it('displays total trades count', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('50')).toBeInTheDocument();
      expect(screen.getByText('30W / 20L')).toBeInTheDocument();
    });

    it('displays win rate', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('60.00%')).toBeInTheDocument();
    });

    it('displays profit factor', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('2.00')).toBeInTheDocument();
    });

    it('displays date range', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText(/540 candles/)).toBeInTheDocument();
    });
  });

  describe('P&L formatting', () => {
    it('formats positive P&L with plus sign', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult({ metrics: { ...createMockResult().metrics, totalPnl: '500.00' } })}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('+$500.00')).toBeInTheDocument();
    });

    it('formats negative P&L with minus sign', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult({ metrics: { ...createMockResult().metrics, totalPnl: '-500.00' } })}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('-$500.00')).toBeInTheDocument();
    });
  });

  describe('equity curve', () => {
    it('renders equity curve chart when data exists', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByTestId('equity-curve-chart')).toBeInTheDocument();
      expect(screen.getByText('Equity Chart (3 points)')).toBeInTheDocument();
    });

    it('does not render chart when equity curve is empty', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult({ equityCurve: [] })}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.queryByTestId('equity-curve-chart')).not.toBeInTheDocument();
    });
  });

  describe('trade subset cards', () => {
    it('displays worst trades card', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('Worst Trades')).toBeInTheDocument();
    });

    it('displays best trades card', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('Best Trades')).toBeInTheDocument();
    });

    it('displays longs vs shorts card', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('Longs vs Shorts')).toBeInTheDocument();
    });

    it('opens modal when trade subset card is clicked', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      fireEvent.click(screen.getByText('Worst Trades'));
      expect(screen.getByTestId('trade-subset-modal')).toBeInTheDocument();
    });
  });

  describe('open in chart button', () => {
    it('shows open in chart button when results have trades', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult()}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.getByText('Open in Chart')).toBeInTheDocument();
    });

    it('hides button when no trades', () => {
      render(
        <BacktestResultsPanel
          {...defaultProps}
          result={createMockResult({ trades: [] })}
          selectedStrategy={mockStrategy}
        />
      );

      expect(screen.queryByText('Open in Chart')).not.toBeInTheDocument();
    });
  });
});
