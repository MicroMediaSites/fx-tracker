/**
 * useBacktestJob - Generic hook for backtest job tracking and event handling.
 *
 * Encapsulates common job management logic shared across different backtesting types:
 * - Walk-Forward Analysis
 * - Train/Test Split
 * - Monte Carlo Simulation
 * - etc.
 *
 * Handles:
 * - Job ID state and refs for stable event callbacks
 * - Local-store reads for active and completed jobs (AGT-645; was Zero)
 * - Reconnection to running jobs on mount (with stale detection)
 * - Event listeners for heartbeat and completion
 * - Local-store updates on job progress/completion/failure/cancel
 *
 * The wickd local store (~/.wickd/app.db) is the only persistence: job rows
 * are written by this hook on start/heartbeat/completion, so a job survives a
 * window reload and the stale detector can fail jobs the app crashed out of.
 */

import { useState, useEffect, useMemo, useRef, useCallback } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import {
  listBacktestJobs,
  saveBacktestJob,
  updateBacktestJob,
} from '../../../lib/localStore';
import type { BacktestJob } from '../walkForwardTypes';

// Re-export BacktestJob for consumers
export type { BacktestJob } from '../walkForwardTypes';

/** Callbacks for job lifecycle events */
export interface BacktestJobCallbacks<TProgress = unknown, TResult = unknown> {
  /** Called when a heartbeat event is received with progress details */
  onProgress: (progress: TProgress) => void;
  /** Called when job completes successfully */
  onComplete: (result: TResult) => void;
  /** Called when job fails */
  onError: (error: string) => void;
  /** Called when reconnecting to a running job (to restore config state) */
  onReconnect?: (job: BacktestJob, progressDetail: TProgress | null) => void;
  /** Optional: Custom success notification */
  getSuccessNotification?: (result: TResult) => { title: string; body: string } | null;
  /** Optional: Custom failure notification */
  getFailureNotification?: (error: string) => { title: string; body: string } | null;
}

export interface UseBacktestJobOptions<TProgress = unknown, TResult = unknown> {
  strategyId: string;
  callbacks: BacktestJobCallbacks<TProgress, TResult>;
}

export interface UseBacktestJobReturn {
  /** Current job ID being tracked */
  currentJobId: string | null;
  /** Whether a job is currently running */
  isRunning: boolean;
  /** Error message if job failed */
  error: string | null;
  /** Active jobs for this strategy (pending or running) */
  activeJobs: BacktestJob[];
  /** All jobs for this strategy */
  allJobs: BacktestJob[];
  /** Start a new job - creates DB record and sets up tracking */
  startJob: (jobType: string, params: Record<string, unknown>) => Promise<string>;
  /** Manually set error state */
  setError: (error: string | null) => void;
  /** Signal that the invoke command finished (even if events handled completion) */
  finishRunning: () => void;
  /** Reset all state (when strategy changes) */
  resetState: () => void;
  /** Ref for stable access to current job ID in callbacks */
  currentJobIdRef: React.MutableRefObject<string | null>;
}

/** Stale job threshold - jobs not updated in this time are considered crashed */
const STALE_JOB_THRESHOLD_MS = 60 * 1000;

/** Generic heartbeat event payload */
interface JobHeartbeat {
  jobId: string;
  strategyId: string;
  progressDetail: unknown;
}

/** Generic completion event payload */
interface JobCompleted {
  jobId: string;
  strategyId: string;
  status: 'completed' | 'failed' | 'cancelled';
  hasResult: boolean;
  result?: unknown;
  error?: string;
}

export function useBacktestJob<TProgress = unknown, TResult = unknown>({
  strategyId,
  callbacks,
}: UseBacktestJobOptions<TProgress, TResult>): UseBacktestJobReturn {
  // Job tracking state
  const [currentJobId, setCurrentJobId] = useState<string | null>(null);
  const currentJobIdRef = useRef<string | null>(null);
  const hasAttemptedReconnect = useRef(false);

  // Jobs for this strategy, loaded from the local store.
  const [allJobs, setAllJobs] = useState<BacktestJob[]>([]);

  // Store callbacks in a ref so listener effects don't need to re-fire when
  // the callbacks object changes. Callers may pass an unmemoized object literal,
  // which would be a new reference every render. Using a ref keeps the listeners
  // stable and avoids unnecessary teardown/setup churn.
  const callbacksRef = useRef(callbacks);
  useEffect(() => {
    callbacksRef.current = callbacks;
  }, [callbacks]);

  // Running and error state
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Keep ref in sync with state
  useEffect(() => {
    currentJobIdRef.current = currentJobId;
  }, [currentJobId]);

  // Reload this strategy's jobs from the local store. The store is not
  // reactive, so every job write below re-runs this.
  const refreshJobs = useCallback(async () => {
    try {
      setAllJobs(await listBacktestJobs(strategyId));
    } catch (e) {
      console.error('[useBacktestJob] Failed to load jobs:', e);
    }
  }, [strategyId]);

  useEffect(() => {
    refreshJobs();
  }, [refreshJobs]);

  const activeJobs = useMemo(
    () => allJobs.filter((j) => j.status === 'running' || j.status === 'pending'),
    [allJobs]
  );

  // Reset all state (when strategy changes)
  const resetState = useCallback(() => {
    setCurrentJobId(null);
    currentJobIdRef.current = null;
    hasAttemptedReconnect.current = false;
    setIsRunning(false);
    setError(null);
  }, []);

  // Reconnect to running job on mount, or mark stale jobs as failed
  // IMPORTANT: Only runs once on initial mount to avoid overwriting live progress with stale DB data
  useEffect(() => {
    if (hasAttemptedReconnect.current) return;
    if (activeJobs.length === 0) return;

    const runningJob = activeJobs.find(j => j.status === 'running' || j.status === 'pending');
    if (!runningJob) return;

    hasAttemptedReconnect.current = true;

    const now = Date.now();
    const jobAge = now - runningJob.updated_at;

    // If job is stale (no heartbeat in 60s), mark it as failed
    if (jobAge > STALE_JOB_THRESHOLD_MS) {
      console.log(`Marking stale job ${runningJob.id} as failed (last update ${Math.round(jobAge / 1000)}s ago)`);
      (async () => {
        try {
          await updateBacktestJob(runningJob.id, {
            status: 'failed',
            error_message: 'Job interrupted - app was closed or crashed during execution',
            completed_at: now,
            updated_at: now,
          });
          await refreshJobs();
        } catch (e) {
          console.error('Failed to mark stale job as failed:', e);
        }
      })();
      return; // Don't reconnect to stale job
    }

    // Job is fresh, reconnect to it
    setCurrentJobId(runningJob.id);
    setIsRunning(true);

    // Parse progress detail if available
    let progressDetail: TProgress | null = null;
    if (runningJob.progress_detail) {
      try {
        progressDetail = JSON.parse(runningJob.progress_detail) as TProgress;
      } catch {
        // Ignore parse errors
      }
    }

    // Notify consumer about reconnection
    if (callbacksRef.current.onReconnect) {
      callbacksRef.current.onReconnect(runningJob, progressDetail);
    }
  }, [activeJobs, refreshJobs]);

  // Listen for job heartbeat events
  // Uses ref for currentJobId to avoid re-creating listener when job ID changes
  //
  // BUG-044: Uses cancelled flag to prevent orphaned listeners.
  // The listen() call is async, so if the effect re-fires before it resolves,
  // the cleanup function would see unlisten as null and skip cleanup, orphaning
  // the listener. The cancelled flag ensures we clean up even in that case.
  // Callbacks are accessed via callbacksRef to avoid listener churn.
  useEffect(() => {
    let cancelled = false;
    let unlistenFn: UnlistenFn | null = null;

    listen<JobHeartbeat>('job-heartbeat', (event) => {
      if (event.payload.strategyId === strategyId) {
        if (!currentJobIdRef.current) {
          setCurrentJobId(event.payload.jobId);
        }
        callbacksRef.current.onProgress(event.payload.progressDetail as TProgress);
        setIsRunning(true);

        // Persist progress so the reconnect/stale-detection path (which reads
        // the local store) sees a live heartbeat. Fire-and-forget: the run
        // itself does not depend on this write.
        updateBacktestJob(event.payload.jobId, {
          status: 'running',
          progress_detail: event.payload.progressDetail
            ? JSON.stringify(event.payload.progressDetail)
            : null,
          updated_at: Date.now(),
        }).catch((e) => {
          console.error('Failed to persist job heartbeat:', e);
        });
      }
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlistenFn = fn;
      }
    });

    return () => {
      cancelled = true;
      if (unlistenFn) unlistenFn();
    };
  }, [strategyId]);

  // Listen for job completion events - update both React state AND the local store
  //
  // BUG-044: Same cancelled flag pattern as heartbeat listener above.
  // Callbacks accessed via callbacksRef to avoid listener churn.
  useEffect(() => {
    let cancelled = false;
    let unlistenFn: UnlistenFn | null = null;

    listen<JobCompleted>('job-completed', async (event) => {
      if (event.payload.strategyId !== strategyId) return;

      setIsRunning(false);

      const now = Date.now();
      const jobId = currentJobIdRef.current || event.payload.jobId;

      if (event.payload.status === 'completed' && event.payload.hasResult && event.payload.result) {
        const result = event.payload.result as TResult;
        callbacksRef.current.onComplete(result);

        // Update the local store with completed status and result
        if (jobId) {
          try {
            await updateBacktestJob(jobId, {
              status: 'completed',
              progress: 100,
              result: JSON.stringify(event.payload.result),
              completed_at: now,
              updated_at: now,
            });
            await refreshJobs();
          } catch (e) {
            console.error('Failed to update job as completed:', e);
          }
        }

        // Send success notification if provided
        if (callbacksRef.current.getSuccessNotification) {
          const notification = callbacksRef.current.getSuccessNotification(result);
          if (notification) {
            sendNotificationSafe(notification.title, notification.body);
          }
        }
      } else if (event.payload.status === 'failed') {
        const errorMsg = event.payload.error || 'Job failed';
        setError(errorMsg);
        callbacksRef.current.onError(errorMsg);

        // Update the local store with failed status and error
        if (jobId) {
          try {
            await updateBacktestJob(jobId, {
              status: 'failed',
              error_message: errorMsg,
              completed_at: now,
              updated_at: now,
            });
            await refreshJobs();
          } catch (e) {
            console.error('Failed to update job as failed:', e);
          }
        }

        // Send failure notification if provided
        if (callbacksRef.current.getFailureNotification) {
          const notification = callbacksRef.current.getFailureNotification(errorMsg);
          if (notification) {
            sendNotificationSafe(notification.title, notification.body);
          }
        }
      } else if (event.payload.status === 'cancelled') {
        // Update the local store with cancelled status
        if (jobId) {
          try {
            await updateBacktestJob(jobId, {
              status: 'cancelled',
              completed_at: now,
              updated_at: now,
            });
            await refreshJobs();
          } catch (e) {
            console.error('Failed to update job as cancelled:', e);
          }
        }
      }

      setCurrentJobId(null);
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlistenFn = fn;
      }
    });

    return () => {
      cancelled = true;
      if (unlistenFn) unlistenFn();
    };
  }, [strategyId, refreshJobs]);

  // Start a new job - creates DB record and sets up tracking
  const startJob = useCallback(async (jobType: string, params: Record<string, unknown>): Promise<string> => {
    const jobId = crypto.randomUUID();

    setIsRunning(true);
    setError(null);
    setCurrentJobId(jobId);

    try {
      const now = Date.now();
      await saveBacktestJob({
        id: jobId,
        strategy_id: strategyId,
        job_type: jobType,
        status: 'pending',
        params: JSON.stringify(params),
        progress: 0,
        progress_detail: null,
        result: null,
        error_message: null,
        created_at: now,
        updated_at: now,
        completed_at: null,
      });
      await refreshJobs();
    } catch (e) {
      console.error('Failed to create job record:', e);
      // Don't throw - job can still run without DB record
    }

    return jobId;
  }, [strategyId, refreshJobs]);

  // Signal that invoke finished (cleanup running state if events didn't handle it)
  const finishRunning = useCallback(() => {
    setIsRunning(false);
    setCurrentJobId(null);
  }, []);

  return {
    currentJobId,
    isRunning,
    error,
    activeJobs,
    allJobs,
    startJob,
    setError,
    finishRunning,
    resetState,
    currentJobIdRef,
  };
}

// Helper to send notifications safely
async function sendNotificationSafe(title: string, body: string): Promise<void> {
  try {
    const { isPermissionGranted, requestPermission, sendNotification } = await import('@tauri-apps/plugin-notification');

    let permissionGranted = await isPermissionGranted();
    if (!permissionGranted) {
      const permission = await requestPermission();
      permissionGranted = permission === 'granted';
    }

    if (permissionGranted) {
      sendNotification({ title, body });
    }
  } catch (e) {
    console.error('Failed to send notification:', e);
  }
}
