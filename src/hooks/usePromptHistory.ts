/**
 * usePromptHistory - React hook for prompt history (up-arrow cycling)
 *
 * AGT-650: prompt history moved from Zero (`prompt_history` table + Clerk
 * auth) to plain localStorage — it is input-convenience state, not domain
 * data, so it doesn't warrant a local-store table. Keeps the last 100
 * prompts per window origin, persisted across sessions.
 */

import { useState, useCallback, useEffect } from 'react';

// Limits
const MAX_PROMPT_HISTORY = 100;

const STORAGE_KEY = 'wickd_prompt_history';

function readStoredPrompts(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      return parsed.filter((p): p is string => typeof p === 'string');
    }
  } catch {
    // Ignore parse errors — treat as empty history
  }
  return [];
}

function writeStoredPrompts(prompts: string[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(prompts.slice(0, MAX_PROMPT_HISTORY)));
  } catch {
    // localStorage might be unavailable — history just won't persist
  }
}

export interface UsePromptHistoryReturn {
  /** All prompts sorted newest first */
  prompts: string[];
  /** Add a new prompt */
  addPrompt: (content: string) => Promise<void>;
  /** Navigate up in history (older prompts) */
  navigateUp: () => string | null;
  /** Navigate down in history (newer prompts) */
  navigateDown: () => string | null;
  /** Reset navigation index (call when user types) */
  resetNavigation: () => void;
  /** Current navigation index (-1 = not navigating) */
  navigationIndex: number;
  /** Clear all prompt history */
  clearHistory: () => Promise<void>;
}

/**
 * Hook for prompt history navigation (up-arrow cycling)
 */
export function usePromptHistory(): UsePromptHistoryReturn {
  // Navigation state
  const [navigationIndex, setNavigationIndex] = useState(-1);
  const [prompts, setPrompts] = useState<string[]>(() => readStoredPrompts());

  // Reset navigation when prompts change
  useEffect(() => {
    setNavigationIndex(-1);
  }, [prompts.length]);

  // Add a new prompt
  const addPrompt = useCallback(
    async (content: string) => {
      const trimmed = content.trim();
      if (!trimmed) return;

      setPrompts((prev) => {
        // Don't add duplicates (if the same as last prompt)
        if (prev.length > 0 && prev[0] === trimmed) return prev;
        const next = [trimmed, ...prev].slice(0, MAX_PROMPT_HISTORY);
        writeStoredPrompts(next);
        return next;
      });

      // Reset navigation
      setNavigationIndex(-1);
    },
    []
  );

  // Navigate up (older prompts)
  const navigateUp = useCallback((): string | null => {
    if (prompts.length === 0) return null;

    const newIndex = Math.min(navigationIndex + 1, prompts.length - 1);
    setNavigationIndex(newIndex);
    return prompts[newIndex] ?? null;
  }, [prompts, navigationIndex]);

  // Navigate down (newer prompts)
  const navigateDown = useCallback((): string | null => {
    if (navigationIndex <= 0) {
      setNavigationIndex(-1);
      return ''; // Return empty to clear input
    }

    const newIndex = navigationIndex - 1;
    setNavigationIndex(newIndex);
    return prompts[newIndex] ?? null;
  }, [prompts, navigationIndex]);

  // Reset navigation (call when user types)
  const resetNavigation = useCallback(() => {
    setNavigationIndex(-1);
  }, []);

  // Clear all history
  const clearHistory = useCallback(async () => {
    setPrompts([]);
    writeStoredPrompts([]);
    setNavigationIndex(-1);
  }, []);

  return {
    prompts,
    addPrompt,
    navigateUp,
    navigateDown,
    resetNavigation,
    navigationIndex,
    clearHistory,
  };
}
