/**
 * Utility functions and constants for WalkForward components
 */

export const TRAIN_WINDOW_OPTIONS = [
  { value: 3, label: '3 months' },
  { value: 6, label: '6 months' },
  { value: 9, label: '9 months' },
  { value: 12, label: '12 months' },
];

export const TEST_WINDOW_OPTIONS = [
  { value: 1, label: '1 month' },
  { value: 2, label: '2 months' },
  { value: 3, label: '3 months' },
];

// Calculate default walk-forward dates: 7 years back to 6 years back from current year
export const getDefaultWalkForwardDates = (): { from: string; to: string } => {
  const currentYear = new Date().getFullYear();
  const fromYear = currentYear - 7;
  const toYear = currentYear - 6;
  return {
    from: `${fromYear}-01-01`,
    to: `${toYear}-01-01`,
  };
};

export interface EfficiencyBadge {
  text: string;
  bg: string;
  border: string;
  warning: boolean;
}

// Get efficiency badge - check actual P&L for profitability, not Sharpe ratio
// Sharpe measures risk-adjusted returns, not profitability. A strategy can be profitable
// with negative Sharpe if returns are volatile.
export const getEfficiencyBadge = (efficiency: number, oosSharpe: number, oosTotalPnl?: string): EfficiencyBadge => {
  // Use actual P&L to determine profitability, not Sharpe ratio
  const actualPnl = oosTotalPnl ? parseFloat(oosTotalPnl) : null;
  const isUnprofitable = actualPnl !== null ? actualPnl < 0 : oosSharpe < 0;

  if (isUnprofitable) {
    return {
      text: 'Unprofitable',
      bg: 'bg-red-500/20',
      border: 'border-red-500/30',
      warning: true
    };
  }
  // Show efficiency badges based on Sharpe ratio efficiency
  if (efficiency >= 75)
    return { text: 'Excellent', bg: 'bg-green-500/20', border: 'border-green-500/30', warning: false };
  if (efficiency >= 50)
    return { text: 'Good', bg: 'bg-yellow-500/20', border: 'border-yellow-500/30', warning: false };
  if (efficiency >= 25)
    return { text: 'Fair', bg: 'bg-orange-500/20', border: 'border-orange-500/30', warning: false };
  return { text: 'Poor', bg: 'bg-red-500/20', border: 'border-red-500/30', warning: false };
};

export const getEfficiencyColor = (efficiency: number, oosSharpe: number): string => {
  // If OOS Sharpe is negative, always show red regardless of efficiency percentage
  if (oosSharpe < 0) return 'text-red-400';
  if (efficiency >= 75) return 'text-green-400';
  if (efficiency >= 50) return 'text-yellow-400';
  return 'text-red-400';
};

// Calculate expected windows (matches backend's generate_windows logic)
export const calculateExpectedWindows = (
  devDateFrom: string,
  devDateTo: string,
  trainMonths: number,
  testMonths: number
): number => {
  if (!devDateFrom || !devDateTo) return 0;
  const start = new Date(devDateFrom);
  const end = new Date(devDateTo);
  // Total months inclusive (Jan to Jul = 7 months)
  const totalMonths =
    (end.getFullYear() - start.getFullYear()) * 12 + (end.getMonth() - start.getMonth()) + 1;

  // First test starts in month (trainMonths + 1) - right after training ends
  const firstTestMonth = trainMonths + 1;

  // Last test must END within data range. Backend uses exclusive end:
  // test_end = add_months(test_start, testMonths), and test_end must be <= data_end
  // So last valid test_start month = totalMonths - testMonths
  const lastTestMonth = totalMonths - testMonths;

  if (lastTestMonth < firstTestMonth) return 0;

  // Number of windows (each steps by testMonths for rolling walk-forward)
  return Math.floor((lastTestMonth - firstTestMonth) / testMonths) + 1;
};
