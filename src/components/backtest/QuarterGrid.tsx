/**
 * QuarterGrid - Reusable quarter selection grid.
 *
 * Modes:
 * - 'backtest': Click quarters to run backtests (SimpleHistoricalFlow)
 * - 'holdout-selection': Toggle quarters for holdout selection (WalkForwardFlow)
 */
import { useMemo } from 'react';

export interface QuarterSegment {
  label: string;
  year: number;
  quarter: number;
  startDate: string;
  endDate: string;
  isFuture?: boolean;
  isRetired?: boolean;  // Holdout mode: quarter has aged out of 2-year window
}

interface BacktestRun {
  config: {
    instrument: string;
    granularity: string;
    dateFrom?: string;
    dateTo?: string;
  };
  result: {
    metrics: {
      totalReturnPct: string;
      annualizedReturnPct: string;
    };
  };
}

interface QuarterGridProps {
  yearsToShow: number;
  runs: BacktestRun[];
  instrument: string;
  granularity: string;
  onQuarterClick: (quarter: QuarterSegment) => void;
  onYearClick?: (year: number, startDate: string, endDate: string) => void;
  onAddYears?: () => void;
  disabled?: boolean;
  // For holdout selection mode
  mode?: 'backtest' | 'holdout-selection';
  selectedQuarters?: Set<string>;
}

// Generate quarter key for matching
const getQuarterKey = (quarter: QuarterSegment): string => {
  return `${quarter.year}-Q${quarter.quarter}`;
};

// Generate quarters for the given number of years
const generateQuarters = (yearsToShow: number): QuarterSegment[] => {
  const quarters: QuarterSegment[] = [];
  const now = new Date();
  const currentYear = now.getFullYear();
  const currentMonth = now.getMonth();
  const currentQuarter = Math.floor(currentMonth / 3) + 1;

  const quarterBoundaries = [
    { start: { month: 0, day: 1 }, end: { month: 2, day: 31 } },
    { start: { month: 3, day: 1 }, end: { month: 5, day: 30 } },
    { start: { month: 6, day: 1 }, end: { month: 8, day: 30 } },
    { start: { month: 9, day: 1 }, end: { month: 11, day: 31 } },
  ];

  for (let yearOffset = 0; yearOffset < yearsToShow; yearOffset++) {
    const year = currentYear - yearOffset;
    // Always show all 4 quarters for the current year, future ones will be disabled
    for (let q = 1; q <= 4; q++) {
      const isFuture = year === currentYear && q > currentQuarter;

      const bounds = quarterBoundaries[q - 1];
      const startDate = new Date(year, bounds.start.month, bounds.start.day);
      const endDate = new Date(year, bounds.end.month, bounds.end.day);
      const isCurrentQuarter = year === currentYear && q === currentQuarter;
      const effectiveEndDate = isCurrentQuarter ? now : endDate;

      quarters.push({
        label: `Q${q} '${String(year).slice(2)}`,
        year,
        quarter: q,
        startDate: startDate.toISOString().split('T')[0],
        endDate: isFuture ? endDate.toISOString().split('T')[0] : effectiveEndDate.toISOString().split('T')[0],
        isFuture,
      });
    }
  }

  return quarters;
};

/**
 * Generate holdout quarters with a rolling 2-year window.
 *
 * Shows:
 * - 2 full years back + current year up to current quarter
 * - Retired quarters (dashed outline, not selectable) when new quarters push them out
 *
 * Example for Jan 2026 (Q1):
 * - 2024 Q1-Q4: all active
 * - 2025 Q1-Q4: all active
 * - 2026 Q1: active (current)
 *
 * Example for Apr 2026 (Q2):
 * - 2024 Q1: retired (dashed)
 * - 2024 Q2-Q4: active
 * - 2025 Q1-Q4: active
 * - 2026 Q1-Q2: active
 */
const generateHoldoutQuarters = (): QuarterSegment[] => {
  const quarters: QuarterSegment[] = [];
  const now = new Date();
  const currentYear = now.getFullYear();
  const currentMonth = now.getMonth();
  const currentQuarter = Math.floor(currentMonth / 3) + 1;

  const quarterBoundaries = [
    { start: { month: 0, day: 1 }, end: { month: 2, day: 31 } },
    { start: { month: 3, day: 1 }, end: { month: 5, day: 30 } },
    { start: { month: 6, day: 1 }, end: { month: 8, day: 30 } },
    { start: { month: 9, day: 1 }, end: { month: 11, day: 31 } },
  ];

  // Show 2 years back + current year
  const yearsToShow = [currentYear - 2, currentYear - 1, currentYear];

  for (const year of yearsToShow) {
    for (let q = 1; q <= 4; q++) {
      // For current year, only show up to current quarter
      if (year === currentYear && q > currentQuarter) continue;

      // Determine if this quarter is retired
      // Q1 of 2-years-ago retires when we reach Q2 of current year
      // Q2 of 2-years-ago retires when we reach Q3 of current year, etc.
      const isRetired = year === currentYear - 2 && q < currentQuarter;

      const bounds = quarterBoundaries[q - 1];
      const startDate = new Date(year, bounds.start.month, bounds.start.day);
      const endDate = new Date(year, bounds.end.month, bounds.end.day);

      // For current quarter, use current date as end
      const isCurrentQuarter = year === currentYear && q === currentQuarter;
      const effectiveEndDate = isCurrentQuarter ? now : endDate;

      quarters.push({
        label: `Q${q} '${String(year).slice(2)}`,
        year,
        quarter: q,
        startDate: startDate.toISOString().split('T')[0],
        endDate: effectiveEndDate.toISOString().split('T')[0],
        isFuture: false,
        isRetired,
      });
    }
  }

  return quarters;
};

/**
 * Get the maximum allowed dev end date based on holdout quarters.
 * Dev period must end before the first active (non-retired) holdout quarter starts.
 */
export const getMaxDevEndDate = (): string => {
  const quarters = generateHoldoutQuarters();
  const firstActiveQuarter = quarters.find(q => !q.isRetired);

  if (!firstActiveQuarter) {
    // Fallback: end of previous year
    const now = new Date();
    return `${now.getFullYear() - 1}-12-31`;
  }

  // Max dev end is the day before the first active holdout quarter starts
  const startDate = new Date(firstActiveQuarter.startDate);
  startDate.setDate(startDate.getDate() - 1);
  return startDate.toISOString().split('T')[0];
};

export const QuarterGrid = ({
  yearsToShow,
  runs,
  instrument,
  granularity,
  onQuarterClick,
  onYearClick,
  onAddYears,
  disabled = false,
  mode = 'backtest',
  selectedQuarters,
}: QuarterGridProps) => {
  const isHoldoutMode = mode === 'holdout-selection';

  // Use different quarter generation for holdout mode (rolling 2-year window)
  const quarters = useMemo(
    () => (isHoldoutMode ? generateHoldoutQuarters() : generateQuarters(yearsToShow)),
    [yearsToShow, isHoldoutMode]
  );
  const years = useMemo(
    () => [...new Set(quarters.map((q) => q.year))].sort((a, b) => b - a),  // Newest first
    [quarters]
  );

  // Check if a quarter has a run result
  const getQuarterRun = (quarter: QuarterSegment) => {
    return runs.find(
      (r) =>
        r.config.dateFrom === quarter.startDate &&
        r.config.dateTo === quarter.endDate &&
        r.config.instrument === instrument &&
        r.config.granularity === granularity
    );
  };

  // Check if a year has a full-year run
  const getYearRun = (year: number) => {
    const now = new Date();
    const isCurrentYear = year === now.getFullYear();
    const yearStartDate = `${year}-01-01`;
    const yearEndDate = isCurrentYear ? now.toISOString().split('T')[0] : `${year}-12-31`;

    return runs.find(
      (r) =>
        r.config.dateFrom === yearStartDate &&
        r.config.dateTo === yearEndDate &&
        r.config.instrument === instrument &&
        r.config.granularity === granularity
    );
  };

  return (
    <div className="space-y-1">
      {years.map((year) => {
        const yearRun = getYearRun(year);
        const yearReturnPct = yearRun
          ? parseFloat(yearRun.result.metrics.annualizedReturnPct)
          : null;
        const now = new Date();
        const isCurrentYear = year === now.getFullYear();
        const yearStartDate = `${year}-01-01`;
        const yearEndDate = isCurrentYear ? now.toISOString().split('T')[0] : `${year}-12-31`;

        return (
          <div key={year} className="flex items-center gap-1">
            {/* Year button - only in backtest mode */}
            {!isHoldoutMode && onYearClick && (
              <button
                onClick={() => onYearClick(year, yearStartDate, yearEndDate)}
                disabled={disabled}
                className={`text-xs w-8 px-1 py-0.5 rounded transition-colors disabled:opacity-50 ${
                  yearRun
                    ? yearReturnPct !== null && yearReturnPct >= 0
                      ? 'bg-[var(--color-buy)]/20 text-[var(--color-buy)] hover:bg-[var(--color-buy)]/30'
                      : 'bg-[var(--color-sell)]/20 text-[var(--color-sell)] hover:bg-[var(--color-sell)]/30'
                    : 'text-[var(--color-text-muted)] hover:text-[var(--color-info)] hover:bg-[var(--color-info)]/20'
                }`}
                title={
                  yearRun ? `Full year: ${yearReturnPct?.toFixed(0)}%` : 'Click to test full year'
                }
              >
                '{String(year).slice(2)}
              </button>
            )}
            {/* Year label for holdout mode */}
            {isHoldoutMode && (
              <span className="text-xs w-8 text-[var(--color-text-muted)]">'{String(year).slice(2)}</span>
            )}

            {/* Quarter buttons */}
            <div className="grid grid-cols-4 gap-1 flex-1">
              {quarters
                .filter((q) => q.year === year)
                .map((quarter) => {
                  const run = getQuarterRun(quarter);
                  const annualizedPct = run
                    ? parseFloat(run.result.metrics.annualizedReturnPct)
                    : null;
                  const periodPct = run
                    ? parseFloat(run.result.metrics.totalReturnPct)
                    : null;
                  const quarterKey = getQuarterKey(quarter);
                  const isSelected = selectedQuarters?.has(quarterKey);

                  if (isHoldoutMode) {
                    // Holdout selection mode - toggle selection
                    const isFutureQuarter = quarter.isFuture === true;
                    const isRetired = quarter.isRetired === true;
                    const isDisabled = disabled || isFutureQuarter || isRetired;

                    return (
                      <button
                        key={quarterKey}
                        onClick={() => !isRetired && onQuarterClick(quarter)}
                        disabled={isDisabled}
                        className={`w-full px-2 py-2.5 rounded text-center transition-colors disabled:cursor-not-allowed ${
                          isFutureQuarter
                            ? 'border border-[var(--color-border)]/30 text-[var(--color-text-muted)] opacity-40'
                            : isRetired
                            ? 'border border-dashed border-[var(--color-border)]/50 text-[var(--color-text-muted)] opacity-50'
                            : isSelected
                            ? 'bg-[var(--color-warning)]/15 border border-[var(--color-warning)]/50 hover:bg-[var(--color-warning)]/25'
                            : disabled
                            ? 'border border-[var(--color-border)] text-[var(--color-text-muted)] opacity-50'
                            : 'border border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-hover)]'
                        }`}
                        title={
                          isFutureQuarter
                            ? 'Future quarter - no data yet'
                            : isRetired
                            ? 'Quarter has aged out of holdout window'
                            : undefined
                        }
                      >
                        <div className={`text-xs font-medium ${
                          isFutureQuarter || isRetired
                            ? 'text-[var(--color-text-muted)]'
                            : isSelected
                            ? 'text-[var(--color-text-primary)]'
                            : ''
                        }`}>Q{quarter.quarter}</div>
                        {isRetired && (
                          <div className="text-[10px] text-[var(--color-text-muted)]">Retired</div>
                        )}
                        {isSelected && !isFutureQuarter && !isRetired && (
                          <div className="text-[10px] text-[var(--color-warning)]">Holdout</div>
                        )}
                      </button>
                    );
                  }

                  // Backtest mode - run backtests
                  const isFutureQuarter = quarter.isFuture === true;
                  const tooltip = isFutureQuarter
                    ? 'Future quarter - no data yet'
                    : run && periodPct !== null && annualizedPct !== null
                    ? `Period: ${periodPct >= 0 ? '+' : ''}${periodPct.toFixed(1)}% | Annualized: ${annualizedPct >= 0 ? '+' : ''}${annualizedPct.toFixed(0)}%`
                    : 'Click to run backtest';
                  return (
                    <button
                      key={quarterKey}
                      onClick={() => onQuarterClick(quarter)}
                      disabled={disabled || isFutureQuarter}
                      className={`w-full px-2 py-2.5 rounded text-center transition-colors disabled:cursor-not-allowed ${
                        isFutureQuarter
                          ? 'border border-[var(--color-border)]/30 text-[var(--color-text-muted)] opacity-40'
                          : run
                          ? periodPct !== null && periodPct >= 0
                            ? 'bg-[var(--color-buy)]/10 border border-[var(--color-buy)]/40 hover:bg-[var(--color-buy)]/20'
                            : 'bg-[var(--color-sell)]/10 border border-[var(--color-sell)]/40 hover:bg-[var(--color-sell)]/20'
                          : disabled
                          ? 'border border-[var(--color-border)] text-[var(--color-text-muted)] opacity-50'
                          : 'border border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-hover)]'
                      }`}
                      title={tooltip}
                    >
                      <div className={`text-xs font-medium ${
                        isFutureQuarter
                          ? 'text-[var(--color-text-muted)]'
                          : run
                          ? 'text-[var(--color-text-primary)]'
                          : ''
                      }`}>Q{quarter.quarter}</div>
                      {run && periodPct !== null && !isFutureQuarter && (
                        <div
                          className={`text-[10px] ${periodPct >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}
                        >
                          {periodPct >= 0 ? '+' : ''}
                          {periodPct.toFixed(1)}%
                        </div>
                      )}
                    </button>
                  );
                })}
            </div>
          </div>
        );
      })}

      {/* Add more years button */}
      {onAddYears && (
        <button
          onClick={onAddYears}
          className="w-full mt-2 px-2 py-1.5 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] border border-dashed border-[var(--color-border)] hover:border-[var(--color-text-muted)] rounded transition-colors flex items-center justify-center gap-1"
        >
          <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M12 6v6m0 0v6m0-6h6m-6 0H6"
            />
          </svg>
          Add '{new Date().getFullYear() - yearsToShow} quarters
        </button>
      )}
    </div>
  );
}

// Export helper functions for external use
export { generateQuarters, generateHoldoutQuarters, getQuarterKey };
