import type { TourStep } from '../components/onboarding/FirstRunTour';

export const accountTourSteps: TourStep[] = [
  {
    target: '[data-tour="account-summary"]',
    title: 'Account Summary',
    description: 'View your balance, P&L, and key account metrics at a glance.',
    position: 'bottom',
  },
  {
    target: '[data-tour="positions"]',
    title: 'Open Positions',
    description: 'Track your current trades with real-time profit/loss updates.',
    position: 'left',
  },
  {
    target: '[data-tour="history"]',
    title: 'Trade History',
    description: 'Review past trades and analyze your performance over time.',
    position: 'top',
  },
  {
    target: '[data-tour="profile-menu"]',
    title: 'More Windows',
    description: 'Access Charts, Backtesting, Live Monitor, and more from this menu.',
    position: 'bottom',
  },
  {
    target: '[data-tour="ai-overlay"]',
    title: 'Market Feed',
    description: 'Pull down for the AI market-awareness feed — what to care about right now, refreshed every 15 minutes.',
    position: 'top',
  },
];

export const chartTourSteps: TourStep[] = [
  {
    target: '[data-tour="chart-instrument"]',
    title: 'Instrument & Timeframe',
    description: 'Select currency pairs and timeframes to view price action.',
    position: 'bottom',
  },
  {
    target: '[data-tour="chart-canvas"]',
    title: 'Price Chart',
    description: 'Interactive candlestick chart with zoom, scroll, and crosshair.',
    position: 'top',
  },
  {
    target: '[data-tour="chart-indicators"]',
    title: 'Indicators',
    description: 'Add technical indicators like RSI, MACD, and moving averages.',
    position: 'bottom',
  },
  {
    target: '[data-tour="ai-overlay"]',
    title: 'Market Feed',
    description: 'Pull down for the AI market-awareness feed — what to care about right now, refreshed every 15 minutes.',
    position: 'top',
  },
];

export const backtestTourSteps: TourStep[] = [
  {
    target: '[data-tour="strategy-list"]',
    title: 'Strategy List',
    description: 'Create, select, and manage your trading strategies here.',
    position: 'right',
  },
  {
    target: '[data-tour="backtest-results"]',
    title: 'Results Panel',
    description: 'Run backtests and review performance metrics, equity curves, and trade logs.',
    position: 'left',
  },
  {
    target: '[data-tour="ai-overlay"]',
    title: 'Market Feed',
    description: 'Pull down for the AI market-awareness feed — what to care about right now, refreshed every 15 minutes.',
    position: 'top',
  },
];

export const watcherTourSteps: TourStep[] = [
  {
    target: '[data-tour="watcher-monitors"]',
    title: 'Monitors',
    description: 'Add strategies and instruments to watch for live trading signals.',
    position: 'bottom',
  },
  {
    target: '[data-tour="watcher-matches"]',
    title: 'Pattern Matches',
    description: 'View and execute detected signals when your strategy conditions are met.',
    position: 'top',
  },
  {
    target: '[data-tour="ai-overlay"]',
    title: 'Market Feed',
    description: 'Pull down for the AI market-awareness feed — what to care about right now, refreshed every 15 minutes.',
    position: 'top',
  },
];

export const tradeAnalysisTourSteps: TourStep[] = [
  {
    target: '[data-tour="analysis-filters"]',
    title: 'Filters',
    description: 'Filter trades by date, instrument, and direction to focus your analysis.',
    position: 'bottom',
  },
  {
    target: '[data-tour="analysis-stats"]',
    title: 'Performance Stats',
    description: 'See win rate, profit factor, expectancy, and other key metrics.',
    position: 'bottom',
  },
  {
    target: '[data-tour="ai-overlay"]',
    title: 'Market Feed',
    description: 'Pull down for the AI market-awareness feed — what to care about right now, refreshed every 15 minutes.',
    position: 'top',
  },
];
