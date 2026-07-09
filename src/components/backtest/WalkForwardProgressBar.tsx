/**
 * WalkForwardProgressBar - Progress display during walk-forward analysis
 */
import { WalkForwardProgress } from '../../types/strategy';

interface WalkForwardProgressBarProps {
  progress: WalkForwardProgress;
  onCancel: () => void;
}

export const WalkForwardProgressBar = ({ progress, onCancel }: WalkForwardProgressBarProps) => {
  return (
    <div className="bg-[var(--color-bg-elevated)] rounded-lg border border-[var(--color-border)] p-4">
      <div className="flex justify-between items-start mb-3">
        <div className="flex items-center gap-3">
          {/* Animated candlestick loader - keeping hardcoded SVG colors for animation */}
          <div className="flex items-end gap-0.5 h-6">
            <style>{`
              @keyframes candle-pulse {
                0%, 100% { transform: scaleY(0.5); opacity: 0.5; }
                50% { transform: scaleY(1); opacity: 1; }
              }
              .candle-animate { animation: candle-pulse 1.2s ease-in-out infinite; transform-origin: bottom; }
              .candle-1 { animation-delay: 0s; }
              .candle-2 { animation-delay: 0.2s; }
              .candle-3 { animation-delay: 0.4s; }
              .candle-4 { animation-delay: 0.6s; }
            `}</style>
            {/* Candle 1 - bullish */}
            <svg className="candle-animate candle-1" width="6" height="24" viewBox="0 0 6 24">
              <line x1="3" y1="2" x2="3" y2="6" stroke="#4ade80" strokeWidth="1" />
              <rect x="1" y="6" width="4" height="10" fill="#4ade80" />
              <line x1="3" y1="16" x2="3" y2="22" stroke="#4ade80" strokeWidth="1" />
            </svg>
            {/* Candle 2 - bearish */}
            <svg className="candle-animate candle-2" width="6" height="24" viewBox="0 0 6 24">
              <line x1="3" y1="4" x2="3" y2="8" stroke="#f87171" strokeWidth="1" />
              <rect x="1" y="8" width="4" height="8" fill="#f87171" />
              <line x1="3" y1="16" x2="3" y2="20" stroke="#f87171" strokeWidth="1" />
            </svg>
            {/* Candle 3 - bullish */}
            <svg className="candle-animate candle-3" width="6" height="24" viewBox="0 0 6 24">
              <line x1="3" y1="3" x2="3" y2="7" stroke="#4ade80" strokeWidth="1" />
              <rect x="1" y="7" width="4" height="12" fill="#4ade80" />
              <line x1="3" y1="19" x2="3" y2="22" stroke="#4ade80" strokeWidth="1" />
            </svg>
            {/* Candle 4 - bearish */}
            <svg className="candle-animate candle-4" width="6" height="24" viewBox="0 0 6 24">
              <line x1="3" y1="5" x2="3" y2="9" stroke="#f87171" strokeWidth="1" />
              <rect x="1" y="9" width="4" height="6" fill="#f87171" />
              <line x1="3" y1="15" x2="3" y2="19" stroke="#f87171" strokeWidth="1" />
            </svg>
          </div>
          <div>
            <div className="text-sm font-medium text-[var(--color-text-primary)] mb-1">
              {progress.phase === 'optimization' ? 'Optimizing' : 'Backtesting'}{' '}
              {progress.trainStart && progress.trainEnd && (
                <span className="font-normal text-[var(--color-text-secondary)]">
                  {new Date(progress.trainStart).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}
                  {' – '}
                  {new Date(progress.trainEnd).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}
                </span>
              )}
            </div>
            <div className="text-xs text-[var(--color-text-muted)]">
              Window {progress.windowNum} of {progress.totalWindows}
              {progress.testStart && progress.testEnd && (
                <span className="ml-2 text-[var(--color-text-muted)]">
                  → Test: {new Date(progress.testStart).toLocaleDateString('en-US', { month: 'short', year: 'numeric' })}
                </span>
              )}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-sm text-[var(--color-text-muted)]">{progress.percent}%</span>
          <button
            onClick={onCancel}
            className="px-3 py-1 text-sm bg-[var(--color-sell)]/80 hover:bg-[var(--color-sell)] rounded transition-colors text-[var(--color-text-primary)]"
          >
            Cancel
          </button>
        </div>
      </div>
      <div className="w-full bg-[var(--color-bg-hover)] rounded-full h-2">
        <div
          className="bg-purple-600 h-2 rounded-full transition-all duration-300"
          style={{ width: `${progress.percent}%` }}
        />
      </div>
    </div>
  );
};
