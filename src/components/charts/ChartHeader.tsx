import { SymbolPicker } from '../ui/SymbolPicker';
import { Combobox } from '../ui/Combobox';
import { LivePriceDisplay } from './LivePriceDisplay';
import { CandleCountdown } from './CandleCountdown';
import { IndicatorMenu } from './IndicatorMenu';
import { SRToolsMenu } from './SRToolsMenu';
import { useSettingsStore } from '../../stores/settingsStore';
import { GRANULARITIES } from '../../constants';
import type { PriceUpdate, OHLCData, ChartIndicatorConfig } from './chartTypes';
import type { IndicatorType } from '../../types/strategy';

interface ChartHeaderProps {
  // Instrument/granularity state
  instrument: string;
  granularity: string;
  loading: boolean;
  onInstrumentChange: (instrument: string) => void;
  onGranularityChange: (granularity: string) => void;

  // Price display
  isHistoricalView: boolean;
  hoveredCandle: OHLCData | null;
  streaming: boolean;
  currentPrice: PriceUpdate | null;

  // S/R Tools (only shown on main chart)
  isMainChart: boolean;
  srEditingMode: boolean;
  pendingZoneBoundary: number | null;
  secondBoundary: number | null;
  srMenuOpen: boolean;
  zoneCount: number;
  importingPivots: boolean;
  confirmClearAll: boolean;
  onSaveZone: () => void;
  onCancelZoneEdit: () => void;
  onSrMenuToggle: () => void;
  onSrMenuClose: () => void;
  onDrawZone: () => void;
  onImportPivots: (timeframe: 'daily' | 'weekly') => void;
  onClearAllZones: () => void;
  onConfirmClearAll: () => void;
  onCancelClearAll: () => void;

  // Indicator menu
  indicatorMenuOpen: boolean;
  indicators: ChartIndicatorConfig[];
  onIndicatorMenuToggle: () => void;
  onIndicatorMenuClose: () => void;
  onAddIndicator: (type: IndicatorType, params: Record<string, number>, colors?: Record<string, string>) => void;
  onUpdateIndicator: (id: string, params: Record<string, number>, colors?: Record<string, string>) => void;
  onRemoveIndicator: (id: string) => void;

  // Signal context (badge only — execution moved to the wickd CLI, AGT-652)
  signalDirection: 'long' | 'short' | null;
  strategyId: string | null;

  // Recenter the chart view (re-enable price auto-scale, scroll to latest)
  onRecenter: () => void;
}

export const ChartHeader = ({
  instrument,
  granularity,
  loading,
  onInstrumentChange,
  onGranularityChange,
  isHistoricalView,
  hoveredCandle,
  streaming,
  currentPrice,
  isMainChart,
  srEditingMode,
  pendingZoneBoundary,
  secondBoundary,
  srMenuOpen,
  zoneCount,
  importingPivots,
  confirmClearAll,
  onSaveZone,
  onCancelZoneEdit,
  onSrMenuToggle,
  onSrMenuClose,
  onDrawZone,
  onImportPivots,
  onClearAllZones,
  onConfirmClearAll,
  onCancelClearAll,
  indicatorMenuOpen,
  indicators,
  onIndicatorMenuToggle,
  onIndicatorMenuClose,
  onAddIndicator,
  onUpdateIndicator,
  onRemoveIndicator,
  signalDirection,
  strategyId,
  onRecenter,
}: ChartHeaderProps) => {
  const { mySymbols } = useSettingsStore();

  // Convert GRANULARITIES to ComboboxOption format
  const granularityOptions = GRANULARITIES.map(g => ({ value: g.value, label: g.label }));

  return (
    <div className="px-4 py-2 flex justify-between items-center">
      {/* Left: Instrument/Timeframe pickers + Live Price */}
      <div className="flex items-center gap-4">
        {/* Instrument and timeframe as searchable comboboxes */}
        <div className="flex items-center gap-3" data-tour="chart-instrument">
          <SymbolPicker
            value={instrument}
            onChange={onInstrumentChange}
            symbols={mySymbols}
            showChevron
          />
          <Combobox
            value={granularity}
            onChange={onGranularityChange}
            options={granularityOptions}
            width="w-24"
            showChevron
          />
        </div>

        {/* Live price or Historical View OHLC */}
        <div className="flex items-center text-sm border-l border-[var(--color-border)] pl-4 min-w-[320px]">
          <LivePriceDisplay
            isHistoricalView={isHistoricalView}
            hoveredCandle={hoveredCandle}
            streaming={streaming}
            currentPrice={currentPrice}
            instrument={instrument}
          />
        </div>

        {/* Loading indicator */}
        {loading && (
          <div className="border-l border-[var(--color-border)] pl-4">
            <span className="text-[var(--color-text-muted)] text-xs">Loading...</span>
          </div>
        )}

        {/* Candle countdown timer */}
        {!isHistoricalView && !loading && (
          <div className="border-l border-[var(--color-border)] pl-4">
            <CandleCountdown granularity={granularity} />
          </div>
        )}
      </div>

      {/* Right: S/R Tools + Indicators + Execute */}
      <div className="flex items-center gap-3">
        {/* Recenter view (also: double-click the chart) */}
        <button
          onClick={onRecenter}
          className="p-1.5 rounded text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
          title="Recenter chart (double-click chart)"
          aria-label="Recenter chart"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <circle cx="12" cy="12" r="3" />
            <path strokeLinecap="round" d="M12 2v4m0 12v4M2 12h4m12 0h4" />
          </svg>
        </button>

        {/* S/R Tools - only on main chart */}
        {isMainChart && (
          <div className="relative">
            {srEditingMode ? (
              <div className="flex items-center gap-1">
                <button
                  onClick={onSaveZone}
                  disabled={pendingZoneBoundary === null || secondBoundary === null}
                  className={`px-3 py-1.5 rounded-l text-sm font-medium transition-colors ${
                    pendingZoneBoundary !== null && secondBoundary !== null
                      ? 'bg-[var(--color-buy)] text-white hover:bg-[var(--color-buy)]/80'
                      : 'bg-[var(--color-info)] text-white opacity-50 cursor-not-allowed'
                  }`}
                  title={pendingZoneBoundary !== null && secondBoundary !== null ? 'Save zone' : 'Set both boundaries first'}
                >
                  Save
                </button>
                <button
                  onClick={onCancelZoneEdit}
                  className="px-2 py-1.5 rounded-r text-sm font-medium bg-[var(--color-bg-card)] text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-elevated)] border-l border-[var(--color-border)]"
                  title="Cancel (Esc)"
                >
                  Cancel
                </button>
              </div>
            ) : (
              <>
                <button
                  onClick={onSrMenuToggle}
                  className={`px-3 py-1.5 rounded text-sm font-medium transition-colors flex items-center gap-1.5 ${
                    srMenuOpen ? 'text-[var(--color-info-text)]' : 'text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]'
                  }`}
                >
                  Zones
                  <svg className={`w-3 h-3 transition-transform ${srMenuOpen ? 'rotate-180' : ''}`} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                  </svg>
                </button>
                <SRToolsMenu
                  isOpen={srMenuOpen}
                  onClose={onSrMenuClose}
                  isMainChart={isMainChart}
                  zoneCount={zoneCount}
                  importingPivots={importingPivots}
                  confirmClearAll={confirmClearAll}
                  onDrawZone={onDrawZone}
                  onImportPivots={onImportPivots}
                  onClearAll={onClearAllZones}
                  onConfirmClearAll={onConfirmClearAll}
                  onCancelClearAll={onCancelClearAll}
                />
              </>
            )}
          </div>
        )}

        {/* Indicators Popover */}
        <div className="relative" data-tour="chart-indicators">
          <button
            onClick={onIndicatorMenuToggle}
            className={`px-3 py-1.5 rounded text-sm font-medium transition-colors flex items-center gap-1.5 ${
              indicatorMenuOpen || indicators.length > 0
                ? 'text-[var(--color-info-text)]'
                : 'text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]'
            }`}
          >
            Indicators
            {indicators.length > 0 && (
              <span className="text-[var(--color-text-muted)] text-xs">
                ({indicators.length})
              </span>
            )}
            <svg className={`w-3 h-3 transition-transform ${indicatorMenuOpen ? 'rotate-180' : ''}`} fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
            </svg>
          </button>
          <IndicatorMenu
            isOpen={indicatorMenuOpen}
            onClose={onIndicatorMenuClose}
            indicators={indicators}
            onAddIndicator={onAddIndicator}
            onUpdateIndicator={onUpdateIndicator}
            onRemoveIndicator={onRemoveIndicator}
          />
        </div>

        {/* Signal badge (execution lives in the wickd CLI: wickd approve) */}
        {signalDirection && strategyId && (
          <span
            className={`px-3 py-1.5 rounded text-sm font-medium ${
              signalDirection === 'long'
                ? 'bg-[var(--color-buy)]/20 text-[var(--color-buy)]'
                : 'bg-[var(--color-sell)]/20 text-[var(--color-sell)]'
            }`}
            title="Execution happens in the wickd CLI (wickd approve)"
          >
            Signal: {signalDirection.toUpperCase()}
          </span>
        )}
      </div>
    </div>
  );
};
