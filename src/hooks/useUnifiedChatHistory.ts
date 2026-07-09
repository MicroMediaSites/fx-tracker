/**
 * useUnifiedChatHistory - React hook for terminal chat history
 *
 * AGT-650: chat history moved from Zero (`chat_message` table + Clerk auth)
 * to local storage. The AI chat surface itself is retired with the cloud
 * proxy (`is_chat_enabled` is false), so this hook only preserves the existing
 * transcript UI: past messages render and can be cleared.
 *
 * AGT-669 (privacy): history is NO LONGER written to plaintext localStorage.
 * Messages carry full window/trading context (`windowContext`), and persisting
 * that unencrypted to localStorage leaked sensitive local-first state at rest.
 * Since the chat surface is gated off there is nothing durable worth keeping,
 * so history is now in-memory only (per window session) and any previously
 * persisted plaintext transcript is purged from localStorage on load.
 */

import { useState, useCallback, useMemo, useEffect } from 'react';

import type { ChatContext } from './useTerminalChat';

// Cap in-memory history so a long-lived window doesn't grow unbounded.
const MAX_STORED_MESSAGES = 50;

// Legacy key that previously held the plaintext transcript. Retained only so
// we can proactively delete any data left behind by earlier builds.
const LEGACY_STORAGE_KEY = 'wickd_chat_history';

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  windowType: string;
  windowContext: string;
  isCompaction: boolean;
  createdAt: number;
}

/**
 * Remove any plaintext transcript persisted by pre-AGT-669 builds. Called once
 * on mount so upgrading users don't keep leaking window/trading context at rest.
 */
function purgeLegacyStoredMessages(): void {
  try {
    localStorage.removeItem(LEGACY_STORAGE_KEY);
  } catch {
    // localStorage might be unavailable — nothing to purge.
  }
}

export interface UseUnifiedChatHistoryReturn {
  /** All messages (excluding compaction) sorted by created_at ascending */
  messages: ChatMessage[];
  /** The compaction message — always null (compaction retired with the AI proxy) */
  compaction: ChatMessage | null;
  /** Add a user message */
  addUserMessage: (content: string, context: ChatContext) => Promise<void>;
  /** Add an assistant message */
  addAssistantMessage: (content: string, context: ChatContext) => Promise<void>;
  /** Clear all chat history */
  clearHistory: () => Promise<void>;
  /** Refresh messages (call on focus) */
  syncMessages: () => void;
  /** Whether initial sync is in progress (never — storage is in-memory) */
  isLoading: boolean;
  /** Whether first sync has completed (always — storage is in-memory) */
  hasSynced: boolean;
  /** Build chat history for API */
  buildApiHistory: () => Array<{ role: 'user' | 'assistant'; content: string }>;
}

/**
 * Hook for unified chat history across all terminal windows
 */
export function useUnifiedChatHistory(): UseUnifiedChatHistoryReturn {
  const [stored, setStored] = useState<ChatMessage[]>([]);

  // On mount, delete any plaintext transcript persisted by older builds.
  useEffect(() => {
    purgeLegacyStoredMessages();
  }, []);

  const messages = useMemo(
    () => stored.filter((m) => !m.isCompaction).sort((a, b) => a.createdAt - b.createdAt),
    [stored]
  );

  const appendMessage = useCallback(
    (role: 'user' | 'assistant', content: string, context: ChatContext) => {
      const message: ChatMessage = {
        id: crypto.randomUUID(),
        role,
        content,
        windowType: context.type,
        windowContext: JSON.stringify(context),
        isCompaction: false,
        createdAt: Date.now(),
      };
      // In-memory only — never persisted to plaintext localStorage (AGT-669).
      setStored((prev) => [...prev, message].slice(-MAX_STORED_MESSAGES));
    },
    []
  );

  // Add a user message
  const addUserMessage = useCallback(
    async (content: string, context: ChatContext) => {
      appendMessage('user', content, context);
    },
    [appendMessage]
  );

  // Add an assistant message
  const addAssistantMessage = useCallback(
    async (content: string, context: ChatContext) => {
      appendMessage('assistant', content, context);
    },
    [appendMessage]
  );

  // Clear all history
  const clearHistory = useCallback(async () => {
    setStored([]);
    // Belt-and-suspenders: also drop any legacy persisted copy.
    purgeLegacyStoredMessages();
  }, []);

  // Sync messages (called on focus). History is in-memory and window-local now,
  // so there is nothing to re-read; kept as a no-op to preserve the hook API.
  const syncMessages = useCallback(() => {
    // no-op — history is no longer persisted or shared across windows.
  }, []);

  // Build API history
  const buildApiHistory = useCallback((): Array<{ role: 'user' | 'assistant'; content: string }> => {
    return messages.map((m) => ({ role: m.role, content: m.content }));
  }, [messages]);

  return {
    messages,
    compaction: null,
    addUserMessage,
    addAssistantMessage,
    clearHistory,
    syncMessages,
    isLoading: false,
    hasSynced: true,
    buildApiHistory,
  };
}
