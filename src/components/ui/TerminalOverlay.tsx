import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  useTerminalChat,
  generateSessionId,
  type ChatMessage,
  type ChatContext,
} from '../../hooks/useTerminalChat';
import { buildMinimalContext } from '../../lib/chatContextBuilder';
import { buildContextForCategory } from '../../lib/contextLoader';
import { useUnifiedChatHistory } from '../../hooks/useUnifiedChatHistory';
import { usePromptHistory } from '../../hooks/usePromptHistory';
import { ConfirmModal } from './ConfirmModal';
import { AI_DISCLAIMER_TEXT, AI_COMPLIANCE_PATTERNS } from '../../constants';
import type { SuggestedPrompt } from '../../lib/suggestedPrompts';

/** Max characters for user prompt (~500 tokens) */
const MAX_PROMPT_LENGTH = 2000;

interface TerminalOverlayProps {
  height: number;
  currentWindow: string;
  /** Optional function to build context for AI. Falls back to minimal context. */
  contextProvider?: () => ChatContext;
  /** Called when user sends a message - parent can expand the terminal */
  onRequestExpand?: () => void;
  /** Header shown in empty state (cyan) */
  header?: string;
  /** Description shown below header (gray) */
  headerDescription?: string;
  /** Welcome content lines shown when empty - supports "Try:" prompts and "I can see:" context */
  welcomeContent?: string[];
  /** Contextual suggested prompts shown as clickable chips below welcome content */
  suggestedPrompts?: SuggestedPrompt[];
}

/**
 * AI-powered terminal overlay with streaming responses.
 * Height controlled by parent via drag handle.
 */
export const TerminalOverlay = ({
  height,
  currentWindow,
  contextProvider,
  onRequestExpand,
  header,
  headerDescription,
  welcomeContent,
  suggestedPrompts,
}: TerminalOverlayProps) => {
  const [input, setInput] = useState('');
  const [aiEnabled, setAiEnabled] = useState<boolean | null>(null);
  const [isStreamingActive, setIsStreamingActive] = useState(false);
  // Track user message count for suggested prompt chip dismissal
  const [messageCount, setMessageCount] = useState(0);
  const [showClearConfirm, setShowClearConfirm] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const outputRef = useRef<HTMLDivElement>(null);

  // Unified chat history (Zero-synced across all windows)
  const {
    messages: unifiedMessages,
    addUserMessage,
    addAssistantMessage,
    clearHistory,
    syncMessages,
    isLoading: chatLoading,
    hasSynced,
    buildApiHistory,
  } = useUnifiedChatHistory();

  // Prompt history for up-arrow cycling (Zero-synced)
  const {
    addPrompt,
    navigateUp,
    navigateDown,
    resetNavigation,
    navigationIndex,
  } = usePromptHistory();

  const [savedInput, setSavedInput] = useState(''); // Save current input when navigating

  // Generate session ID once per window type
  const sessionId = useMemo(() => generateSessionId(currentWindow), [currentWindow]);

  // Sync messages on window focus
  useEffect(() => {
    const handleFocus = () => syncMessages();
    window.addEventListener('focus', handleFocus);
    return () => window.removeEventListener('focus', handleFocus);
  }, [syncMessages]);

  // Build chat history for the API (includes compaction context)
  const chatHistory = useMemo((): ChatMessage[] => {
    return buildApiHistory();
  }, [buildApiHistory]);

  // Scroll helper - uses double rAF to ensure DOM is fully updated
  const scrollToBottom = useCallback((smooth = false) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        outputRef.current?.scrollTo({
          top: outputRef.current.scrollHeight,
          behavior: smooth ? 'smooth' : 'auto',
        });
      });
    });
  }, []);

  // Get current context for messages
  const getCurrentContext = useCallback(() => {
    return contextProvider?.() ?? buildMinimalContext(currentWindow);
  }, [contextProvider, currentWindow]);

  // Strip bulky AI-only fields from context before storing in chat_message.
  // These fields are only needed in the system prompt, not per-message history.
  const stripBulkyContextFields = useCallback((context: ChatContext): ChatContext => {
    const { script_content: _, strategy_rules: _1, parameter_definitions: _2, window_summary: _3, selected_window: _4, ...light } = context as Record<string, unknown>;
    return light as ChatContext;
  }, []);

  // Streaming chat hook
  const { isStreaming, currentResponse, sendMessage, cancel, isEnabled } = useTerminalChat({
    sessionId,
    onToken: () => {
      // Small delay to let React render the new content before scrolling
      setTimeout(() => {
        outputRef.current?.scrollTo({
          top: outputRef.current.scrollHeight,
          behavior: 'auto',
        });
      }, 10);
    },
    onComplete: async (fullText, usage, stopReason) => {
      // Save assistant response to unified history
      const context = getCurrentContext();
      const contextForHistory = stripBulkyContextFields(context);

      // If truncated due to max_tokens, append a warning
      let finalText = fullText;
      if (stopReason === 'max_tokens') {
        finalText += '\n\n⚠️ [Response was truncated due to length limit. Try asking a more specific question.]';
      }

      addAssistantMessage(finalText, contextForHistory as typeof context);
      setIsStreamingActive(false);
      scrollToBottom();

      // Refocus input after streaming completes (explicit, in addition to effect-based refocus)
      requestAnimationFrame(() => {
        inputRef.current?.focus();
      });

      // AGT-650: cloud token-usage metering removed with the queries-service.
      void usage;
    },
    onError: (error) => {
      // Show error to user (not persisted to history)
      console.error('[TerminalOverlay] Chat error:', error.message);

      setErrorMessage(error.message);
      setIsStreamingActive(false);

      // Refocus input after error (explicit, in addition to effect-based refocus)
      requestAnimationFrame(() => {
        inputRef.current?.focus();
      });
    },
  });

  // Check if AI is enabled on mount
  useEffect(() => {
    isEnabled().then(setAiEnabled);
  }, [isEnabled]);

  // Listen for strategy promotion success events from AI-initiated promotions.
  // Show as a transient notification, NOT persisted to chat history.
  // Tauri events broadcast to all windows — persisting would create duplicates
  // across every open window and flood the chat history with non-AI messages.
  const [promotionNotice, setPromotionNotice] = useState<string | null>(null);
  const promotionTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    const unlisten = listen<{ strategy_id: string; strategy_name: string }>(
      'strategy-promoted',
      (event) => {
        // Clear any existing timer so a new event resets the countdown
        if (promotionTimerRef.current) clearTimeout(promotionTimerRef.current);
        setPromotionNotice(`Strategy "${event.payload.strategy_name}" is now live for trading.`);
        scrollToBottom();
        promotionTimerRef.current = setTimeout(() => setPromotionNotice(null), 10000);
      }
    );

    return () => {
      unlisten.then(fn => fn());
      if (promotionTimerRef.current) clearTimeout(promotionTimerRef.current);
    };
  }, [scrollToBottom]);

  // Helper to display usage report (AGT-650: metering removed with the cloud tier)
  const displayUsageReport = useCallback(async () => {
    const lines: string[] = [
      'AI Usage Report',
      '───────────────────────────────',
      'Token metering was retired with the cloud tier (AGT-650).',
      'AI availability now depends only on the local AI configuration.',
      '───────────────────────────────',
    ];
    const context = getCurrentContext();
    addAssistantMessage(lines.join('\n'), stripBulkyContextFields(context));
    scrollToBottom();
  }, [getCurrentContext, addAssistantMessage, stripBulkyContextFields, scrollToBottom]);

  const handleSubmit = async () => {
    if (!input.trim() || isStreaming) return;

    // Clear any previous error
    setErrorMessage(null);

    const userInput = input.trim();

    // Handle slash commands
    if (userInput === '/clear') {
      setInput('');
      // Check if user opted out of confirmation
      const skipConfirm = localStorage.getItem('candlesight_skip_clear_confirm') === 'true';
      if (skipConfirm) {
        clearHistory();
        resetNavigation();
        setSavedInput('');
      } else {
        setShowClearConfirm(true);
      }
      return;
    }

    // Handle /usage command
    if (userInput === '/usage') {
      setInput('');
      await displayUsageReport();
      return;
    }

    // AGT-650: quota checks removed with the cloud tier — the backend
    // command reports clearly if AI is unavailable.

    // Add to prompt history for up/down navigation
    addPrompt(userInput);
    resetNavigation();
    setSavedInput('');
    setMessageCount(prev => prev + 1);

    // Build full context from window state
    const fullContext = getCurrentContext();

    // Save user input to unified history (bulky AI-only fields stripped)
    await addUserMessage(userInput, stripBulkyContextFields(fullContext));
    setInput('');
    setIsStreamingActive(true);

    // Request expansion if at minimal height
    onRequestExpand?.();

    // Scroll to bottom
    scrollToBottom();

    // Reclaim focus immediately after send - the textarea gets disabled during streaming
    // which causes the browser to blur it. We re-focus so it's ready when streaming ends.
    // Even though it's disabled, keeping focus prevents focus from drifting elsewhere.
    requestAnimationFrame(() => {
      inputRef.current?.focus();
    });

    try {
      // AGT-650: the cloud prompt classifier is gone — send the full window
      // context (the classifier's 'general' fallback behavior).
      const context = buildContextForCategory(fullContext, {
        primary: 'general',
        secondary: null,
        confidence: 'high',
        reasoning: 'cloud classifier retired (AGT-650); defaulting to general',
        source: 'heuristic',
      });

      await sendMessage(userInput, context, chatHistory);
    } catch (error) {
      console.error('[TerminalOverlay] Send error:', error);
      setErrorMessage(error instanceof Error ? error.message : String(error));
      setIsStreamingActive(false);

      // Refocus input after send error
      requestAnimationFrame(() => {
        inputRef.current?.focus();
      });
    }
  };

  // Handle Escape key to cancel streaming
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Check both hook state and local state - they can get out of sync
      if (e.key === 'Escape' && (isStreaming || isStreamingActive)) {
        cancel();
        setIsStreamingActive(false); // Force reset local state
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isStreaming, isStreamingActive, cancel]);

  // Ref for the terminal container - used to scope focus management
  const terminalRef = useRef<HTMLDivElement>(null);

  // Aggressive focus management - refocus input after any focus loss while terminal is open.
  // This prevents focus from being stolen by streaming state changes, scroll operations,
  // DOM updates during token rendering, or other React re-renders.
  const focusInput = useCallback(() => {
    // Don't steal focus if user is selecting text
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed) return;
    // Don't focus if a modal is open (e.g., clear confirmation)
    if (showClearConfirm) return;
    inputRef.current?.focus();
  }, [showClearConfirm]);

  // Refocus input when streaming ends (isStreaming transitions from true to false).
  // When the textarea is disabled={isStreaming}, the browser removes focus.
  // We need to reclaim it once the textarea is re-enabled.
  const prevStreamingRef = useRef(false);
  useEffect(() => {
    if (prevStreamingRef.current && !isStreaming && height > 0) {
      // Streaming just ended - textarea was re-enabled, reclaim focus
      // Use rAF to ensure the DOM has updated the disabled attribute
      requestAnimationFrame(() => {
        focusInput();
      });
    }
    prevStreamingRef.current = isStreaming;
  }, [isStreaming, height, focusInput]);

  // Also refocus when isStreamingActive ends (covers error paths and local state resets)
  const prevStreamingActiveRef = useRef(false);
  useEffect(() => {
    if (prevStreamingActiveRef.current && !isStreamingActive && height > 0) {
      requestAnimationFrame(() => {
        focusInput();
      });
    }
    prevStreamingActiveRef.current = isStreamingActive;
  }, [isStreamingActive, height, focusInput]);

  // Periodic focus guardian - reclaims focus if it drifts away while terminal is open.
  // Checks every 500ms. This catches edge cases like scroll-induced blur,
  // portal re-renders, or other subtle focus theft that individual effects miss.
  useEffect(() => {
    if (height <= 0) return;

    const interval = setInterval(() => {
      const active = document.activeElement;
      const input = inputRef.current;
      const terminal = terminalRef.current;
      if (!input) return;
      // Already focused - nothing to do
      if (active === input) return;
      // Focus drifted to body/document/null - always reclaim
      if (active === document.body || active === null || active === document.documentElement) {
        focusInput();
        return;
      }
      // Focus is on a non-interactive element inside the terminal (e.g., output div,
      // span after clicking). Reclaim it, but don't steal from buttons/links/inputs
      // inside the terminal (e.g., upgrade link, trial notice).
      if (terminal && active && terminal.contains(active)) {
        const tag = (active as HTMLElement).tagName?.toLowerCase();
        const isInteractive = tag === 'button' || tag === 'a' || tag === 'input' || tag === 'select' || tag === 'textarea';
        if (!isInteractive) {
          focusInput();
        }
      }
    }, 500);

    return () => clearInterval(interval);
  }, [height, focusInput]);

  // Auto-focus input and scroll to bottom when terminal opens
  const prevHeightRef = useRef(0);
  const didInitialScroll = useRef(false);
  useEffect(() => {
    if (height > 0 && prevHeightRef.current === 0) {
      // Terminal just opened - focus input and scroll to bottom after DOM settles
      didInitialScroll.current = false; // Reset so we scroll when messages load
      setTimeout(() => {
        focusInput();
        scrollToBottom();
      }, 50);
    } else if (height === 0) {
      // Terminal closed - reset scroll flag
      didInitialScroll.current = false;
    }
    prevHeightRef.current = height;
  }, [height, scrollToBottom, focusInput]);

  // Scroll to bottom when messages first load
  useEffect(() => {
    if (hasSynced && height > 0 && !didInitialScroll.current && unifiedMessages.length > 0) {
      didInitialScroll.current = true;
      scrollToBottom();
    }
  }, [hasSynced, height, unifiedMessages.length, scrollToBottom]);

  // Auto-scroll during streaming - runs AFTER React renders the new content
  useEffect(() => {
    if (isStreamingActive && currentResponse) {
      // Use instant scroll (not smooth) to keep up with fast token updates
      scrollToBottom(false);
    }
  }, [isStreamingActive, currentResponse, scrollToBottom]);

  // Safety timeout - reset stuck streaming state after 60s of no activity
  const lastResponseRef = useRef(currentResponse);
  useEffect(() => {
    if (!isStreamingActive) return;

    lastResponseRef.current = currentResponse;
    const timeout = setTimeout(() => {
      // If response hasn't changed in 60s, assume stuck
      if (isStreamingActive && lastResponseRef.current === currentResponse) {
        console.warn('[TerminalOverlay] Streaming timeout - resetting stuck state');
        setIsStreamingActive(false);
        setErrorMessage('Response timed out. Please try again.');
      }
    }, 60000);

    return () => clearTimeout(timeout);
  }, [isStreamingActive, currentResponse]);

  // On mouseup in the output area, reclaim focus after a brief delay.
  // The delay allows the selection to settle - if the user selected text we won't steal focus,
  // but if they just clicked (no selection), we reclaim it for the prompt.
  // NOTE: Must be before early returns to maintain consistent hook count.
  const handleOutputMouseUp = useCallback(() => {
    setTimeout(() => {
      focusInput();
    }, 50);
  }, [focusInput]);

  // Track when input was set by a chip click to auto-submit
  const pendingChipSubmitRef = useRef<string | null>(null);

  // Keep a ref to the latest handleSubmit to avoid stale closures in effects.
  // handleSubmit closes over input, messages, quota state, etc. — calling a stale
  // version can submit with outdated state. The ref is updated on every render so
  // the effect always invokes the current version.
  const handleSubmitRef = useRef(handleSubmit);
  handleSubmitRef.current = handleSubmit;

  // Auto-submit after chip sets the input value
  useEffect(() => {
    if (pendingChipSubmitRef.current && input === pendingChipSubmitRef.current && !isStreaming) {
      pendingChipSubmitRef.current = null;
      handleSubmitRef.current();
    }
  }, [input, isStreaming]);

  // Refined chip click: set pending ref, then set input
  const onChipClick = useCallback((prompt: string) => {
    if (isStreaming) return;
    pendingChipSubmitRef.current = prompt;
    setInput(prompt);
  }, [isStreaming]);

  // Whether to show suggested prompt chips
  const showSuggestedPrompts = suggestedPrompts && suggestedPrompts.length > 0 && messageCount < 2 && !isStreamingActive;

  if (height === 0) return null;

  const INPUT_HEIGHT = 28;
  const outputHeight = Math.max(0, height - INPUT_HEIGHT);

  // AI not available message
  if (aiEnabled === false) {
    return (
      <div className="h-full flex flex-col bg-transparent font-mono text-xs">
        <div className="flex-1 flex items-center justify-center text-gray-500">
          AI features are temporarily unavailable. Please try again later.
        </div>
      </div>
    );
  }

  // Loading state while syncing chat history
  if (chatLoading && !hasSynced) {
    return (
      <div className="h-full flex flex-col bg-transparent font-mono text-xs">
        <div className="flex-1 flex items-center justify-center">
          <span className="text-gray-500 animate-pulse">loading...</span>
        </div>
      </div>
    );
  }

  // Focus input when clicking anywhere in the terminal (unless selecting text)
  const handleTerminalClick = () => {
    focusInput();
  };

  return (
    <div
      ref={terminalRef}
      className="h-full flex flex-col bg-transparent font-mono text-xs relative"
      onClick={handleTerminalClick}
    >
      {/* Output area - always rendered to avoid layout shift, height controlled via flex */}
      <div
        ref={outputRef}
        className={`overflow-y-auto flex-1 min-h-0 select-text ${outputHeight > 0 ? '' : 'hidden'}`}
        onMouseUp={handleOutputMouseUp}
      >
        <div className="p-3 space-y-0.5 select-text">
          {/* Empty state - comment style */}
          {unifiedMessages.length === 0 && !isStreamingActive && (
            <div className="space-y-0.5">
              {header && <p className="text-cyan-400"># {header}</p>}
              {headerDescription && <p className="text-gray-600"># {headerDescription}</p>}
              {(header || headerDescription) && <p className="text-gray-700">#</p>}
              {welcomeContent && welcomeContent.map((line, i) => (
                <p key={i} className={line === '' ? 'text-gray-700' : 'text-gray-600'}>
                  {line === '' ? '#' : `# ${line}`}
                </p>
              ))}
              {!header && !headerDescription && !welcomeContent && (
                <>
                  <p className="text-cyan-400"># Ask AI anything</p>
                  <p className="text-gray-600"># I have context about this window</p>
                </>
              )}
            </div>
          )}
          {/* Suggested prompt chips - shown when few messages sent */}
          {showSuggestedPrompts && (
            <div className="flex flex-wrap gap-2 mt-2 mb-3">
              {suggestedPrompts.map((sp, i) => (
                <button
                  key={i}
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    onChipClick(sp.prompt);
                  }}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-full bg-[var(--color-bg-card,#1a1f2e)] border border-[var(--color-border,#2d3748)] hover:border-[var(--color-info,#60a5fa)] text-[var(--color-text-secondary,#9ca3af)] hover:text-[var(--color-text-primary,#e5e7eb)] transition-colors duration-150 cursor-pointer"
                >
                  <svg width="12" height="12" viewBox="0 0 16 16" fill="none" className="flex-shrink-0 opacity-60">
                    <path d="M8 1l1.5 4.5L14 7l-4.5 1.5L8 13l-1.5-4.5L2 7l4.5-1.5L8 1z" fill="currentColor" />
                  </svg>
                  {sp.label}
                </button>
              ))}
            </div>
          )}
          {/* Chat messages */}
          {unifiedMessages.map((msg) => {
            // Render content with compliance sentence and disclaimer highlighting for AI responses
            const isAssistant = msg.role === 'assistant';
            const renderContent = (content: string) => {
              if (!isAssistant) {
                return <span className="whitespace-pre-wrap break-words">{content}</span>;
              }

              // Find sentence containing a compliance pattern (usually first sentence)
              // Split on sentence boundaries (. followed by space or newline)
              const sentences = content.split(/(?<=\.)\s+/);
              let complianceSentence: string | null = null;
              let complianceSentenceIdx = -1;

              for (let i = 0; i < sentences.length; i++) {
                const sentenceLower = sentences[i].toLowerCase();
                if (AI_COMPLIANCE_PATTERNS.some(pattern => sentenceLower.includes(pattern.toLowerCase()))) {
                  complianceSentence = sentences[i];
                  complianceSentenceIdx = i;
                  break;
                }
              }

              // Check for disclaimer (usually at end)
              const hasDisclaimer = content.includes(AI_DISCLAIMER_TEXT);

              // If neither, return plain content
              if (!complianceSentence && !hasDisclaimer) {
                return <span className="whitespace-pre-wrap break-words">{content}</span>;
              }

              // Build content with highlighted sections
              const elements: React.ReactNode[] = [];

              // Handle compliance sentence (highlight the whole sentence)
              if (complianceSentence && complianceSentenceIdx >= 0) {
                // Add sentences before compliance
                if (complianceSentenceIdx > 0) {
                  elements.push(sentences.slice(0, complianceSentenceIdx).join(' ') + ' ');
                }
                // Add highlighted compliance sentence
                elements.push(
                  <span key="compliance" className="text-amber-400/80 italic">
                    {complianceSentence}
                  </span>
                );
                // Remaining content after compliance sentence
                const afterCompliance = sentences.slice(complianceSentenceIdx + 1).join(' ');
                if (afterCompliance) {
                  // Check if disclaimer is in the remaining content
                  if (hasDisclaimer && afterCompliance.includes(AI_DISCLAIMER_TEXT)) {
                    const disclaimerIdx = afterCompliance.indexOf(AI_DISCLAIMER_TEXT);
                    const beforeDisclaimer = afterCompliance.substring(0, disclaimerIdx).trimEnd();
                    elements.push(' ' + beforeDisclaimer);
                    elements.push(beforeDisclaimer ? '\n\n' : '');
                    elements.push(
                      <span key="disclaimer" className="text-amber-400/80 italic">
                        {AI_DISCLAIMER_TEXT}
                      </span>
                    );
                  } else {
                    elements.push(' ' + afterCompliance);
                  }
                }
              } else if (hasDisclaimer) {
                // No compliance sentence, just disclaimer
                const disclaimerIdx = content.indexOf(AI_DISCLAIMER_TEXT);
                const beforeDisclaimer = content.substring(0, disclaimerIdx).trimEnd();
                elements.push(beforeDisclaimer);
                elements.push(beforeDisclaimer ? '\n\n' : '');
                elements.push(
                  <span key="disclaimer" className="text-amber-400/80 italic">
                    {AI_DISCLAIMER_TEXT}
                  </span>
                );
              }

              return <span className="whitespace-pre-wrap break-words">{elements}</span>;
            };

            return (
              <div
                key={msg.id}
                className={`flex gap-2 ${isAssistant ? 'mb-4' : 'mb-1'} ${
                  msg.role === 'user' ? 'text-gray-100' : 'text-emerald-300'
                }`}
              >
                <span className={`flex-shrink-0 ${
                  msg.role === 'user' ? 'text-cyan-400' : 'text-emerald-300'
                }`}>
                  {msg.role === 'user' ? '>' : '←'}
                </span>
                {renderContent(msg.content)}
              </div>
            );
          })}
          {/* Streaming response (not persisted until complete) */}
          {isStreamingActive && (
            <div className="flex gap-2 mb-4 text-emerald-300">
              <span className="flex-shrink-0 text-emerald-300">←</span>
              <span className="whitespace-pre-wrap break-words">
                {currentResponse || <span className="text-gray-500 animate-pulse">thinking...</span>}
              </span>
            </div>
          )}
          {/* Error message */}
          {errorMessage && !isStreamingActive && (
            <div className="flex gap-2 mb-4 text-amber-400">
              <span className="flex-shrink-0">⚠</span>
              <span className="whitespace-pre-wrap break-words">{errorMessage}</span>
            </div>
          )}
          {/* Transient promotion notice (not persisted to chat history) */}
          {promotionNotice && (
            <div className="flex gap-2 mb-4 text-cyan-300">
              <span className="flex-shrink-0">←</span>
              <span className="whitespace-pre-wrap break-words">{promotionNotice}</span>
            </div>
          )}
        </div>
      </div>

      {/* Input area - at BOTTOM, immediately above the drag handle */}
      <div className="flex items-start px-3 py-1.5 border-t border-gray-800 bg-transparent flex-shrink-0">
        <span className={`mr-2 leading-[1.4] ${isStreaming ? 'text-gray-500' : 'text-cyan-400'}`}>{'>'}</span>
        <textarea
          ref={inputRef}
          value={input}
          onChange={(e) => {
            setInput(e.target.value);
            // Auto-resize
            if (inputRef.current) {
              inputRef.current.style.height = 'auto';
              inputRef.current.style.height = Math.min(inputRef.current.scrollHeight, 120) + 'px';
            }
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              handleSubmit();
              // Reset height after submit
              if (inputRef.current) {
                inputRef.current.style.height = 'auto';
              }
            } else if (e.key === 'ArrowUp') {
              const textarea = inputRef.current;
              if (!textarea) return;

              const cursorPos = textarea.selectionStart;
              // Find if cursor is on the first line
              const textBeforeCursor = textarea.value.substring(0, cursorPos);
              const isOnFirstLine = !textBeforeCursor.includes('\n');

              if (!isOnFirstLine) {
                // Multiline input, cursor not on first line — let default arrow behavior work
                return;
              }

              if (cursorPos > 0 && navigationIndex === -1) {
                // On first line but not at position 0 — move cursor to start
                e.preventDefault();
                textarea.setSelectionRange(0, 0);
                return;
              }

              // Cursor is at position 0 (or already navigating history) — go to previous prompt
              e.preventDefault();
              if (navigationIndex === -1) {
                setSavedInput(input);
              }
              const prompt = navigateUp();
              if (prompt !== null) {
                setInput(prompt);
              }
            } else if (e.key === 'ArrowDown') {
              const textarea = inputRef.current;
              if (!textarea) return;

              const cursorPos = textarea.selectionStart;
              const textAfterCursor = textarea.value.substring(cursorPos);
              const isOnLastLine = !textAfterCursor.includes('\n');

              if (!isOnLastLine) {
                // Multiline input, cursor not on last line — let default arrow behavior work
                return;
              }

              if (navigationIndex === -1) {
                // Not navigating history — let default behavior work
                return;
              }

              // Navigating history — go to next prompt
              e.preventDefault();
              const prompt = navigateDown();
              if (prompt !== null) {
                setInput(prompt === '' ? savedInput : prompt);
              }
            }
          }}
          placeholder={
            isStreaming
              ? 'thinking... (Esc to cancel)'
              : 'ask me anything...'
          }
          disabled={isStreaming}
          maxLength={MAX_PROMPT_LENGTH}
          rows={1}
          className="flex-1 bg-transparent border-none outline-none text-gray-100 placeholder-gray-600 disabled:opacity-50 resize-none leading-[1.4] overflow-y-auto"
          autoComplete="off"
        />
      </div>

      {/* Clear history confirmation */}
      <ConfirmModal
        isOpen={showClearConfirm}
        title="Clear AI Chat History?"
        message="This will permanently delete all chat history across all windows. The AI will lose context from previous conversations."
        confirmLabel="Clear History"
        cancelLabel="Keep History"
        variant="warning"
        showDontAskAgain
        onDontAskAgainChange={(checked) => {
          if (checked) {
            localStorage.setItem('candlesight_skip_clear_confirm', 'true');
          }
        }}
        onConfirm={() => {
          clearHistory();
          resetNavigation();
          setSavedInput('');
          setShowClearConfirm(false);
        }}
        onCancel={() => setShowClearConfirm(false)}
      />
    </div>
  );
};
