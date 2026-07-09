import { Strategy } from '../../types/strategy';
import { StrategyVersion } from './types';
import { isStoreStrategy } from '../../lib/strategyStore';

/**
 * Thin strategy header for the viewer/runner backtest window (AGT-651):
 * name + status badges, notes, read-only source viewing, and promotion.
 * The builder-era Edit/Clone/Save actions are gone — strategies are
 * authored through the wickd CLI against the unified `.rhai` store.
 */
interface StrategyHeaderBarProps {
  selectedStrategy: Strategy;
  workingCopyModified: boolean;
  activeVersionId: string;
  activeVersion: StrategyVersion | undefined;
  /** Absent for read-only store entries (promotion is a local-store concept). */
  onPromoteClick?: () => void;
  /** Present when the strategy has viewable `.rhai` source. */
  onViewSource?: () => void;
  onNotesClick: () => void;
}

export const StrategyHeaderBar = ({
  selectedStrategy,
  workingCopyModified,
  activeVersionId: _activeVersionId,
  activeVersion: _activeVersion,
  onPromoteClick,
  onViewSource,
  onNotesClick,
}: StrategyHeaderBarProps) => {
  const fromStore = isStoreStrategy(selectedStrategy.id);
  return (
    <div className="pb-4 mb-4 border-b border-[var(--color-border)]">
      {/* Header bar */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <h2 className="text-lg font-semibold text-[var(--color-text-primary)]">{selectedStrategy.name}</h2>
          {fromStore && (
            <span
              className="text-[10px] px-1.5 py-0.5 bg-[var(--color-info)]/20 text-[var(--color-info)] rounded"
              title="Read-only .rhai strategy from ~/.wickd/strategies (managed via the wickd CLI)"
              data-testid="store-badge"
            >
              .rhai store
            </span>
          )}
          {workingCopyModified && (
            <span className="text-[10px] px-1.5 py-0.5 bg-[var(--color-info)]/20 text-[var(--color-info)] rounded">Modified</span>
          )}
          {selectedStrategy?.is_locked && (
            <span className={`text-[10px] px-1.5 py-0.5 rounded flex items-center gap-1 ${
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
          {/* Notes button */}
          <button
            onClick={onNotesClick}
            className="p-1 text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
            title="Strategy Notes"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
            </svg>
          </button>
        </div>

        <div className="flex items-center gap-2">
          {/* Read-only source viewer (scripted strategies) */}
          {onViewSource && (
            <button
              onClick={onViewSource}
              className="text-xs px-2 py-1 border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:border-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] rounded transition-colors"
              data-testid="view-source-button"
            >
              View Source
            </button>
          )}

          {/* Promote to live trading / Demote button (local-store rows only) */}
          {onPromoteClick && (
            selectedStrategy?.is_promoted ? (
              <button
                onClick={onPromoteClick}
                className="text-xs px-2 py-1 bg-[var(--color-warning)] hover:bg-[var(--color-warning)]/90 text-white rounded transition-colors"
                title="Remove from live trading"
              >
                Deactivate
              </button>
            ) : (
              <button
                onClick={onPromoteClick}
                className="text-xs px-2 py-1 border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:border-[var(--color-buy)] hover:text-[var(--color-buy)] rounded transition-colors"
                title="Promote to live trading"
              >
                Go Live
              </button>
            )
          )}
        </div>
      </div>
    </div>
  );
};
