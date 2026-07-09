import { useMemo } from 'react';
import { Strategy, BacktestMethodology } from '../../types/strategy';
import { isStoreStrategy } from '../../lib/strategyStore';
import { MethodologySelector, MethodologyInfo } from '../strategy/MethodologySelector';
import { StrategyVersion } from './types';
import { filterReferencedParameters } from './strategyUtils';
import { TestableParametersPanel, SingleTestingParams, RangeTestingParams, UseDefaultParams } from './TestableParametersPanel';
import { TestZonesPanel, TestZone, strategyUsesCustomZones } from './TestZonesPanel';

interface StrategyListPanelProps {
  strategies: Strategy[];
  selectedStrategy: Strategy | undefined;
  workingCopy: Strategy | null;
  selectedStrategyId: string | null;
  strategyListExpanded: boolean;
  showArchived: boolean;
  methodology: BacktestMethodology | null;
  error: string | null;
  onSelectStrategy: (id: string) => void;
  onToggleExpanded: () => void;
  onShowArchivedChange: (show: boolean) => void;
  onMethodologyChange: (methodology: BacktestMethodology | null) => void;
  onToggleArchive: () => void;
  onResetVersions: (strategy: Strategy) => StrategyVersion[];
  testingValues: SingleTestingParams;
  rangeValues: RangeTestingParams;
  useDefaultParams: UseDefaultParams;
  onTestingValueChange: (paramId: string, value: number) => void;
  onRangeValueChange: (paramId: string, field: 'min' | 'max' | 'step', value: number) => void;
  onUseDefaultChange: (paramId: string, useDefault: boolean) => void;
  onResolveParameter: (paramId: string, value: number) => void;
  onResetTestingValues: () => void;
  onSaveTestingValuesToStrategy: () => void;
  testingValuesModified: boolean;
  rangeValuesModified: boolean;
  testZones: TestZone[];
  onTestZonesChange: (zones: TestZone[]) => void;
}

export const StrategyListPanel = ({
  strategies,
  selectedStrategy,
  workingCopy,
  selectedStrategyId,
  strategyListExpanded,
  showArchived,
  methodology,
  error,
  onSelectStrategy,
  onToggleExpanded,
  onShowArchivedChange,
  onMethodologyChange,
  onToggleArchive,
  onResetVersions,
  testingValues,
  rangeValues,
  useDefaultParams,
  onTestingValueChange,
  onRangeValueChange,
  onUseDefaultChange,
  onResolveParameter,
  onResetTestingValues,
  onSaveTestingValuesToStrategy,
  testingValuesModified,
  rangeValuesModified,
  testZones,
  onTestZonesChange,
}: StrategyListPanelProps) => {
  const isWalkForward = methodology === 'walk_forward';
  // Only show parameters that are actually referenced via $param in the strategy.
  // This filters out orphaned parameter definitions that were unlinked but not deleted.
  const activeStrategy = workingCopy || selectedStrategy;
  const currentParameters = useMemo(
    () => activeStrategy ? filterReferencedParameters(activeStrategy) : [],
    [activeStrategy]
  );
  const hasParameters = currentParameters.length > 0;

  return (
    <div className="space-y-6">
      {/* Strategy Selection Section */}
      <section>
        <div className="flex justify-between items-center mb-4">
          <h2 className="text-lg font-semibold text-[var(--color-text-primary)]">Strategies</h2>
        </div>

        {/* Show Archived checkbox */}
        <label className="flex items-center gap-2 mb-3 text-sm text-[var(--color-text-muted)] cursor-pointer">
          <input
            type="checkbox"
            checked={showArchived}
            onChange={(e) => onShowArchivedChange(e.target.checked)}
            className="rounded border-[var(--color-border)] bg-[var(--color-bg-hover)] text-[var(--color-info)] focus:ring-[var(--color-info)] focus:ring-offset-0"
          />
          Show Archived
        </label>

        {strategies.length === 0 ? (
          <p className="text-sm text-[var(--color-text-muted)]">
            {showArchived
              ? 'No strategies yet. Add one with the wickd CLI: wickd strategy add <script.rhai>.'
              : 'No active strategies. Check "Show Archived" to see archived strategies.'}
          </p>
        ) : (
          <div className="relative">
            {/* Combined selector + details card */}
            <button
              onClick={onToggleExpanded}
              className="w-full text-left pl-3 pr-2 py-2 border-l-2 border-[var(--color-info)] hover:bg-[var(--color-bg-hover)]/50 transition-colors rounded-r"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="flex-1 min-w-0">
                  {/* Strategy name + badges */}
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className={`font-medium truncate ${selectedStrategy ? 'text-[var(--color-text-primary)]' : 'text-[var(--color-text-muted)]'}`}>
                      {selectedStrategy?.name || 'Select a strategy...'}
                    </span>
                    {selectedStrategy && isStoreStrategy(selectedStrategy.id) && (
                      <span
                        className="text-[10px] font-medium px-1.5 py-0.5 bg-[var(--color-info)]/20 text-[var(--color-info)] rounded"
                        title="Read-only .rhai strategy from ~/.wickd/strategies"
                        data-testid="store-badge"
                      >
                        .rhai store
                      </span>
                    )}
                    {selectedStrategy?.strategy_type === 'scripted' && !isStoreStrategy(selectedStrategy.id) && (
                      <span className="text-[10px] font-medium px-1.5 py-0.5 bg-purple-500/20 text-purple-300 rounded">
                        AI
                      </span>
                    )}
                    {selectedStrategy?.is_archived && (
                      <span className="text-xs px-1.5 py-0.5 rounded bg-[var(--color-bg-hover)] text-[var(--color-text-muted)]">
                        Archived
                      </span>
                    )}
                    {selectedStrategy?.is_locked && (
                      <span className={`text-xs px-1.5 py-0.5 rounded flex items-center gap-1 ${
                        selectedStrategy.is_promoted
                          ? 'bg-[var(--color-buy)]/20 text-[var(--color-buy)]'
                          : 'bg-[var(--color-warning)]/20 text-[var(--color-warning)]'
                      }`}>
                        <svg className="h-2.5 w-2.5" fill="currentColor" viewBox="0 0 20 20">
                          <path fillRule="evenodd" d="M5 9V7a5 5 0 0110 0v2a2 2 0 012 2v5a2 2 0 01-2 2H5a2 2 0 01-2-2v-5a2 2 0 012-2zm8-2v2H7V7a3 3 0 016 0z" clipRule="evenodd" />
                        </svg>
                        {selectedStrategy.is_promoted ? 'Live' : 'Locked'}
                      </span>
                    )}
                  </div>
                  {/* Stats */}
                  {selectedStrategy && (
                    <div className="text-xs text-[var(--color-text-muted)] mt-0.5">
                      {selectedStrategy.strategy_type === 'scripted'
                        ? 'Scripted strategy (Rhai)'
                        : `${selectedStrategy.indicators.length} indicators · ${selectedStrategy.entry_rules.length} entry · ${selectedStrategy.exit_rules.length} exit`
                      }
                    </div>
                  )}
                </div>
                <svg
                  className={`h-4 w-4 text-[var(--color-text-muted)] transition-transform flex-shrink-0 mt-1 ${strategyListExpanded ? 'rotate-180' : ''}`}
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                >
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                </svg>
              </div>
            </button>

            {/* Action row - outside the clickable area (list management only;
                editing lives in the wickd CLI since AGT-651) */}
            {selectedStrategy && (
              <div className="flex items-center justify-between mt-1 pl-3">
                <div className="flex items-center gap-2">
                  {!isStoreStrategy(selectedStrategy.id) &&
                    selectedStrategy.is_locked &&
                    !selectedStrategy.is_promoted && (
                    <button
                      onClick={onToggleArchive}
                      className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
                      title={selectedStrategy.is_archived ? 'Unarchive strategy' : 'Archive strategy'}
                    >
                      {selectedStrategy.is_archived ? 'Unarchive' : 'Archive'}
                    </button>
                  )}
                  {selectedStrategy.is_locked && (
                    <span className="text-xs text-[var(--color-warning)]">
                      {selectedStrategy.is_promoted ? 'Locked while live' : 'Locked'}
                    </span>
                  )}
                </div>
              </div>
            )}

            {/* Dropdown menu */}
            {strategyListExpanded && (
              <div className="absolute z-10 w-full mt-1 bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded shadow-lg max-h-64 overflow-y-auto">
                {strategies.map((strategy) => (
                  <button
                    key={strategy.id}
                    onClick={() => {
                      onSelectStrategy(strategy.id);
                      onToggleExpanded();
                    }}
                    className={`w-full text-left px-3 py-2 hover:bg-[var(--color-bg-hover)] transition-colors flex items-center justify-between text-[var(--color-text-primary)] ${
                      selectedStrategyId === strategy.id ? 'bg-[var(--color-info)]/20 text-[var(--color-info)]' : ''
                    }`}
                  >
                    <span className="truncate">{strategy.name}</span>
                    {isStoreStrategy(strategy.id) && (
                      <span
                        className="text-[10px] font-medium px-1.5 py-0.5 bg-[var(--color-info)]/20 text-[var(--color-info)] rounded ml-2 flex-shrink-0"
                        data-testid="store-badge-row"
                      >
                        .rhai store
                      </span>
                    )}
                    {strategy.strategy_type === 'scripted' && !isStoreStrategy(strategy.id) && (
                      <span className="text-[10px] font-medium px-1.5 py-0.5 bg-purple-500/20 text-purple-300 rounded ml-2 flex-shrink-0">
                        AI
                      </span>
                    )}
                    {strategy.is_archived && (
                      <span className="text-xs px-1.5 py-0.5 rounded bg-[var(--color-bg-hover)] text-[var(--color-text-muted)] ml-2 flex-shrink-0">
                        Archived
                      </span>
                    )}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </section>

      {/* Backtest Config Section */}
      {selectedStrategy && (
        <section className="pt-6 border-t border-[var(--color-border)]">
          <h3 className="text-md font-medium text-[var(--color-text-primary)] mb-4">Backtest Config</h3>

          <div className="space-y-4">
            <MethodologySelector
              value={methodology}
              onChange={(newMethodology) => {
                onMethodologyChange(newMethodology);
                // Clear versions when methodology changes
                if (newMethodology !== methodology) {
                  onResetVersions(selectedStrategy);
                }
              }}
            />

            {!methodology && (
              <div className="text-sm text-[var(--color-text-muted)] py-2">
                Select a methodology to begin backtesting.
              </div>
            )}

            {methodology && (
              <MethodologyInfo methodology={methodology} />
            )}

            {methodology && hasParameters && (
              <TestableParametersPanel
                parameters={currentParameters}
                mode={isWalkForward ? 'range' : 'single'}
                singleValues={testingValues}
                rangeValues={rangeValues}
                useDefaultParams={useDefaultParams}
                onSingleChange={onTestingValueChange}
                onRangeChange={onRangeValueChange}
                onUseDefaultChange={onUseDefaultChange}
                onResolveParameter={onResolveParameter}
                onReset={onResetTestingValues}
                onSaveToStrategy={onSaveTestingValuesToStrategy}
                hasChanges={isWalkForward ? rangeValuesModified : testingValuesModified}
              />
            )}

            {/* Test Zones Panel - show when methodology selected */}
            {methodology && workingCopy && (
              <TestZonesPanel
                zones={testZones}
                onChange={onTestZonesChange}
                showWarning={strategyUsesCustomZones(workingCopy)}
              />
            )}
          </div>
        </section>
      )}

      {error && (
        <div className="p-3 bg-[var(--color-sell)]/10 border-l-2 border-[var(--color-sell)] text-[var(--color-sell)] text-sm">
          {error}
        </div>
      )}
    </div>
  );
};
