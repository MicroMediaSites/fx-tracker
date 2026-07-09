import { useState, useEffect, useRef, useCallback, useMemo, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useSettingsStore } from '../../stores/settingsStore';
import { SettingsModal } from '../settings/SettingsModal';
import { TerminalOverlay } from './TerminalOverlay';
import { getSuggestedPrompts } from '../../lib/suggestedPrompts';
import type { ChatContext } from '../../hooks/useTerminalChat';
import logo from '../../assets/logo_crop_bw.png';

type WindowType = 'account' | 'charting' | 'backtesting' | 'ticket' | 'watcher' | 'tradeanalysis';

interface OandaCredentials {
  environment: string;
  accountAlias?: string;
}

interface WindowHeaderProps {
  title: string | ReactNode;
  currentWindow: WindowType;
  /** Optional content to render between title and badge/menu */
  children?: ReactNode;
  /** Optional extra content for the right side (before badge) */
  rightContent?: ReactNode;
  /** Optional sub-header content rendered below the main header */
  subHeader?: ReactNode;
  /** Use full width (no max-w constraint) */
  fullWidth?: boolean;
  /** External control of settings modal - if provided, parent owns the state */
  settingsOpen?: boolean;
  /** Callback when settings should open/close - required if settingsOpen is provided */
  onSettingsChange?: (open: boolean) => void;
  /** Optional function to build context for AI terminal. Called on each message. */
  terminalContextProvider?: () => ChatContext;
  /** AI terminal header (cyan) */
  terminalHeader?: string;
  /** AI terminal description (gray) */
  terminalHeaderDescription?: string;
  /** AI terminal welcome content lines */
  terminalWelcomeContent?: string[];
}

/**
 * Reusable header component for all windows
 * Includes environment badge, profile menu, and settings modal
 * Listens for environment-changed events to sync badge across windows
 */
export const WindowHeader = ({
  title,
  currentWindow,
  children,
  rightContent,
  subHeader,
  fullWidth = false,
  settingsOpen: externalSettingsOpen,
  onSettingsChange,
  terminalContextProvider,
  terminalHeader,
  terminalHeaderDescription,
  terminalWelcomeContent,
}: WindowHeaderProps) => {
  const dataSource = useSettingsStore((s) => s.dataSource);
  // The AI terminal is always available; the backend command reports
  // clearly if local AI is not configured.
  const hasAiTerminal = true;
  const [credentials, setCredentials] = useState<OandaCredentials | null>(null);
  const [internalSettingsOpen, setInternalSettingsOpen] = useState(false);
  // Flame states: 'hidden' (initial), 'animating-rays', 'animating-ignition', 'steady'
  const [flameState, setFlameState] = useState<'hidden' | 'animating-rays' | 'animating-ignition' | 'steady'>('hidden');

  // Compute contextual suggested prompts for the AI terminal.
  // Uses the context provider if available to determine state (empty vs active).
  const suggestedPrompts = useMemo(() => {
    try {
      const context = terminalContextProvider?.() ?? { type: currentWindow };
      return getSuggestedPrompts(currentWindow, context as Record<string, unknown>);
    } catch {
      // Context provider may throw if data isn't ready yet - fall back to window-type-only prompts
      return getSuggestedPrompts(currentWindow, { type: currentWindow });
    }
  }, [currentWindow, terminalContextProvider]);

  // Header collapse state - persisted per window type
  const headerCollapseKey = `header-collapsed:${currentWindow}`;
  const [headerCollapsed, setHeaderCollapsed] = useState(() => {
    try {
      return localStorage.getItem(headerCollapseKey) === 'true';
    } catch {
      return false;
    }
  });

  const toggleHeaderCollapse = () => {
    const newValue = !headerCollapsed;
    setHeaderCollapsed(newValue);
    try {
      localStorage.setItem(headerCollapseKey, String(newValue));
    } catch {
      // localStorage not available
    }
  };

  // Set CSS variable for header height (used by sticky elements)
  useEffect(() => {
    // Collapsed header is 24px (h-6), expanded is ~60px (py-3 padding + content)
    // Terminal handle adds 12px when AI terminal is available
    const baseHeight = headerCollapsed ? 24 : 60;
    const handleHeight = hasAiTerminal && !headerCollapsed ? 12 : 0;
    const headerHeight = baseHeight + handleHeight;
    document.documentElement.style.setProperty('--header-height', `${headerHeight}px`);
  }, [headerCollapsed, hasAiTerminal]);

  // Use external state if provided, otherwise use internal state
  const isSettingsOpen = externalSettingsOpen ?? internalSettingsOpen;
  const setSettingsOpen = onSettingsChange ?? setInternalSettingsOpen;

  // Terminal state
  const [terminalHeight, setTerminalHeight] = useState(0);
  const [isDragging, setIsDragging] = useState(false);
  const [isAnimating, setIsAnimating] = useState(false);
  const [hasManuallyResized, setHasManuallyResized] = useState(false);
  const dragStartRef = useRef({ y: 0, height: 0 });
  // Ref to track current terminal height for mouseUp handler (avoids stale closure)
  const terminalHeightRef = useRef(terminalHeight);
  terminalHeightRef.current = terminalHeight;
  const headerRef = useRef<HTMLElement>(null);
  const [headerBottom, setHeaderBottom] = useState(0);

  const MIN_HEIGHT = 30;
  const DEFAULT_OPEN_HEIGHT = 36; // Minimal height when opening via handle click
  const DEFAULT_EXPANDED_HEIGHT = 200;
  const MAX_HEIGHT = typeof window !== 'undefined' ? window.innerHeight * 0.8 : 500;

  // Persist terminal height per window type
  const storageKey = `terminal-height:${currentWindow}`;

  // Load persisted height on mount
  const getPersistedHeight = useCallback((): number => {
    try {
      const stored = localStorage.getItem(storageKey);
      if (stored) {
        const height = parseInt(stored, 10);
        if (!isNaN(height) && height >= DEFAULT_OPEN_HEIGHT && height <= MAX_HEIGHT) {
          return height;
        }
      }
    } catch {
      // localStorage not available
    }
    return DEFAULT_OPEN_HEIGHT;
  }, [storageKey, MAX_HEIGHT]);

  const persistHeight = useCallback((height: number) => {
    try {
      if (height >= DEFAULT_OPEN_HEIGHT) {
        localStorage.setItem(storageKey, String(height));
      }
    } catch {
      // localStorage not available
    }
  }, [storageKey]);

  // Handle drag - only depends on isDragging to avoid re-render loop (BUG-076)
  // Uses terminalHeightRef so handleMouseUp reads the latest height without
  // adding terminalHeight to the dependency array.
  useEffect(() => {
    if (!isDragging) return;

    const handleMouseMove = (e: MouseEvent) => {
      const deltaY = e.clientY - dragStartRef.current.y;
      const newHeight = Math.min(MAX_HEIGHT, Math.max(0, dragStartRef.current.height + deltaY));
      setTerminalHeight(newHeight);
    };

    const handleMouseUp = () => {
      setIsDragging(false);
      const currentHeight = terminalHeightRef.current;
      if (currentHeight > MIN_HEIGHT) {
        persistHeight(currentHeight);
        setHasManuallyResized(true);
      }
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isDragging, persistHeight]);

  const handleDragStart = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
    setIsAnimating(false);
    dragStartRef.current = { y: e.clientY, height: terminalHeight };
  };

  // Called by TerminalOverlay when user sends a message - expand if at minimal height
  const handleRequestExpand = () => {
    if (!hasManuallyResized && terminalHeight <= 50) {
      setIsAnimating(true);
      setTerminalHeight(DEFAULT_EXPANDED_HEIGHT);
    }
  };

  // Keyboard shortcut (Cmd+K) - toggle with animation (only if user has AI access)
  useEffect(() => {
    if (!hasAiTerminal) return; // Don't register shortcut if user doesn't have access

    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        setIsAnimating(true);
        if (terminalHeight > 0) {
          // Close - persist current height first
          if (terminalHeight > MIN_HEIGHT) {
            persistHeight(terminalHeight);
          }
          setTerminalHeight(0);
          setHasManuallyResized(false);
        } else {
          // Open to persisted height
          setTerminalHeight(getPersistedHeight());
        }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [terminalHeight, persistHeight, getPersistedHeight, hasAiTerminal]);

  // Fetch credentials initially and when dataSource changes
  useEffect(() => {
    invoke<OandaCredentials>('get_oanda_credentials')
      .then(async (creds) => {
        setCredentials(creds);

        if (flameState === 'hidden') {
          // Check if vault was unlocked recently (within 5 seconds)
          const unlockedAtStr = localStorage.getItem('vault-unlocked-at');
          const unlockedAt = unlockedAtStr ? parseInt(unlockedAtStr, 10) : 0;
          const isRecentUnlock = Date.now() - unlockedAt < 5000;

          if (isRecentUnlock) {
            // First login - focused window shows rays, others show ignition only
            const focused = await getCurrentWebviewWindow().isFocused();
            setFlameState(focused ? 'animating-rays' : 'animating-ignition');
          } else {
            // Already logged in - show steady flame
            setFlameState('steady');
          }
        }
      })
      .catch(() => setCredentials(null));
  }, [dataSource]);

  // Listen for environment changes from other windows
  useEffect(() => {
    const unlisten = listen('environment-changed', () => {
      // Re-fetch credentials when environment changes
      invoke<OandaCredentials>('get_oanda_credentials')
        .then(setCredentials)
        .catch(() => setCredentials(null));
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);


  // Transition to steady state after animation completes
  useEffect(() => {
    if (flameState === 'animating-rays' || flameState === 'animating-ignition') {
      const timer = setTimeout(() => {
        setFlameState('steady');
      }, 7500); // Longest animation is ~7s
      return () => clearTimeout(timer);
    }
  }, [flameState]);

  // Track header bottom position for terminal portal positioning
  useEffect(() => {
    const updateHeaderBottom = () => {
      if (headerRef.current) {
        const rect = headerRef.current.getBoundingClientRect();
        setHeaderBottom(rect.bottom);
      }
    };

    updateHeaderBottom();
    window.addEventListener('resize', updateHeaderBottom);
    window.addEventListener('scroll', updateHeaderBottom);

    return () => {
      window.removeEventListener('resize', updateHeaderBottom);
      window.removeEventListener('scroll', updateHeaderBottom);
    };
  }, [headerCollapsed]);

  // Environment badge component
  const EnvironmentBadge = () => {
    if (credentials?.environment === 'live') {
      return (
        <span className="px-2 py-0.5 text-xs font-medium rounded bg-green-900/50 text-green-400 border border-green-600/30">
          Live
        </span>
      );
    }

    return (
      <span className="px-2 py-0.5 text-xs font-medium rounded bg-amber-900/50 text-amber-400 border border-amber-600/30">
        Demo
      </span>
    );
  };

  return (
    <>
      <header ref={headerRef} className="bg-[#0f1419] sticky top-0 z-[200] flex-shrink-0">
        {headerCollapsed ? (
          /* Collapsed header - thin bar with expand button */
          <div className="flex items-center justify-center h-6 border-b border-gray-700 bg-[#0f1419]/90">
            <button
              onClick={toggleHeaderCollapse}
              className="group flex items-center gap-1.5 px-3 py-0.5 text-gray-500 hover:text-gray-300 transition-colors"
              title="Show header"
            >
              <svg className="w-3 h-3 transition-transform group-hover:translate-y-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>
          </div>
        ) : (
          /* Expanded header - full content */
          <>
            <div className={`${fullWidth ? '' : 'max-w-7xl mx-auto'} px-4 py-3 flex justify-between items-center border-b border-gray-700`}>
              {/* Left side: Logo, Title and optional children */}
              <div className="flex items-center gap-4">
                {/* Logo with flame glow effect */}
                <div className={`relative z-30 ${
                  flameState === 'animating-rays' ? 'rays-active' :
                  flameState === 'animating-ignition' ? 'ignition-active' :
                  flameState === 'steady' ? 'flame-steady' : ''
                }`}>
                  {/* Breathing glow - slow pulse when connected */}
                  <div className="flame-breathe" />
                  {/* Glow behind the flame - AI purple/pink hue */}
                  <div className="absolute w-4 h-4 rounded-full blur-sm flame-glow-outer" />
                  {/* Dead zone around wick - dark ring where gases haven't ignited */}
                  <div
                    className="absolute w-2 h-2 rounded-full z-[5] flame-dead-zone"
                    style={{
                      background: 'radial-gradient(circle, rgba(17,24,39,0.8) 0%, rgba(17,24,39,0.55) 40%, transparent 70%)',
                      top: '0px',
                      left: 'calc(50% - 1.25px)',
                      transform: 'translateX(-50%)',
                    }}
                  />
                  {/* Inner bright core */}
                  <div
                    className="absolute w-2 h-2 rounded-full blur-[2px] flame-glow-inner"
                    style={{
                      background: 'radial-gradient(circle, rgba(255,255,255,0.9) 0%, rgba(236,72,153,1) 50%, rgba(236,72,153,0) 100%)',
                      top: '-1px',
                      left: 'calc(50% - 1.5px)',
                    }}
                  />
                  {/* Rotating light rays */}
                  <div className="flame-rays" style={{ top: '4px', left: '50%' }} />
                  <div className="flame-ray-3" style={{ top: '4px', left: '50%' }} />
                  <img src={logo} alt="wickd" className="h-7 w-auto invert relative z-10" />
                </div>
                {typeof title === 'string' ? (
                  <h1 className="text-lg font-semibold text-gray-100">{title}</h1>
                ) : (
                  title
                )}
                {children}
              </div>

              {/* Right side: Optional content, badge, menu, and collapse button */}
              <div className="flex items-center gap-3">
                {rightContent}
                <EnvironmentBadge />
                <button
                  onClick={() => setSettingsOpen(true)}
                  className="p-1 text-gray-500 hover:text-gray-300 transition-colors"
                  title="Settings"
                  aria-label="Settings"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                  </svg>
                </button>
                <button
                  onClick={toggleHeaderCollapse}
                  className="p-1 text-gray-500 hover:text-gray-300 transition-colors"
                  title="Hide header"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 15l7-7 7 7" />
                  </svg>
                </button>
              </div>
            </div>
            {/* Optional sub-header (e.g., chart controls) */}
            {subHeader}
          </>
        )}
        {/* Terminal container - handle + content, handle drags down revealing content above */}
      </header>
      {/* Terminal rendered via portal to body for correct fixed positioning.
          z-index scale: content (z-10/z-20) < terminal (z-35) < dropdowns (z-50) < modals (z-[150]) */}
      {hasAiTerminal && createPortal(
        <div
          className="fixed left-0 right-0 z-[35] flex flex-col pointer-events-none"
          data-testid="terminal-portal"
          style={{
            top: headerBottom,
            height: terminalHeight + 8, // content height + handle height
            transition: isAnimating && !isDragging ? 'height 0.3s ease-out' : 'none',
          }}
        >
          {/* Terminal content - above the handle */}
          <div
            className="flex-1 bg-gray-950/[0.87] overflow-hidden shadow-lg pointer-events-auto"
            style={{ height: terminalHeight }}
          >
            <TerminalOverlay
              height={terminalHeight}
              currentWindow={currentWindow}
              contextProvider={terminalContextProvider}
              onRequestExpand={handleRequestExpand}
              header={terminalHeader}
              headerDescription={terminalHeaderDescription}
              welcomeContent={terminalWelcomeContent}
              suggestedPrompts={suggestedPrompts}
            />
          </div>

          {/* Drag handle - at bottom, moves with terminal */}
          <div
            className={`flex justify-center items-center h-3 bg-gradient-to-b from-gray-800/50 to-[#0d1117] pointer-events-auto ${isDragging ? 'cursor-grabbing' : 'cursor-ns-resize'}`}
            onMouseDown={handleDragStart}
            title="Drag to resize"
            data-testid="terminal-drag-handle"
            data-tour="ai-overlay"
          >
            {/* Icon container - line bends into chevron on hover, click toggles terminal */}
            <div
              className="group/icon relative h-full flex items-center justify-center px-4 cursor-pointer"
              onMouseDown={(e) => e.stopPropagation()}
              title={terminalHeight > 0 ? 'Close terminal (⌘K)' : 'Open terminal (⌘K)'}
              onClick={() => {
                setIsAnimating(true);
                if (terminalHeight > 0) {
                  // Close - persist current height
                  if (terminalHeight > MIN_HEIGHT) {
                    persistHeight(terminalHeight);
                  }
                  setTerminalHeight(0);
                  setHasManuallyResized(false);
                } else {
                  // Open via handle click - reset to minimal height and clear persisted
                  try {
                    localStorage.removeItem(storageKey);
                  } catch {
                    // localStorage not available
                  }
                  setTerminalHeight(DEFAULT_OPEN_HEIGHT);
                  setHasManuallyResized(false);
                }
              }}
            >
              {/* Two-part line that bends into chevron - direction depends on terminal state */}
              <div className={`relative w-6 h-2 flex items-center justify-center transition-transform duration-200 ${
                terminalHeight > 0 ? 'group-hover/icon:-translate-y-[3px]' : 'group-hover/icon:translate-y-[3px]'
              }`}>
                {/* Left segment - rotates based on terminal state */}
                <div className={`absolute right-1/2 w-3 h-0.5 bg-emerald-400 rounded-l-full origin-right transition-transform duration-200 ${
                  terminalHeight > 0 ? 'group-hover/icon:-rotate-[25deg]' : 'group-hover/icon:rotate-[25deg]'
                }`} />
                {/* Right segment - rotates based on terminal state */}
                <div className={`absolute left-1/2 w-3 h-0.5 bg-emerald-400 rounded-r-full origin-left transition-transform duration-200 ${
                  terminalHeight > 0 ? 'group-hover/icon:rotate-[25deg]' : 'group-hover/icon:-rotate-[25deg]'
                }`} />
              </div>
            </div>
          </div>
        </div>,
        document.body
      )}
      <SettingsModal isOpen={isSettingsOpen} onClose={() => setSettingsOpen(false)} />
    </>
  );
}
