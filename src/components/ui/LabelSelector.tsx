/**
 * LabelSelector - Select labels to attach to a future trade
 *
 * Unlike LabelPicker (which manages labels on an existing entity),
 * this component lets users pre-select labels before an entity exists.
 *
 * Features:
 * - Select from existing labels
 * - Create new labels inline
 * - Smart suggestions (sessions, live strategies, instruments)
 */
import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import { LabelPill, getDefaultColor } from './LabelPill';
import { useSettingsStore } from '../../stores/settingsStore';
import {
  listLabels,
  listStrategies,
  newLocalLabel,
  saveLabel,
  type LocalLabel,
  type LocalStrategy,
} from '../../lib/localStore';

interface LabelSelectorProps {
  /** Currently selected label IDs */
  selectedIds: string[];
  /** Callback when selection changes */
  onChange: (ids: string[]) => void;
}

export const LabelSelector = ({ selectedIds, onChange }: LabelSelectorProps) => {
  const { mySymbols } = useSettingsStore();

  // Labels + promoted strategies from the local store (AGT-650: was Zero).
  const [labels, setLabels] = useState<LocalLabel[]>([]);
  const [strategies, setStrategies] = useState<LocalStrategy[]>([]);

  const reloadLabels = useCallback(async () => {
    try {
      setLabels(await listLabels());
    } catch (err) {
      console.error('[LabelSelector] Failed to load labels:', err);
    }
  }, []);

  useEffect(() => {
    reloadLabels();
    listStrategies()
      .then((rows) => setStrategies(rows.filter((r) => r.is_promoted && r.is_active)))
      .catch(() => setStrategies([]));
  }, [reloadLabels]);

  const [isOpen, setIsOpen] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [newLabelName, setNewLabelName] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Close on outside click
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
        setIsCreating(false);
        setNewLabelName('');
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  // Focus input when entering create mode
  useEffect(() => {
    if (isCreating && inputRef.current) {
      inputRef.current.focus();
    }
  }, [isCreating]);

  // Get selected labels
  const selectedLabels = useMemo(() => {
    return (labels || []).filter((l) => selectedIds.includes(l.id));
  }, [labels, selectedIds]);

  // Get unselected labels
  const unselectedLabels = useMemo(() => {
    return (labels || []).filter((l) => !selectedIds.includes(l.id));
  }, [labels, selectedIds]);

  // Build smart suggestions
  const suggestions = useMemo(() => {
    const items: Array<{ name: string; category: string }> = [];
    const existingNames = new Set((labels || []).map((l) => l.name.toLowerCase()));

    // Session suggestions
    const sessions = ['Asian', 'London', 'NY'];
    sessions.forEach((session) => {
      if (!existingNames.has(session.toLowerCase())) {
        items.push({ name: session, category: 'Session' });
      }
    });

    // Live strategy suggestions
    (strategies || []).forEach((s) => {
      if (!existingNames.has(s.name.toLowerCase())) {
        items.push({ name: s.name, category: 'Strategy' });
      }
    });

    // Instrument suggestions
    mySymbols.forEach((symbol) => {
      const formatted = symbol.replace('_', '/');
      if (!existingNames.has(formatted.toLowerCase())) {
        items.push({ name: formatted, category: 'Instrument' });
      }
    });

    // Filter by search text
    if (newLabelName) {
      const lower = newLabelName.toLowerCase();
      return items.filter((i) => i.name.toLowerCase().includes(lower));
    }

    return items;
  }, [labels, strategies, mySymbols, newLabelName]);

  // Check if newLabelName matches a suggestion exactly
  const matchesSuggestion = useMemo(() => {
    if (!newLabelName) return false;
    return suggestions.some((s) => s.name.toLowerCase() === newLabelName.toLowerCase());
  }, [suggestions, newLabelName]);

  // Toggle a label selection
  const toggleLabel = useCallback(
    (labelId: string) => {
      if (selectedIds.includes(labelId)) {
        onChange(selectedIds.filter((id) => id !== labelId));
      } else {
        onChange([...selectedIds, labelId]);
      }
    },
    [selectedIds, onChange]
  );

  // Create a new label and select it
  const handleCreateLabel = useCallback(async (nameOverride?: string) => {
    const name = nameOverride || newLabelName;
    if (isSubmitting || !name.trim()) return;

    // Check if label already exists
    const existingLabel = (labels || []).find(
      (l) => l.name.toLowerCase() === name.trim().toLowerCase()
    );
    if (existingLabel) {
      // If it exists, just select it
      if (!selectedIds.includes(existingLabel.id)) {
        onChange([...selectedIds, existingLabel.id]);
      }
      setNewLabelName('');
      setIsCreating(false);
      return;
    }

    setIsSubmitting(true);
    try {
      const label = newLocalLabel(name.trim());
      await saveLabel(label);
      await reloadLabels();
      // Auto-select the new label
      onChange([...selectedIds, label.id]);
      setNewLabelName('');
      // Stay in create mode so user can add more labels
    } finally {
      setIsSubmitting(false);
    }
  }, [newLabelName, labels, selectedIds, onChange, isSubmitting, reloadLabels]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && newLabelName && !matchesSuggestion) {
      handleCreateLabel();
    } else if (e.key === 'Escape') {
      setIsCreating(false);
      setNewLabelName('');
    }
  };

  return (
    <div className="relative" ref={containerRef}>
      {/* Selected labels + Add button */}
      <div className="flex flex-wrap items-center gap-1.5">
        {selectedLabels.map((label) => (
          <LabelPill
            key={label.id}
            name={label.name}
            color={label.color ?? undefined}
            size="xs"
            onRemove={() => toggleLabel(label.id)}
          />
        ))}
        <button
          onClick={() => setIsOpen(!isOpen)}
          className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] rounded-full border border-dashed border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
        >
          <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
          </svg>
          {selectedLabels.length === 0 ? 'Add Labels' : 'Add'}
        </button>
      </div>

      {/* Dropdown */}
      {isOpen && (
        <div className="absolute bottom-full left-0 mb-1 min-w-[200px] bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-lg shadow-xl z-50 overflow-hidden">
          {!isCreating ? (
            <>
              {/* Available labels to select */}
              {unselectedLabels.length > 0 && (
                <div className="py-1 max-h-32 overflow-auto">
                  {unselectedLabels.map((label) => (
                    <button
                      key={label.id}
                      onClick={() => {
                        toggleLabel(label.id);
                      }}
                      className="w-full px-3 py-1.5 text-left text-xs hover:bg-[var(--color-bg-hover)] transition-colors flex items-center gap-2"
                    >
                      <span
                        className="w-2 h-2 rounded-full"
                        style={{ backgroundColor: label.color || getDefaultColor(label.name) }}
                      />
                      {label.name}
                    </button>
                  ))}
                </div>
              )}

              {unselectedLabels.length === 0 && labels && labels.length > 0 && (
                <div className="px-3 py-2 text-xs text-[var(--color-text-muted)]">All labels selected</div>
              )}

              {(!labels || labels.length === 0) && (
                <div className="px-3 py-2 text-xs text-[var(--color-text-muted)]">No labels yet</div>
              )}

              {/* New label button */}
              <div className="border-t border-[var(--color-border)]">
                <button
                  onClick={() => setIsCreating(true)}
                  className="w-full px-3 py-2 text-left text-xs text-[var(--color-info)] hover:bg-[var(--color-bg-hover)] transition-colors flex items-center gap-2"
                >
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                  </svg>
                  New Label
                </button>
              </div>
            </>
          ) : (
            /* Create mode */
            <div className="p-2">
              <input
                ref={inputRef}
                type="text"
                value={newLabelName}
                onChange={(e) => setNewLabelName(e.target.value)}
                onKeyDown={handleKeyDown}
                placeholder="Search or create..."
                className="w-full bg-[var(--color-bg-hover)] border border-[var(--color-border)] rounded px-2 py-1.5 text-xs text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)] focus:outline-none focus:border-[var(--color-border-focus)]"
              />

              {/* Suggestions */}
              {suggestions.length > 0 && (
                <div className="mt-2">
                  <div className="text-[10px] uppercase tracking-wide text-[var(--color-text-muted)] mb-1.5 px-1">
                    Suggested
                  </div>
                  <div className="flex flex-wrap gap-1">
                    {suggestions.slice(0, 8).map((s) => (
                      <button
                        key={s.name}
                        onClick={() => handleCreateLabel(s.name)}
                        disabled={isSubmitting}
                        className="px-2 py-0.5 text-[11px] rounded-full border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:border-[var(--color-text-muted)] transition-colors disabled:opacity-50"
                        title={s.category}
                      >
                        {s.name}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Create custom - only show if doesn't match existing label or suggestion */}
              {newLabelName && !matchesSuggestion && !(labels || []).some(l => l.name.toLowerCase() === newLabelName.toLowerCase()) && (
                <button
                  onClick={() => handleCreateLabel()}
                  disabled={isSubmitting}
                  className="mt-2 w-full px-2 py-1.5 text-xs text-left text-[var(--color-info)] hover:bg-[var(--color-bg-hover)] rounded transition-colors disabled:opacity-50 flex items-center gap-2"
                >
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                  </svg>
                  Create "{newLabelName}"
                </button>
              )}

              <button
                onClick={() => {
                  setIsCreating(false);
                  setNewLabelName('');
                }}
                className="mt-2 w-full px-2 py-1 text-[11px] text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
              >
                ← Back
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
};
