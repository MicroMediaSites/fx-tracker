/**
 * SourceViewerModal — read-only `.rhai` source viewer (AGT-651).
 *
 * The app is a strategy viewer/runner: scripts are authored through the
 * wickd CLI (or any editor) into the unified store, and this modal is the
 * app-side "inspect source" surface. Strictly read-only — it reuses
 * ScriptPanel without its editable mode.
 */

import { ScriptPanel } from '../strategy/ScriptPanel';

interface SourceViewerModalProps {
  isOpen: boolean;
  onClose: () => void;
  strategyName: string;
  script: string;
}

export const SourceViewerModal = ({
  isOpen,
  onClose,
  strategyName,
  script,
}: SourceViewerModalProps) => {
  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
      onClick={onClose}
      data-testid="source-viewer-modal"
    >
      <div
        className="w-full max-w-3xl max-h-[85vh] flex flex-col bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-lg shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--color-border)]">
          <div className="min-w-0">
            <h2 className="text-sm font-semibold text-[var(--color-text-primary)] truncate">
              {strategyName}
            </h2>
            <p className="text-xs text-[var(--color-text-muted)]">
              Read-only — edit via <code>wickd strategy update</code>
            </p>
          </div>
          <button
            onClick={onClose}
            className="text-xs px-2 py-1 border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] rounded transition-colors"
            data-testid="source-viewer-close"
          >
            Close
          </button>
        </div>
        <div className="p-4 overflow-auto">
          <ScriptPanel script={script} className="max-h-[65vh]" />
        </div>
      </div>
    </div>
  );
};
