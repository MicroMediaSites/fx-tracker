import { Strategy } from '../../types/strategy';

export interface BacktestMetrics {
  totalPnl: string;
  totalReturnPct: string;
  annualizedReturnPct: string;
  winningTrades: number;
  losingTrades: number;
  winRate: string;
  avgWin: string;
  avgLoss: string;
  profitFactor: string;
  maxDrawdownPct: string;
  sharpeRatio: string;
  totalTrades: number;
  finalBalance: string;
}

export interface TradeData {
  tradeNum: number;
  direction: string;
  entryTime: string;
  exitTime: string;
  entryPrice: string;
  exitPrice: string;
  units: string;
  pnl: string;
  pnlPct: string;
  cumulativePnl: string;
}

export interface EquityPoint {
  time: string;
  balance: string;
}

export interface DataRange {
  startTime: string;
  endTime: string;
  totalCandles: number;
}

export interface BacktestResultData {
  metrics: BacktestMetrics;
  trades: TradeData[];
  equityCurve: EquityPoint[];
  dataRange: DataRange;
}

export type View = 'list' | 'builder' | 'backtest' | 'version-editor';

export interface BacktestRun {
  config: {
    instrument: string;
    granularity: string;
    candleCount?: number;
    dateFrom?: string;
    dateTo?: string;
  };
  result: BacktestResultData;
  timestamp: number;
}

export interface StrategyVersion {
  id: string; // 'original', 'v1', 'v2', etc.
  label: string;
  strategy: Strategy;
  runs: BacktestRun[];
}

export interface PromotionAcknowledgements {
  ownLogic: boolean;
  independentChoices: boolean;
  noGuarantee: boolean;
  responsible: boolean;
}

export interface DynamicParameter {
  id: string;
  name: string;
  default: number;
  type: 'number' | 'integer' | 'select' | 'boolean';
}
