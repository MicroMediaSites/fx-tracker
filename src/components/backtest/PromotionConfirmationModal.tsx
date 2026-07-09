import { Strategy } from '../../types/strategy';
import { PromotionAcknowledgements } from './types';

interface PromotionConfirmationModalProps {
  strategy: Strategy;
  acknowledgements: PromotionAcknowledgements;
  onAcknowledgementChange: (key: keyof PromotionAcknowledgements, value: boolean) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

export const PromotionConfirmationModal = ({
  strategy,
  acknowledgements,
  onAcknowledgementChange,
  onConfirm,
  onCancel,
}: PromotionConfirmationModalProps) => {
  const allAcknowledged = Object.values(acknowledgements).every(Boolean);

  return (
    <div
      className="fixed inset-0 z-[150] flex items-center justify-center"
      onClick={onCancel}
    >
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-[var(--color-bg-elevated)] rounded-lg shadow-xl max-w-lg w-full mx-4 p-6"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-lg font-semibold text-[var(--color-text-primary)] mb-4">Promote Strategy to Live Trading</h3>

        <div className="bg-[var(--color-bg-card)] border border-[var(--color-border)] rounded p-4 mb-4">
          <p className="text-sm text-[var(--color-text-secondary)] mb-3">
            You are about to promote <span className="font-semibold text-[var(--color-text-primary)]">"{strategy.name}"</span> to live trading.
          </p>
          <p className="text-sm text-[var(--color-text-secondary)]">
            Once promoted, this strategy will appear in your Live Monitor window where you can act on pattern matches in real-time.
          </p>
        </div>

        {/* Version info */}
        <div className="bg-[var(--color-bg-card)] border border-[var(--color-border)] rounded px-4 py-2 mb-4 text-xs text-[var(--color-text-muted)]">
          Promoting version <span className="font-mono text-[var(--color-text-secondary)]">v{strategy.version}</span>
          {' '}created {new Date(strategy.created_at).toLocaleDateString()}
        </div>

        {/* Acknowledgement checkboxes */}
        <div className="bg-[var(--color-bg-card)] border border-[var(--color-border)] rounded p-4 mb-4">
          <p className="text-sm text-[var(--color-text-secondary)] mb-3 font-medium">Please confirm you understand:</p>
          <div className="space-y-3">
            <label className="flex items-start gap-3 cursor-pointer group">
              <input
                type="checkbox"
                checked={acknowledgements.ownLogic}
                onChange={(e) => onAcknowledgementChange('ownLogic', e.target.checked)}
                className="mt-0.5 w-4 h-4 rounded bg-[var(--color-bg-hover)] border-[var(--color-border)] text-[var(--color-buy)] focus:ring-[var(--color-buy)] focus:ring-offset-[var(--color-bg-elevated)]"
              />
              <span className="text-sm text-[var(--color-text-secondary)] group-hover:text-[var(--color-text-primary)] transition-colors">
                This strategy represents my own trading logic and decisions
              </span>
            </label>
            <label className="flex items-start gap-3 cursor-pointer group">
              <input
                type="checkbox"
                checked={acknowledgements.independentChoices}
                onChange={(e) => onAcknowledgementChange('independentChoices', e.target.checked)}
                className="mt-0.5 w-4 h-4 rounded bg-[var(--color-bg-hover)] border-[var(--color-border)] text-[var(--color-buy)] focus:ring-[var(--color-buy)] focus:ring-offset-[var(--color-bg-elevated)]"
              />
              <span className="text-sm text-[var(--color-text-secondary)] group-hover:text-[var(--color-text-primary)] transition-colors">
                Any trades I execute are my independent choices
              </span>
            </label>
            <label className="flex items-start gap-3 cursor-pointer group">
              <input
                type="checkbox"
                checked={acknowledgements.noGuarantee}
                onChange={(e) => onAcknowledgementChange('noGuarantee', e.target.checked)}
                className="mt-0.5 w-4 h-4 rounded bg-[var(--color-bg-hover)] border-[var(--color-border)] text-[var(--color-buy)] focus:ring-[var(--color-buy)] focus:ring-offset-[var(--color-bg-elevated)]"
              />
              <span className="text-sm text-[var(--color-text-secondary)] group-hover:text-[var(--color-text-primary)] transition-colors">
                Past backtest performance does not guarantee future results
              </span>
            </label>
            <label className="flex items-start gap-3 cursor-pointer group">
              <input
                type="checkbox"
                checked={acknowledgements.responsible}
                onChange={(e) => onAcknowledgementChange('responsible', e.target.checked)}
                className="mt-0.5 w-4 h-4 rounded bg-[var(--color-bg-hover)] border-[var(--color-border)] text-[var(--color-buy)] focus:ring-[var(--color-buy)] focus:ring-offset-[var(--color-bg-elevated)]"
              />
              <span className="text-sm text-[var(--color-text-secondary)] group-hover:text-[var(--color-text-primary)] transition-colors">
                I am responsible for all trading decisions
              </span>
            </label>
          </div>
        </div>

        {/* Lock warning */}
        <div className="bg-[var(--color-warning)]/20 border border-[var(--color-warning)]/50 rounded p-3 mb-6">
          <p className="text-xs text-[var(--color-warning-text)]">
            <strong>Note:</strong> Once promoted, this strategy will be locked to prevent accidental changes.
            You can clone it later if you want to iterate on a new version.
          </p>
        </div>

        <div className="flex justify-end gap-3">
          <button
            onClick={onCancel}
            className="px-4 py-2 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={!allAcknowledged}
            className="px-4 py-2 bg-[var(--color-buy)] rounded hover:bg-[var(--color-buy)]/90 transition-colors font-medium text-[var(--color-text-primary)] disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:bg-[var(--color-buy)]"
          >
            Promote to Live
          </button>
        </div>
      </div>
    </div>
  );
};
