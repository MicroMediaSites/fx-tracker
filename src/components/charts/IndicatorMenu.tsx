import { useState, useRef, useEffect } from 'react';
import { useMemo } from 'react';
import {
  INDICATOR_METADATA,
  type IndicatorType,
} from '../../types/strategy';
import type { ChartIndicatorConfig } from './chartTypes';
import {
  INDICATOR_CATEGORIES,
  INDICATOR_TYPES_BY_CATEGORY,
  formatIndicatorLabel,
} from './indicatorHelpers';
import { IndicatorConfigModal } from './IndicatorConfigModal';

interface IndicatorMenuProps {
  isOpen: boolean;
  onClose: () => void;
  indicators: ChartIndicatorConfig[];
  onAddIndicator: (type: IndicatorType, params: Record<string, number>, colors?: Record<string, string>) => void;
  onUpdateIndicator: (id: string, params: Record<string, number>, colors?: Record<string, string>) => void;
  onRemoveIndicator: (id: string) => void;
}

export const IndicatorMenu = ({
  isOpen,
  onClose,
  indicators,
  onAddIndicator,
  onUpdateIndicator,
  onRemoveIndicator,
}: IndicatorMenuProps) => {
  // Modal state (only used for editing)
  const [modalOpen, setModalOpen] = useState(false);
  const [editingIndicator, setEditingIndicator] = useState<ChartIndicatorConfig | null>(null);

  // Flash highlight state
  const [flashingType, setFlashingType] = useState<IndicatorType | null>(null);
  const flashTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Clear flash timer on unmount (Bug #20)
  useEffect(() => {
    return () => {
      if (flashTimerRef.current !== null) {
        clearTimeout(flashTimerRef.current);
      }
    };
  }, []);

  // Count of each indicator type currently active
  const countByType = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const ind of indicators) {
      counts[ind.type] = (counts[ind.type] ?? 0) + 1;
    }
    return counts;
  }, [indicators]);

  if (!isOpen) return null;

  const handleAddClick = (type: IndicatorType) => {
    // Show flash highlight - instant blue, then fade over 500ms (Bug #20: clear previous timer)
    if (flashTimerRef.current !== null) {
      clearTimeout(flashTimerRef.current);
    }
    setFlashingType(type);
    flashTimerRef.current = setTimeout(() => {
      setFlashingType(null);
      flashTimerRef.current = null;
    }, 50);
    // Add with defaults immediately - no modal
    onAddIndicator(type, {});
  };

  const handleEditClick = (indicator: ChartIndicatorConfig) => {
    setEditingIndicator(indicator);
    setModalOpen(true);
  };

  const handleModalSave = (_type: IndicatorType, params: Record<string, number>, colors?: Record<string, string>) => {
    if (editingIndicator) {
      onUpdateIndicator(editingIndicator.id, params, colors);
    }
    setModalOpen(false);
    setEditingIndicator(null);
  };

  const handleModalClose = () => {
    setModalOpen(false);
    setEditingIndicator(null);
  };

  return (
    <>
      <div className="fixed inset-0 z-40" onClick={onClose} />
      <div className="absolute right-0 top-full mt-1 w-64 bg-[var(--color-bg-elevated)] rounded-lg shadow-lg z-50 py-2 max-h-96 flex flex-col">
        {/* Active Indicators Section - sticky at top */}
        <div className="flex-shrink-0">
          <div className="px-4 py-1 text-xs font-semibold text-[var(--color-text-muted)] uppercase tracking-wider">
            Active {indicators.length > 0 && `(${indicators.length})`}
          </div>
          <div className="min-h-[72px] max-h-[72px] overflow-y-auto">
            {indicators.length === 0 ? (
              <div className="px-4 py-6 text-sm text-[var(--color-text-muted)] text-center">
                None selected
              </div>
            ) : (
              indicators.map((ind) => (
                <div
                  key={ind.id}
                  className="w-full px-4 py-2 flex items-center justify-between hover:bg-[var(--color-bg-card)] group"
                >
                  <button
                    onClick={() => handleEditClick(ind)}
                    className="flex-1 text-left text-sm text-[var(--color-info-text)] hover:underline"
                  >
                    {formatIndicatorLabel(ind)}
                  </button>
                  <button
                    onClick={() => onRemoveIndicator(ind.id)}
                    className="p-1 text-[var(--color-text-muted)] hover:text-[var(--color-sell)] opacity-0 group-hover:opacity-100 transition-opacity"
                    title="Remove"
                  >
                    <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                    </svg>
                  </button>
                </div>
              ))
            )}
          </div>
          <div className="border-t border-[var(--color-border)] my-2" />
        </div>

        {/* Add Indicator Section - scrollable */}
        <div className="flex-1 overflow-y-auto min-h-0">
          {INDICATOR_CATEGORIES.map((category) => {
            const types = INDICATOR_TYPES_BY_CATEGORY[category];
            if (!types || types.length === 0) return null;

            return (
              <div key={category} className="mb-2 last:mb-0">
                <div className="px-4 py-1 text-xs text-[var(--color-text-muted)]">
                  {category}
                </div>
                {types.map((type) => {
                  const meta = INDICATOR_METADATA[type];
                  const isFlashing = flashingType === type;
                  const activeCount = countByType[type] ?? 0;

                  return (
                    <button
                      key={type}
                      onClick={() => handleAddClick(type)}
                      className="w-full px-4 py-2 text-left text-sm hover:bg-[var(--color-bg-card)] flex items-center justify-between"
                    >
                      <span
                        className={`transition-colors ${
                          isFlashing
                            ? 'text-[var(--color-info-text)] duration-0'
                            : 'text-[var(--color-text-secondary)] duration-500'
                        }`}
                      >
                        {meta.fullName ?? meta.label}
                      </span>
                      <span className="flex items-center gap-2">
                        {activeCount > 0 && (
                          <span className="text-xs text-[var(--color-text-muted)]">
                            ({activeCount})
                          </span>
                        )}
                      </span>
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>

      {/* Config Modal - only used for editing */}
      <IndicatorConfigModal
        isOpen={modalOpen}
        indicator={editingIndicator ?? undefined}
        onSave={handleModalSave}
        onClose={handleModalClose}
      />
    </>
  );
};
