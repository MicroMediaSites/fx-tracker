/**
 * Hook for managing streaming chat with the AI terminal.
 * Handles Tauri event listeners, message accumulation, and state management.
 */

import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

// Types matching Rust backend
export interface ChatTokenEvent {
  session_id: string;
  text: string;
}

export interface ChatCompleteEvent {
  session_id: string;
  full_text: string;
  input_tokens: number;
  output_tokens: number;
  /** Stop reason: "end_turn" (normal), "max_tokens" (truncated), etc. */
  stop_reason: string | null;
}

export interface ChatErrorEvent {
  session_id: string;
  error_type: string;
  message: string;
}

export interface ChatMessage {
  role: 'user' | 'assistant';
  content: string;
}

export interface ChatContext {
  type: string;
  [key: string]: unknown;
}

interface UseTerminalChatOptions {
  /** Unique session ID for this chat instance */
  sessionId: string;
  /** Called when a new token is received */
  onToken?: (text: string) => void;
  /** Called when the stream completes */
  onComplete?: (fullText: string, usage: { input: number; output: number }, stopReason: string | null) => void;
  /** Called on error */
  onError?: (error: { type: string; message: string }) => void;
}

export interface UserContext {
  userId: string;
  tier: string;
}

interface UseTerminalChatReturn {
  /** Whether a stream is currently active */
  isStreaming: boolean;
  /** Current accumulated response text (during streaming) */
  currentResponse: string;
  /** Send a message to the AI */
  sendMessage: (
    message: string,
    context: ChatContext,
    history: ChatMessage[],
    model?: string
  ) => Promise<void>;
  /** Cancel the current stream */
  cancel: () => Promise<void>;
  /** Check if AI is enabled */
  isEnabled: () => Promise<boolean>;
}

export function useTerminalChat(options: UseTerminalChatOptions): UseTerminalChatReturn {
  const { sessionId, onToken, onComplete, onError } = options;

  const [isStreaming, setIsStreaming] = useState(false);
  const [currentResponse, setCurrentResponse] = useState('');
  const accumulatedTextRef = useRef('');
  const unlistenersRef = useRef<UnlistenFn[]>([]);

  // Store callbacks in refs to avoid re-creating listeners on every render
  const onTokenRef = useRef(onToken);
  const onCompleteRef = useRef(onComplete);
  const onErrorRef = useRef(onError);

  // Keep refs updated
  useEffect(() => {
    onTokenRef.current = onToken;
    onCompleteRef.current = onComplete;
    onErrorRef.current = onError;
  }, [onToken, onComplete, onError]);

  // Set up event listeners - only depends on sessionId now
  useEffect(() => {
    let isMounted = true;

    const setupListeners = async () => {
      // Clean up any existing listeners first
      for (const unlisten of unlistenersRef.current) {
        unlisten();
      }
      unlistenersRef.current = [];

      // Don't set up if unmounted during async gap
      if (!isMounted) return;

      // Listen for tokens
      const unlistenToken = await listen<ChatTokenEvent>('chat-token', (event) => {
        if (!isMounted || event.payload.session_id !== sessionId) return;

        accumulatedTextRef.current += event.payload.text;
        setCurrentResponse(accumulatedTextRef.current);
        onTokenRef.current?.(event.payload.text);
      });
      if (isMounted) unlistenersRef.current.push(unlistenToken);

      // Listen for completion
      const unlistenComplete = await listen<ChatCompleteEvent>('chat-complete', (event) => {
        if (!isMounted || event.payload.session_id !== sessionId) return;

        setIsStreaming(false);
        setCurrentResponse('');
        accumulatedTextRef.current = '';

        onCompleteRef.current?.(
          event.payload.full_text,
          {
            input: event.payload.input_tokens,
            output: event.payload.output_tokens,
          },
          event.payload.stop_reason
        );
      });
      if (isMounted) unlistenersRef.current.push(unlistenComplete);

      // Listen for errors
      const unlistenError = await listen<ChatErrorEvent>('chat-error', (event) => {
        if (!isMounted || event.payload.session_id !== sessionId) return;

        setIsStreaming(false);
        setCurrentResponse('');
        accumulatedTextRef.current = '';

        onErrorRef.current?.({
          type: event.payload.error_type,
          message: event.payload.message,
        });
      });
      if (isMounted) unlistenersRef.current.push(unlistenError);
    };

    setupListeners();

    return () => {
      isMounted = false;
      for (const unlisten of unlistenersRef.current) {
        unlisten();
      }
      unlistenersRef.current = [];
    };
  }, [sessionId]);

  // Send a message
  const sendMessage = useCallback(
    async (
      message: string,
      context: ChatContext,
      history: ChatMessage[],
      model?: string
    ) => {
      if (isStreaming) {
        throw new Error('Already streaming');
      }

      setIsStreaming(true);
      accumulatedTextRef.current = '';
      setCurrentResponse('');

      try {
        await invoke('chat_stream', {
          sessionId,
          context,
          message,
          history: history.map((m) => ({
            role: m.role,
            content: m.content,
          })),
          model,
        });
      } catch (error) {
        setIsStreaming(false);
        throw error;
      }
    },
    [sessionId, isStreaming]
  );

  // Cancel the current stream
  const cancel = useCallback(async () => {
    if (!isStreaming) return;

    try {
      await invoke('chat_cancel', { sessionId });
    } catch {
      // Ignore errors - stream may have already ended
    }
  }, [sessionId, isStreaming]);

  // Check if AI is enabled
  const isEnabled = useCallback(async () => {
    try {
      return await invoke<boolean>('is_chat_enabled');
    } catch {
      return false;
    }
  }, []);

  return {
    isStreaming,
    currentResponse,
    sendMessage,
    cancel,
    isEnabled,
  };
}

/**
 * Generate a unique session ID for a chat instance.
 * Call this once per terminal instance.
 */
export function generateSessionId(windowType: string): string {
  return `terminal-${windowType}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}
