/**
 * NotesModal - Unified journal modal for trades and strategies
 *
 * Features:
 * - Notes list with add/delete
 * - Labels via unified LabelPicker
 * - Works for both trades and strategies
 *
 * Notes are served from the local store (~/.wickd/app.db, AGT-646) — no Zero.
 * NOTE: LabelPicker is the labels domain and still needs a Zero context; this
 * modal is only mounted from the legacy cloud windows today.
 */
import { useState, useEffect, useRef, useCallback } from 'react';
import { listNotes, saveNote, deleteNote, newLocalNote, type LocalNote } from '../../lib/localStore';
import { LabelPicker } from './LabelPicker';

type NotesModalProps = {
  isOpen: boolean;
  onClose: () => void;
  /** Called after a note is added or deleted (parents refresh note counts). */
  onNotesChanged?: () => void;
} & (
  | {
      entityType: 'trade';
      entityId: string;
      /** Display title (e.g., "Trade Journal") */
      title: string;
      /** Display subtitle (e.g., "EUR/USD") */
      subtitle: string;
      /** Trade open time for session suggestions */
      openTime?: number;
    }
  | {
      entityType: 'strategy';
      entityId: string;
      /** Display title (e.g., "Strategy Journal") */
      title: string;
      /** Display subtitle (e.g., strategy name) */
      subtitle: string;
      openTime?: never;
    }
);

export const NotesModal = (props: NotesModalProps) => {
  const { isOpen, onClose, onNotesChanged, entityType, entityId, title, subtitle } = props;
  const openTime = entityType === 'trade' ? props.openTime : undefined;

  const [content, setContent] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Notes for this entity, from the local store (already most recent first).
  const [notes, setNotes] = useState<LocalNote[]>([]);

  const refreshNotes = useCallback(async () => {
    try {
      setNotes(
        await listNotes(entityType === 'trade' ? { tradeId: entityId } : { strategyId: entityId }),
      );
    } catch (error) {
      console.error('Failed to load notes:', error);
    }
  }, [entityType, entityId]);

  // Load notes when the modal opens (or the target entity changes while open).
  useEffect(() => {
    if (isOpen) refreshNotes();
  }, [isOpen, refreshNotes]);

  const sortedNotes = notes;

  // Focus textarea when modal opens
  useEffect(() => {
    if (isOpen) {
      setTimeout(() => textareaRef.current?.focus(), 100);
    }
  }, [isOpen]);

  // Handle escape key
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        onClose();
      }
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [isOpen, onClose]);

  const handleSubmit = async (e?: React.FormEvent) => {
    e?.preventDefault();
    if (!content.trim()) return;

    setIsSubmitting(true);
    try {
      await saveNote(
        newLocalNote(
          content.trim(),
          entityType === 'trade' ? { tradeId: entityId } : { strategyId: entityId },
        ),
      );
      setContent('');
      await refreshNotes();
      onNotesChanged?.();
    } catch (error) {
      console.error('Failed to create note:', error);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Cmd/Ctrl + Enter to submit
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleDeleteNote = async (noteId: string) => {
    try {
      await deleteNote(noteId);
      await refreshNotes();
      onNotesChanged?.();
    } catch (error) {
      console.error('Failed to delete note:', error);
    }
  };

  const formatDate = (timestamp: number) => {
    return new Date(timestamp).toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      year: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
    });
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-[150] flex items-center justify-center" onClick={onClose}>
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-[var(--color-bg-elevated)] rounded-lg shadow-xl max-w-2xl w-full mx-4 max-h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-border)]">
          <div>
            <h3 className="text-lg font-semibold text-[var(--color-text-primary)]">{title}</h3>
            <p className="text-sm text-[var(--color-text-muted)]">{subtitle}</p>
          </div>
          <button
            onClick={onClose}
            className="p-1 hover:bg-[var(--color-bg-hover)] rounded transition-colors text-[var(--color-text-muted)]"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* New note input area */}
        <div className="px-6 py-4 border-b border-[var(--color-border)]">
          <div className="flex gap-3">
            <textarea
              ref={textareaRef}
              value={content}
              onChange={(e) => setContent(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Write a note..."
              rows={4}
              className="flex-1 px-3 py-2 bg-[var(--color-bg-page)] border border-[var(--color-border)] rounded-lg focus:outline-none focus:border-[var(--color-info)] text-sm text-[var(--color-text-primary)] placeholder-[var(--color-text-muted)] resize-none overflow-y-auto"
            />
            <button
              onClick={() => handleSubmit()}
              disabled={isSubmitting || !content.trim()}
              className="self-end px-4 py-2 bg-[var(--color-info)] text-white rounded-lg hover:bg-[var(--color-info)]/90 transition-colors text-sm disabled:opacity-50 disabled:cursor-not-allowed whitespace-nowrap"
            >
              {isSubmitting ? 'Saving...' : 'Save Note'}
            </button>
          </div>
          <p className="text-[10px] text-[var(--color-text-muted)] mt-2">
            Press{' '}
            <kbd className="px-1 py-0.5 bg-[var(--color-bg-page)] rounded text-[9px]">⌘</kbd> +{' '}
            <kbd className="px-1 py-0.5 bg-[var(--color-bg-page)] rounded text-[9px]">Enter</kbd> to save
          </p>

          {/* Labels */}
          <div className="mt-3 pt-3 border-t border-[var(--color-border)]/50">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide">Labels</span>
              <LabelPicker entityType={entityType} entityId={entityId} openTime={openTime} />
            </div>
          </div>
        </div>

        {/* Notes list - most recent first */}
        <div className="flex-1 overflow-y-auto px-6 py-4 min-h-[200px]">
          {sortedNotes.length === 0 ? (
            <p className="text-[var(--color-text-muted)] text-center py-8 text-sm">
              No notes yet. Start your journal above.
            </p>
          ) : (
            <div className="space-y-4">
              {sortedNotes.map((note) => (
                <div key={note.id} className="group">
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex-1 min-w-0">
                      <p className="text-[var(--color-text-primary)] text-sm whitespace-pre-wrap leading-relaxed">
                        {note.content}
                      </p>
                      <p className="text-[var(--color-text-muted)] text-xs mt-1.5">{formatDate(note.created_at)}</p>
                    </div>
                    <button
                      onClick={() => handleDeleteNote(note.id)}
                      className="p-1 text-[var(--color-text-muted)] opacity-0 group-hover:opacity-100 hover:text-[var(--color-sell)] hover:bg-[var(--color-sell)]/10 rounded transition-all"
                      title="Delete note"
                    >
                      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                        />
                      </svg>
                    </button>
                  </div>
                  {/* Subtle separator between notes */}
                  <div className="mt-4 border-b border-[var(--color-border)]/30" />
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
