interface SRToolsMenuProps {
  isOpen: boolean;
  onClose: () => void;
  isMainChart: boolean;
  zoneCount: number;
  importingPivots: boolean;
  confirmClearAll: boolean;
  onDrawZone: () => void;
  onImportPivots: (timeframe: 'daily' | 'weekly') => void;
  onClearAll: () => void;
  onConfirmClearAll: () => void;
  onCancelClearAll: () => void;
}

export const SRToolsMenu = ({
  isOpen,
  onClose,
  isMainChart,
  zoneCount,
  importingPivots,
  confirmClearAll,
  onDrawZone,
  onImportPivots,
  onClearAll,
  onConfirmClearAll,
  onCancelClearAll,
}: SRToolsMenuProps) => {
  if (!isOpen) return null;

  return (
    <>
      {/* Backdrop to close menu */}
      <div className="fixed inset-0 z-40" onClick={() => { onClose(); onCancelClearAll(); }} />
      <div className="absolute right-0 top-full mt-1 w-48 bg-[var(--color-bg-elevated)] rounded-lg shadow-lg z-50 py-1">
        {/* Draw Zone - only on main chart */}
        {isMainChart && (
          <button
            onClick={onDrawZone}
            className="w-full px-4 py-2 text-left text-sm text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-card)] flex items-center gap-2"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" />
            </svg>
            Custom
          </button>
        )}
        <button
          onClick={() => onImportPivots('daily')}
          disabled={importingPivots}
          className="w-full px-4 py-2 text-left text-sm text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-card)] flex items-center gap-2 disabled:opacity-50"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
          </svg>
          Daily Pivots
        </button>
        <button
          onClick={() => onImportPivots('weekly')}
          disabled={importingPivots}
          className="w-full px-4 py-2 text-left text-sm text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-card)] flex items-center gap-2 disabled:opacity-50"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
          </svg>
          Weekly Pivots
        </button>
        {/* Clear All - only on main chart */}
        {isMainChart && zoneCount > 0 && (
          <>
            <div className="border-t border-[var(--color-border)] my-1" />
            {confirmClearAll ? (
              <div className="px-4 py-2">
                <p className="text-xs text-[var(--color-text-muted)] mb-2">Clear {zoneCount} zone{zoneCount !== 1 ? 's' : ''}?</p>
                <div className="flex gap-2">
                  <button
                    onClick={onClearAll}
                    className="flex-1 px-2 py-1 text-xs bg-[var(--color-sell)] text-white rounded hover:bg-[var(--color-sell)]/80"
                  >
                    Clear
                  </button>
                  <button
                    onClick={onCancelClearAll}
                    className="flex-1 px-2 py-1 text-xs bg-[var(--color-bg-card)] text-[var(--color-text-secondary)] rounded hover:bg-[var(--color-bg-elevated)]"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            ) : (
              <button
                onClick={onConfirmClearAll}
                className="w-full px-4 py-2 text-left text-sm text-[var(--color-sell)] hover:bg-[var(--color-bg-card)] flex items-center gap-2"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                </svg>
                Clear All
              </button>
            )}
          </>
        )}
      </div>
    </>
  );
};
