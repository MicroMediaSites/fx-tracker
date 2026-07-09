/**
 * WalkForwardConfig - Configuration section for Walk-Forward Analysis
 */
import { useState, useRef, useEffect, useMemo } from 'react';
import { OptimizationObjective, OPTIMIZATION_OBJECTIVE_LABELS } from '../../types/strategy';
import { SymbolPicker } from '../ui/SymbolPicker';
import { GranularitySelector } from '../ui/GranularitySelector';
import { DateInput } from '../ui/DateInput';
import { useSettingsStore } from '../../stores/settingsStore';
import { TRAIN_WINDOW_OPTIONS, TEST_WINDOW_OPTIONS } from './walkForwardUtils';
import { getMaxDevEndDate } from './QuarterGrid';

// Generic combobox for simple value/label options
interface ComboboxOption<T> {
  value: T;
  label: string;
}

interface ComboboxProps<T> {
  value: T;
  onChange: (value: T) => void;
  options: ComboboxOption<T>[];
  disabled?: boolean;
}

function Combobox<T extends string | number>({
  value,
  onChange,
  options,
  disabled = false,
}: ComboboxProps<T>) {
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const currentLabel = options.find(o => o.value === value)?.label || String(value);

  return (
    <div className="relative" ref={containerRef}>
      <button
        type="button"
        onClick={() => !disabled && setIsOpen(!isOpen)}
        disabled={disabled}
        className={`w-full flex items-center justify-between bg-transparent border rounded px-3 py-2 text-xs transition-colors focus:outline-none ${
          isOpen
            ? 'border-[var(--color-border-focus)]'
            : 'border-[var(--color-border)] hover:border-[var(--color-border-focus)]'
        } ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
      >
        <span className="text-[var(--color-text-primary)]">{currentLabel}</span>
        <svg
          className={`w-3 h-3 text-[var(--color-text-muted)] transition-transform duration-200 ${isOpen ? 'rotate-180' : ''}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {isOpen && (
        <div className="absolute top-full left-0 mt-1 w-full bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded shadow-xl z-50 py-1 max-h-48 overflow-auto">
          {options.map((opt) => (
            <button
              key={String(opt.value)}
              type="button"
              onClick={() => {
                onChange(opt.value);
                setIsOpen(false);
              }}
              className={`w-full text-left px-3 py-1.5 text-xs transition-colors ${
                opt.value === value
                  ? 'text-[var(--color-text-primary)] bg-[var(--color-bg-active)]'
                  : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-hover)]'
              }`}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

interface WalkForwardConfigProps {
  anchored: boolean;
  setAnchored: (value: boolean) => void;
  instrument: string;
  setInstrument: (value: string) => void;
  granularity: string;
  setGranularity: (value: string) => void;
  devDateFrom: string;
  setDevDateFrom: (value: string) => void;
  devDateTo: string;
  setDevDateTo: (value: string) => void;
  trainMonths: number;
  setTrainMonths: (value: number) => void;
  testMonths: number;
  setTestMonths: (value: number) => void;
  objective: OptimizationObjective;
  setObjective: (value: OptimizationObjective) => void;
  wfRunning: boolean;
  hasOptimizableParams: boolean;
  expectedWindows: number;
  totalCombinations: number;
  onRunWalkForward: () => void;
}

export const WalkForwardConfig = ({
  anchored,
  setAnchored,
  instrument,
  setInstrument,
  granularity,
  setGranularity,
  devDateFrom,
  setDevDateFrom,
  devDateTo,
  setDevDateTo,
  trainMonths,
  setTrainMonths,
  testMonths,
  setTestMonths,
  objective,
  setObjective,
  wfRunning,
  hasOptimizableParams,
  expectedWindows,
  totalCombinations,
  onRunWalkForward,
}: WalkForwardConfigProps) => {
  const { mySymbols } = useSettingsStore();

  // Calculate max dev end date based on holdout window
  const maxDevEndDate = useMemo(() => getMaxDevEndDate(), []);

  return (
    <div className="space-y-4">
      {/* Row 1: Instrument, Timeframe, Dates */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        <div>
          <label className="block text-xs text-[var(--color-text-muted)] mb-1">Instrument</label>
          <SymbolPicker value={instrument} onChange={setInstrument} symbols={mySymbols} showChevron />
        </div>
        <div>
          <label className="block text-xs text-[var(--color-text-muted)] mb-1">Timeframe</label>
          <GranularitySelector value={granularity} onChange={setGranularity} />
        </div>
        <DateInput
          value={devDateFrom}
          onChange={setDevDateFrom}
          label="Dev Start"
          disabled={wfRunning}
        />
        <DateInput
          value={devDateTo}
          onChange={setDevDateTo}
          label="Dev End"
          disabled={wfRunning}
        />
      </div>

      {/* Row 2: Window Mode, Training, Test, Objective */}
      <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
        <div>
          <label className="block text-xs text-[var(--color-text-muted)] mb-1">Window Mode</label>
          <div className="flex border border-[var(--color-border)] rounded overflow-hidden">
            <button
              type="button"
              onClick={() => setAnchored(false)}
              disabled={wfRunning}
              className={`flex-1 px-2 py-2 text-xs transition-colors ${
                !anchored
                  ? 'bg-[var(--color-info)]/20 text-[var(--color-info)] border-r border-[var(--color-border)]'
                  : 'text-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)] border-r border-[var(--color-border)]'
              } disabled:opacity-50`}
              title="Training window slides forward (fixed size)"
            >
              Rolling
            </button>
            <button
              type="button"
              onClick={() => setAnchored(true)}
              disabled={wfRunning}
              className={`flex-1 px-2 py-2 text-xs transition-colors ${
                anchored
                  ? 'bg-[var(--color-info)]/20 text-[var(--color-info)]'
                  : 'text-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]'
              } disabled:opacity-50`}
              title="Training window expands from fixed start"
            >
              Anchored
            </button>
          </div>
        </div>
        <div>
          <label className="block text-xs text-[var(--color-text-muted)] mb-1">Training</label>
          <Combobox
            value={trainMonths}
            onChange={setTrainMonths}
            options={TRAIN_WINDOW_OPTIONS}
            disabled={wfRunning}
          />
        </div>
        <div>
          <label className="block text-xs text-[var(--color-text-muted)] mb-1">Test</label>
          <Combobox
            value={testMonths}
            onChange={setTestMonths}
            options={TEST_WINDOW_OPTIONS}
            disabled={wfRunning}
          />
        </div>
        <div>
          <label className="block text-xs text-[var(--color-text-muted)] mb-1">Objective</label>
          <Combobox
            value={objective}
            onChange={(v) => setObjective(v as OptimizationObjective)}
            options={Object.entries(OPTIMIZATION_OBJECTIVE_LABELS).map(([value, label]) => ({
              value,
              label,
            }))}
            disabled={wfRunning}
          />
        </div>
        <div className="flex items-end">
          <button
            onClick={onRunWalkForward}
            disabled={wfRunning || !hasOptimizableParams || expectedWindows === 0}
            className="w-full px-3 py-2 text-xs font-medium border border-[var(--color-info)] text-[var(--color-info)] hover:bg-[var(--color-info)]/10 disabled:opacity-50 disabled:cursor-not-allowed rounded transition-colors"
          >
            {wfRunning ? 'Running...' : 'Run Analysis'}
          </button>
        </div>
      </div>

      {/* Expected windows and estimation */}
      <div className="text-xs text-[var(--color-text-muted)] space-y-1">
        {!hasOptimizableParams ? (
          <span className="text-[var(--color-warning)]">
            No optimizable parameters. Add parameters to your strategy first.
          </span>
        ) : expectedWindows > 0 ? (
          <>
            <div>
              <span className="font-medium text-[var(--color-text-primary)]">{expectedWindows}</span> windows × <span className="font-medium text-[var(--color-text-primary)]">{totalCombinations.toLocaleString()}</span> combinations = <span className="font-medium text-[var(--color-text-primary)]">{(expectedWindows * totalCombinations).toLocaleString()}</span> backtests
              {anchored && <span className="ml-2 text-[var(--color-info)]">(anchored)</span>}
            </div>
            {totalCombinations > 10000 && (
              <div className="text-[var(--color-warning)] flex items-center gap-1">
                <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                </svg>
                Large optimization - may take a while
              </div>
            )}
          </>
        ) : devDateFrom && devDateTo ? (
          <span className="text-[var(--color-warning)]">
            Date range too short for {trainMonths}mo training + {testMonths}mo test
          </span>
        ) : (
          <span>Select development period dates</span>
        )}

        {/* Holdout contamination warning */}
        {devDateTo && devDateTo > maxDevEndDate && (
          <div className="text-amber-400 flex items-center gap-1.5 mt-1">
            <svg className="h-3 w-3 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
            </svg>
            <span>Dev period overlaps holdout data. To preserve holdout integrity, end by {maxDevEndDate}.</span>
          </div>
        )}
      </div>
    </div>
  );
};
