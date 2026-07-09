/**
 * Hook for AI-powered strategy error recovery
 *
 * When a strategy fails to parse or validate, this hook calls Haiku to analyze
 * the error and suggest minimal fixes. This is NOT a strategy builder - it only
 * repairs broken JSON.
 *
 * AGT-669 (privacy): recovery ships the user's strategy JSON to Anthropic, which
 * contradicts the local-first promise if done silently. Egress is therefore
 * opt-in with disclosed consent: `recover()` only STAGES a request locally and
 * flips `awaitingConsent` true; nothing leaves the device until the user
 * explicitly calls `confirmRecovery()`.
 */

import { useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Result from the AI recovery attempt */
export interface RecoveryResult {
  /** Human-readable explanation of what was wrong and how to fix it */
  explanation: string;
  /** The corrected strategy JSON, if AI was able to fix it */
  corrected_json: string | null;
  /** List of specific changes made */
  changes_made: string[];
}

interface UseStrategyRecoveryReturn {
  /** Whether recovery is in progress */
  recovering: boolean;
  /** The recovery result, if available */
  result: RecoveryResult | null;
  /** Error from the recovery attempt itself (not the original strategy error) */
  recoverError: string | null;
  /**
   * True when a recovery has been staged and is awaiting the user's explicit
   * consent. While true, NOTHING has been sent off-device yet.
   */
  awaitingConsent: boolean;
  /**
   * Stage a recovery request. This does NOT send anything to Anthropic — it only
   * records the error + strategy JSON locally and flips `awaitingConsent` true.
   * The strategy JSON leaves the device only after `confirmRecovery()`.
   */
  recover: (error: string, strategyJson: string) => void;
  /**
   * Explicit, disclosed consent to proceed: sends the staged strategy JSON to
   * Anthropic for analysis. No-op if nothing is staged.
   */
  confirmRecovery: () => Promise<void>;
  /** Get the corrected JSON (convenience method) */
  getCorrectedJson: () => string | null;
  /** Clear the recovery state (including any staged, un-consented request) */
  clear: () => void;
}

/**
 * Hook for recovering from strategy parsing errors using AI
 *
 * @example
 * ```tsx
 * const { recovering, result, recover } = useStrategyRecovery();
 *
 * // When a strategy error occurs
 * try {
 *   await runBacktest(strategyJson);
 * } catch (err) {
 *   setError(err);
 *   recover(err, strategyJson); // Start recovery automatically
 * }
 *
 * // In render
 * if (recovering) return <div>Analyzing error...</div>;
 * if (result?.corrected_json) {
 *   return <button onClick={() => useFixed(result.corrected_json)}>Apply Fix</button>;
 * }
 * ```
 */
export function useStrategyRecovery(): UseStrategyRecoveryReturn {
  const [recovering, setRecovering] = useState(false);
  const [result, setResult] = useState<RecoveryResult | null>(null);
  const [recoverError, setRecoverError] = useState<string | null>(null);
  const [awaitingConsent, setAwaitingConsent] = useState(false);

  // Holds the staged request locally until the user consents. Nothing here is
  // transmitted; it only exists on-device between recover() and confirmRecovery().
  const pendingRef = useRef<{ error: string; strategyJson: string } | null>(null);

  // Stage a recovery request WITHOUT any network egress. The strategy JSON is
  // retained locally and only sent to Anthropic once the user confirms.
  const recover = useCallback((error: string, strategyJson: string) => {
    pendingRef.current = { error, strategyJson };
    setResult(null);
    setRecoverError(null);
    setRecovering(false);
    setAwaitingConsent(true);
  }, []);

  // Explicit, disclosed consent: transmit the staged strategy JSON to Anthropic.
  const confirmRecovery = useCallback(async () => {
    const pending = pendingRef.current;
    if (!pending) return;

    setAwaitingConsent(false);
    setRecovering(true);
    setResult(null);
    setRecoverError(null);

    try {
      const res = await invoke<RecoveryResult>('recover_strategy_error', {
        errorMessage: pending.error,
        strategyJson: pending.strategyJson,
      });
      setResult(res);
    } catch (err) {
      // Recovery itself failed - fall back to showing original error
      setRecoverError(err instanceof Error ? err.message : String(err));
    } finally {
      setRecovering(false);
    }
  }, []);

  const getCorrectedJson = useCallback(() => {
    return result?.corrected_json ?? null;
  }, [result]);

  const clear = useCallback(() => {
    pendingRef.current = null;
    setResult(null);
    setRecoverError(null);
    setAwaitingConsent(false);
  }, []);

  return {
    recovering,
    result,
    recoverError,
    awaitingConsent,
    recover,
    confirmRecovery,
    getCorrectedJson,
    clear,
  };
}
