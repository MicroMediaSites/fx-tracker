/**
 * AI-powered strategy error recovery component
 *
 * Displays when a strategy fails to parse, automatically analyzes the error
 * with AI, and offers a one-click fix if possible.
 */

import { useEffect, useRef } from 'react';
import { useStrategyRecovery } from '../../hooks/useStrategyRecovery';

interface StrategyErrorRecoveryProps {
  /** The original error message */
  error: string;
  /** The strategy JSON that failed to parse */
  strategyJson: string;
  /** Called when user clicks "Apply Fix" with the corrected JSON */
  onApplyFix: (correctedJson: string) => void;
  /** Called when user wants to apply fix to a copy (safer option) */
  onApplyFixAsCopy?: (correctedJson: string) => void;
  /** Called when user dismisses the recovery UI */
  onDismiss: () => void;
}

/**
 * Inline error recovery component
 *
 * Automatically calls AI to analyze the error when mounted.
 * Shows loading state, then explanation + fix button if available.
 */
export function StrategyErrorRecovery({
  error,
  strategyJson,
  onApplyFix,
  onApplyFixAsCopy,
  onDismiss,
}: StrategyErrorRecoveryProps) {
  const { recovering, result, recoverError, awaitingConsent, recover, confirmRecovery } =
    useStrategyRecovery();

  // Track which error we've already staged recovery for (prevents double-calls)
  const recoveredErrorRef = useRef<string | null>(null);

  // Stage recovery when the component mounts or the error changes. This does NOT
  // send anything off-device (AGT-669) — it only surfaces the consent prompt
  // below; the strategy JSON leaves the device only when the user confirms.
  useEffect(() => {
    // Skip if we've already staged recovery for this exact error
    if (recoveredErrorRef.current === error) return;
    // Skip if already recovering or have a result
    if (recovering || result) return;

    recoveredErrorRef.current = error;
    recover(error, strategyJson);
  }, [error, strategyJson, recover, recovering, result]);

  // Awaiting explicit, disclosed consent before any data leaves the device.
  if (awaitingConsent) {
    return (
      <div className="p-4 bg-slate-800/80 border border-slate-600/50 rounded-lg space-y-3">
        <div className="text-sm text-red-300">{error}</div>
        <div className="text-sm text-slate-300">
          We can send this strategy's configuration to Anthropic's Claude AI to
          analyze the error and suggest a fix. Your strategy JSON will leave your
          device only if you choose to continue.
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            onClick={() => confirmRecovery()}
            className="px-3 py-1.5 bg-slate-600 hover:bg-slate-500 text-white text-sm rounded transition-colors"
          >
            Analyze with AI
          </button>
          <button
            onClick={onDismiss}
            className="px-3 py-1.5 bg-slate-700 hover:bg-slate-600 text-slate-300 text-sm rounded transition-colors"
          >
            No thanks
          </button>
        </div>
      </div>
    );
  }

  // Still analyzing...
  if (recovering) {
    return (
      <div className="p-4 bg-yellow-900/20 border border-yellow-500/30 rounded-lg">
        <div className="flex items-center gap-3">
          <div className="animate-spin h-4 w-4 border-2 border-yellow-400 border-t-transparent rounded-full shrink-0" />
          <span className="text-yellow-300 text-sm">
            Hmm, something's off. Sit tight while we fix it...
          </span>
        </div>
      </div>
    );
  }

  // AI recovery failed - fall back to original error
  if (recoverError || !result) {
    return (
      <div className="p-4 bg-red-900/20 border border-red-500/30 rounded-lg">
        <div className="flex items-start justify-between gap-4">
          <div className="text-red-300 text-sm">{error}</div>
          <button
            onClick={onDismiss}
            className="text-gray-400 hover:text-gray-300 text-sm shrink-0"
          >
            Dismiss
          </button>
        </div>
      </div>
    );
  }

  // Show recovery result
  return (
    <div className="p-4 bg-slate-800/80 border border-slate-600/50 rounded-lg space-y-3">
      {/* Header with icon */}
      <div className="flex items-start gap-3">
        <div className="text-yellow-400 mt-0.5">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 20 20"
            fill="currentColor"
            className="w-5 h-5"
          >
            <path
              fillRule="evenodd"
              d="M8.485 2.495c.673-1.167 2.357-1.167 3.03 0l6.28 10.875c.673 1.167-.17 2.625-1.516 2.625H3.72c-1.347 0-2.189-1.458-1.515-2.625L8.485 2.495zM10 5a.75.75 0 01.75.75v3.5a.75.75 0 01-1.5 0v-3.5A.75.75 0 0110 5zm0 9a1 1 0 100-2 1 1 0 000 2z"
              clipRule="evenodd"
            />
          </svg>
        </div>
        <div className="flex-1">
          <div className="text-sm font-medium text-slate-200 mb-1">
            Strategy Error Detected
          </div>
          <div className="text-sm text-slate-400">{result.explanation}</div>
        </div>
      </div>

      {/* Changes list */}
      {result.changes_made.length > 0 && (
        <div className="pl-8">
          <div className="text-xs text-slate-500 mb-1">
            {result.corrected_json ? 'Changes to apply:' : 'Changes attempted:'}
          </div>
          <ul className="text-xs text-slate-400 list-disc list-inside space-y-0.5">
            {result.changes_made.map((change, i) => (
              <li key={i}>{change}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Actions */}
      <div className="pl-8 space-y-2">
        {result.corrected_json && (
          <div className="flex flex-wrap gap-2">
            {onApplyFixAsCopy && (
              <button
                onClick={() => onApplyFixAsCopy(result.corrected_json!)}
                className="px-3 py-1.5 bg-green-600 hover:bg-green-500 text-white text-sm rounded transition-colors"
              >
                Fix & Save as Copy
              </button>
            )}
            <button
              onClick={() => onApplyFix(result.corrected_json!)}
              className="px-3 py-1.5 bg-slate-600 hover:bg-slate-500 text-white text-sm rounded transition-colors"
            >
              {onApplyFixAsCopy ? 'Fix Current' : 'Apply Fix'}
            </button>
            <button
              onClick={onDismiss}
              className="px-3 py-1.5 bg-slate-700 hover:bg-slate-600 text-slate-300 text-sm rounded transition-colors"
            >
              Dismiss
            </button>
          </div>
        )}
        {result.corrected_json && onApplyFixAsCopy && (
          <p className="text-xs text-slate-500">
            "Save as Copy" creates a new version, keeping your original safe.
          </p>
        )}
        {!result.corrected_json && (
          <button
            onClick={onDismiss}
            className="px-3 py-1.5 bg-slate-600 hover:bg-slate-500 text-white text-sm rounded transition-colors"
          >
            Dismiss
          </button>
        )}
      </div>
    </div>
  );
}
