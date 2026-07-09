import { useState, useEffect, useRef, useCallback, ReactNode } from 'react';

interface CollapsibleSectionProps {
  id: string; // Used for localStorage key
  title: string;
  badge?: ReactNode; // Optional badge next to title (e.g., count)
  action?: ReactNode; // Optional action button in header (e.g., + button)
  defaultCollapsed?: boolean;
  resizable?: boolean; // Enable vertical resizing
  defaultHeight?: number; // Default height in pixels (when resizable)
  minHeight?: number; // Minimum height (default 100)
  maxHeight?: number; // Maximum height (default 600)
  children: ReactNode;
}

export const CollapsibleSection = ({
  id,
  title,
  badge,
  action,
  defaultCollapsed = false,
  resizable = false,
  defaultHeight = 200,
  minHeight = 100,
  maxHeight = 600,
  children,
}: CollapsibleSectionProps) => {
  const collapsedKey = `candlesight_collapsed_${id}`;
  const heightKey = `candlesight_height_${id}`;

  const [collapsed, setCollapsed] = useState(() => {
    const stored = localStorage.getItem(collapsedKey);
    if (stored !== null) {
      return stored === 'true';
    }
    return defaultCollapsed;
  });

  const [height, setHeight] = useState(() => {
    if (!resizable) return defaultHeight;
    const stored = localStorage.getItem(heightKey);
    if (stored !== null) {
      const parsed = parseInt(stored, 10);
      if (!isNaN(parsed)) return Math.min(Math.max(parsed, minHeight), maxHeight);
    }
    return defaultHeight;
  });

  const isDragging = useRef(false);
  const startY = useRef(0);
  const startHeight = useRef(0);

  // Persist collapse state
  useEffect(() => {
    localStorage.setItem(collapsedKey, String(collapsed));
  }, [collapsed, collapsedKey]);

  // Persist height
  useEffect(() => {
    if (resizable) {
      localStorage.setItem(heightKey, String(height));
    }
  }, [height, heightKey, resizable]);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    isDragging.current = true;
    startY.current = e.clientY;
    startHeight.current = height;
    document.body.style.cursor = 'ns-resize';
    document.body.style.userSelect = 'none';
  }, [height]);

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isDragging.current) return;
      const delta = e.clientY - startY.current;
      const newHeight = Math.min(Math.max(startHeight.current + delta, minHeight), maxHeight);
      setHeight(newHeight);
    };

    const handleMouseUp = () => {
      if (isDragging.current) {
        isDragging.current = false;
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
      }
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [minHeight, maxHeight]);

  return (
    <section>
      {/* Header - minimal styling, just a thin separator line below */}
      <div className="flex items-center justify-between py-2 border-b border-[var(--color-border)]">
        <button
          onClick={() => setCollapsed(!collapsed)}
          className="flex items-center gap-2 -m-1 p-1 rounded transition-colors"
        >
          <svg
            className={`w-3 h-3 text-[var(--color-text-muted)] transition-transform ${collapsed ? '' : 'rotate-90'}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
          <h2 className="text-sm font-medium text-[var(--color-text-secondary)]">{title}</h2>
          {badge}
        </button>
        <div className="flex items-center gap-2">
          {action && (
            <div onClick={(e) => e.stopPropagation()}>
              {action}
            </div>
          )}
        </div>
      </div>

      {!collapsed && (
        <>
          <div
            className={`overflow-y-auto ${resizable ? 'pt-3' : 'py-3'}`}
            style={resizable ? { height: `${height}px` } : undefined}
          >
            {children}
          </div>

          {/* Resize handle - full width line with centered grip */}
          {resizable && (
            <div
              onMouseDown={handleMouseDown}
              className="relative h-4 cursor-ns-resize group flex items-center justify-center"
            >
              <div className="absolute inset-x-0 top-1/2 h-px bg-[var(--color-border)] group-hover:bg-[var(--color-text-muted)]/50 transition-colors" />
              <div className="relative w-8 h-1 rounded-full bg-[var(--color-border)] group-hover:bg-[var(--color-text-muted)] transition-colors" />
            </div>
          )}
        </>
      )}
    </section>
  );
};
