/**
 * AI Terminal welcome content for each window type.
 * Shows commands and context awareness. Example prompts are now
 * handled by clickable suggested prompt chips (see suggestedPrompts.ts).
 */

type WindowType = 'account' | 'charting' | 'backtesting' | 'ticket' | 'watcher' | 'tradeanalysis';

interface TerminalWelcome {
  header: string;
  description: string;
  content: string[];
}

/** Command descriptions shown in all windows */
const COMMANDS = [
  '/clear  - Clear chat history',
  '/usage  - View AI token usage',
];

/**
 * Build welcome content for a specific window type.
 * @param windowType - The current window type
 * @param _context - Optional dynamic context (reserved for future use)
 */
export function getTerminalWelcome(
  windowType: WindowType,
  _context?: { instrument?: string; strategyName?: string }
): TerminalWelcome {
  const config: Record<WindowType, TerminalWelcome> = {
    charting: {
      header: 'Chart AI Assistant',
      description: 'I can see your chart, indicators, and price data',
      content: [
        '',
        'Commands:',
        ...COMMANDS.map(cmd => `  ${cmd}`),
      ],
    },

    backtesting: {
      header: 'Strategy AI Assistant',
      description: 'I can see your strategy rules and backtest results',
      content: [
        '',
        'Commands:',
        ...COMMANDS.map(cmd => `  ${cmd}`),
      ],
    },

    tradeanalysis: {
      header: 'Trade Analysis AI',
      description: 'I can see your trade history and statistics',
      content: [
        '',
        'Commands:',
        ...COMMANDS.map(cmd => `  ${cmd}`),
      ],
    },

    account: {
      header: 'Account AI Assistant',
      description: 'I can see your account balance and open positions',
      content: [
        '',
        'Commands:',
        ...COMMANDS.map(cmd => `  ${cmd}`),
      ],
    },

    ticket: {
      header: 'Order AI Assistant',
      description: 'I can see your order details',
      content: [
        '',
        'Commands:',
        ...COMMANDS.map(cmd => `  ${cmd}`),
      ],
    },

    watcher: {
      header: 'Strategy Watcher AI',
      description: 'I can see your running strategies and signals',
      content: [
        '',
        'Commands:',
        ...COMMANDS.map(cmd => `  ${cmd}`),
      ],
    },
  };

  return config[windowType];
}
