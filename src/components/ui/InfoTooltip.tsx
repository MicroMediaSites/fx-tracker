/**
 * InfoTooltip - Small info icon with hover tooltip.
 *
 * Use next to labels to provide additional context.
 */
import { useState, useRef, useEffect } from 'react';

interface InfoTooltipProps {
  text: string;
  className?: string;
}

export const InfoTooltip = ({ text, className = '' }: InfoTooltipProps) => {
  const [show, setShow] = useState(false);
  const [position, setPosition] = useState<'top' | 'bottom'>('top');
  const iconRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (show && iconRef.current) {
      const rect = iconRef.current.getBoundingClientRect();
      // If too close to top, show tooltip below
      setPosition(rect.top < 80 ? 'bottom' : 'top');
    }
  }, [show]);

  return (
    <span
      ref={iconRef}
      className={`relative inline-flex items-center ml-1 ${className}`}
      onMouseEnter={() => setShow(true)}
      onMouseLeave={() => setShow(false)}
    >
      <svg
        className="w-3.5 h-3.5 text-gray-500 hover:text-gray-400 cursor-help"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
        />
      </svg>
      {show && (
        <span
          className={`absolute z-50 px-2 py-1 text-xs text-gray-200 bg-gray-900 rounded shadow-lg whitespace-normal min-w-[180px] max-w-[280px] ${
            position === 'top'
              ? 'bottom-full mb-1 left-1/2 -translate-x-1/2'
              : 'top-full mt-1 left-1/2 -translate-x-1/2'
          }`}
        >
          {text}
        </span>
      )}
    </span>
  );
};
