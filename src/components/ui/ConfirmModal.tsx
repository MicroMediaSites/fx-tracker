import { useEffect, useRef, useState } from 'react';

interface ConfirmModalProps {
  isOpen: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  variant?: 'danger' | 'warning' | 'info';
  onConfirm: () => void;
  onCancel: () => void;
  /** Optional "don't show again" checkbox */
  showDontAskAgain?: boolean;
  /** Called with checkbox state when confirming (only if showDontAskAgain is true) */
  onDontAskAgainChange?: (checked: boolean) => void;
}

export const ConfirmModal = ({
  isOpen,
  title,
  message,
  confirmLabel = 'Confirm',
  cancelLabel = 'Cancel',
  variant = 'danger',
  onConfirm,
  onCancel,
  showDontAskAgain = false,
  onDontAskAgainChange,
}: ConfirmModalProps) => {
  const confirmRef = useRef<HTMLButtonElement>(null);
  const [dontAskAgain, setDontAskAgain] = useState(false);

  // Reset checkbox when modal opens
  useEffect(() => {
    if (isOpen) {
      setDontAskAgain(false);
    }
  }, [isOpen]);

  useEffect(() => {
    if (isOpen) {
      confirmRef.current?.focus();
    }
  }, [isOpen]);

  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        onCancel();
      }
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [isOpen, onCancel]);

  if (!isOpen) return null;

  const confirmButtonClass = {
    danger: 'bg-red-600 hover:bg-red-700',
    warning: 'bg-yellow-600 hover:bg-yellow-700',
    info: 'bg-blue-600 hover:bg-blue-700',
  }[variant];

  return (
    <div
      className="fixed inset-0 z-[200] flex items-center justify-center"
      onClick={onCancel}
    >
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-gray-800 rounded-lg shadow-xl max-w-md w-full mx-4 p-6"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-lg font-semibold mb-2">{title}</h3>
        <p className="text-gray-400 mb-4">{message}</p>
        {showDontAskAgain && (
          <label className="flex items-center gap-2 mb-4 text-sm text-gray-400 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={dontAskAgain}
              onChange={(e) => setDontAskAgain(e.target.checked)}
              className="w-4 h-4 rounded border-gray-600 bg-gray-700 text-cyan-500 focus:ring-cyan-500 focus:ring-offset-gray-800"
            />
            Don't show this warning again
          </label>
        )}
        <div className="flex justify-end gap-3">
          <button
            onClick={onCancel}
            className="px-4 py-2 bg-gray-600 rounded hover:bg-gray-500 transition-colors"
          >
            {cancelLabel}
          </button>
          <button
            ref={confirmRef}
            onClick={() => {
              if (showDontAskAgain && dontAskAgain) {
                onDontAskAgainChange?.(true);
              }
              onConfirm();
            }}
            className={`px-4 py-2 rounded transition-colors ${confirmButtonClass}`}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
