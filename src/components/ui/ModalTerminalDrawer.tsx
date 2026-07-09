import { useState, useRef, useEffect, useCallback } from 'react';
import { TerminalOverlay } from './TerminalOverlay';
import type { ChatContext } from '../../hooks/useTerminalChat';

interface ModalTerminalDrawerProps {
  /** Unique identifier for this modal (used for height persistence and session) */
  modalId: string;
  /** Function to build context for AI. Called on each message. */
  contextProvider: () => ChatContext;
  /** Header shown in empty state (cyan) */
  header?: string;
  /** Description shown below header (gray) */
  headerDescription?: string;
  /** Welcome content lines shown when empty */
  welcomeContent?: string[];
  /** Top offset from modal container (header height) */
  topOffset?: number;
}

const MIN_HEIGHT = 30;
const DEFAULT_OPEN_HEIGHT = 36;
const DEFAULT_EXPANDED_HEIGHT = 200;
const HANDLE_HEIGHT = 12;

/**
 * Terminal drawer component for modals.
 * Provides the same terminal experience as WindowHeader but decoupled from window context.
 * Context is passed via props when the modal mounts.
 *
 * This component is absolutely positioned and overlays modal content (doesn't push it down).
 */
export const ModalTerminalDrawer = ({
  modalId,
  contextProvider,
  header,
  headerDescription,
  welcomeContent,
  topOffset = 73, // Default header height (py-4 = 16*2 + content ~41)
}: ModalTerminalDrawerProps) => {
  const [terminalHeight, setTerminalHeight] = useState(0);
  const [isDragging, setIsDragging] = useState(false);
  const [isAnimating, setIsAnimating] = useState(false);
  const [hasManuallyResized, setHasManuallyResized] = useState(false);
  const dragStartRef = useRef({ y: 0, height: 0 });
  // Ref to track current terminal height for mouseUp handler (avoids stale closure)
  const terminalHeightRef = useRef(terminalHeight);
  terminalHeightRef.current = terminalHeight;

  // Max height for modal context
  const MAX_HEIGHT = typeof window !== 'undefined' ? window.innerHeight * 0.8 : 500;

  // Persist terminal height per modal type
  const storageKey = `modal-terminal-height:${modalId}`;

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

  const handleRequestExpand = useCallback(() => {
    if (!hasManuallyResized && terminalHeight <= 50) {
      setIsAnimating(true);
      setTerminalHeight(DEFAULT_EXPANDED_HEIGHT);
    }
  }, [hasManuallyResized, terminalHeight]);

  const handleToggle = useCallback(() => {
    setIsAnimating(true);
    if (terminalHeight > 0) {
      if (terminalHeight > MIN_HEIGHT) {
        persistHeight(terminalHeight);
      }
      setTerminalHeight(0);
      setHasManuallyResized(false);
    } else {
      setTerminalHeight(getPersistedHeight());
    }
  }, [terminalHeight, persistHeight, getPersistedHeight]);

  // Keyboard shortcut (Cmd+K) - toggle with animation
  // Note: Only capture Cmd+K, not Cmd+C (copy) or other standard shortcuts
  // Uses stopImmediatePropagation to prevent WindowHeader from also responding
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        e.stopImmediatePropagation();
        handleToggle();
      }
    };
    // Use capture phase to intercept before WindowHeader's listener
    document.addEventListener('keydown', handleKeyDown, true);
    return () => document.removeEventListener('keydown', handleKeyDown, true);
  }, [handleToggle]);

  return (
    <div
      className="absolute left-0 right-0 z-20 flex flex-col"
      style={{
        top: topOffset,
        height: terminalHeight + HANDLE_HEIGHT,
        transition: isAnimating && !isDragging ? 'height 0.3s ease-out' : 'none',
      }}
    >
      {/* Terminal content - above the handle */}
      <div
        className="flex-1 bg-gray-950/[0.87] overflow-hidden shadow-lg"
        style={{ height: terminalHeight }}
      >
        <TerminalOverlay
          height={terminalHeight}
          currentWindow={`modal:${modalId}`}
          contextProvider={contextProvider}
          onRequestExpand={handleRequestExpand}
          header={header}
          headerDescription={headerDescription}
          welcomeContent={welcomeContent}
        />
      </div>

      {/* Drag handle - at bottom, moves with terminal */}
      <div
        className={`flex justify-center items-center bg-[#0d1117] ${
          isDragging ? 'cursor-grabbing' : 'cursor-ns-resize'
        }`}
        style={{ height: HANDLE_HEIGHT }}
        onMouseDown={handleDragStart}
        title="Drag to resize"
      >
        <div
          className="group/icon relative h-full flex items-center justify-center px-4 cursor-pointer"
          onMouseDown={(e) => e.stopPropagation()}
          title={terminalHeight > 0 ? 'Close terminal (⌘K)' : 'Open terminal (⌘K)'}
          onClick={handleToggle}
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
    </div>
  );
};
