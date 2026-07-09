/**
 * LabelPicker - Unified dropdown for adding/removing labels on trades or strategies
 *
 * Features:
 * - Shows applied labels as pills with remove buttons
 * - "+" button to open dropdown
 * - Lists existing labels to add
 * - Inline label creation
 * - Smart suggestions for trades (session, strategies, instruments)
 */
import { useState, useRef, useEffect, useMemo, useCallback } from 'react';
import { LabelPill, getDefaultColor } from './LabelPill';
import { useSettingsStore } from '../../stores/settingsStore';
import {
  addStrategyLabel,
  addTradeLabel,
  deleteStrategyLabel,
  deleteTradeLabel,
  listLabels,
  listStrategies,
  listStrategyLabels,
  listTradeLabels,
  newLocalLabel,
  saveLabel,
  type LocalLabel,
  type LocalStrategy,
  type LocalStrategyLabel,
  type LocalTradeLabel,
} from '../../lib/localStore';

interface LabelPickerProps {
  /** Entity type - determines which junction table to use */
  entityType: 'trade' | 'strategy';
  /** Entity ID (trade ID or strategy ID) */
  entityId: string;
  /** Trade open time for session suggestions (only applies to trades) */
  openTime?: number;
}

/**
 * Detect trade session from timestamp
 */
export function getTradeSession(openTime: number): 'Asian' | 'London' | 'NY' | null {
  const date = new Date(openTime);
  const hour = date.getUTCHours();

  // Asian: 00:00-08:00 UTC
  if (hour >= 0 && hour < 8) return 'Asian';
  // London: 08:00-16:00 UTC
  if (hour >= 8 && hour < 16) return 'London';
  // NY: 12:00-21:00 UTC (overlaps with London)
  if (hour >= 12 && hour < 21) return 'NY';

  return null;
}

export const LabelPicker = ({ entityType, entityId, openTime }: LabelPickerProps) => {
  const { mySymbols } = useSettingsStore();

  // Labels + junctions from the local store (AGT-650: was Zero).
  const [labels, setLabels] = useState<LocalLabel[]>([]);
  const [tradeLabels, setTradeLabels] = useState<LocalTradeLabel[]>([]);
  const [strategyLabels, setStrategyLabels] = useState<LocalStrategyLabel[]>([]);

  const reload = useCallback(async () => {
    try {
      setLabels(await listLabels());
      if (entityType === 'trade') {
        setTradeLabels(await listTradeLabels(entityId));
      } else {
        setStrategyLabels(await listStrategyLabels(entityId));
      }
    } catch (err) {
      console.error('[LabelPicker] Failed to load labels:', err);
    }
  }, [entityType, entityId]);

  useEffect(() => {
    reload();
  }, [reload]);

  const appliedLabelIds = useMemo(() => {
    if (entityType === 'trade') {
      return tradeLabels.map((tl) => tl.label_id);
    } else {
      return strategyLabels.map((sl) => sl.label_id);
    }
  }, [entityType, tradeLabels, strategyLabels]);

  // Promoted (live) strategies for suggestions (only for trades)
  const [strategies, setStrategies] = useState<LocalStrategy[]>([]);
  useEffect(() => {
    if (entityType !== 'trade') return;
    listStrategies()
      .then((rows) => setStrategies(rows.filter((r) => r.is_promoted && r.is_active)))
      .catch(() => setStrategies([]));
  }, [entityType]);

  const [isOpen, setIsOpen] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [searchText, setSearchText] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Get trade session for smart label suggestions
  const tradeSession = useMemo(() => {
    if (entityType !== 'trade' || !openTime) return null;
    return getTradeSession(openTime);
  }, [entityType, openTime]);

  // Close on outside click
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
        setIsCreating(false);
        setSearchText('');
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

  // Get labels not yet applied
  const availableLabels = useMemo(() => {
    return (labels || []).filter((l) => !appliedLabelIds.includes(l.id));
  }, [labels, appliedLabelIds]);

  // Get applied labels
  const appliedLabels = useMemo(() => {
    return (labels || []).filter((l) => appliedLabelIds.includes(l.id));
  }, [labels, appliedLabelIds]);

  // Filter available labels by search text
  const filteredLabels = useMemo(() => {
    if (!searchText) return availableLabels;
    const lower = searchText.toLowerCase();
    return availableLabels.filter((l) => l.name.toLowerCase().includes(lower));
  }, [availableLabels, searchText]);

  // Build smart suggestions (only for trades)
  const suggestions = useMemo(() => {
    if (entityType !== 'trade') return [];

    const items: Array<{ name: string; category: string; highlight?: boolean }> = [];
    const existingNames = new Set((labels || []).map((l) => l.name.toLowerCase()));

    // Session suggestion (highlighted if matches trade session)
    const sessions = ['Asian', 'London', 'NY'];
    sessions.forEach((session) => {
      if (!existingNames.has(session.toLowerCase())) {
        items.push({
          name: session,
          category: 'Session',
          highlight: tradeSession === session,
        });
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
    if (searchText) {
      const lower = searchText.toLowerCase();
      return items.filter((i) => i.name.toLowerCase().includes(lower));
    }

    return items;
  }, [entityType, labels, strategies, mySymbols, tradeSession, searchText]);

  // Check if search text matches an existing label exactly
  const exactMatch = useMemo(() => {
    if (!searchText) return false;
    return (labels || []).some((l) => l.name.toLowerCase() === searchText.toLowerCase());
  }, [labels, searchText]);

  // Check if search text matches a suggestion exactly
  const matchesSuggestion = useMemo(() => {
    if (!searchText) return false;
    return suggestions.some((s) => s.name.toLowerCase() === searchText.toLowerCase());
  }, [suggestions, searchText]);

  // Create a new label and apply it
  const handleCreateLabel = useCallback(
    async (name: string) => {
      if (isSubmitting || !name.trim()) return;

      setIsSubmitting(true);
      try {
        const label = newLocalLabel(name.trim());
        await saveLabel(label);

        // Apply the label to the entity
        if (entityType === 'trade') {
          await addTradeLabel(entityId, label.id);
        } else {
          await addStrategyLabel(entityId, label.id);
        }

        await reload();
        setSearchText('');
        setIsCreating(false);
      } finally {
        setIsSubmitting(false);
      }
    },
    [entityType, entityId, isSubmitting, reload]
  );

  // Add an existing label
  const handleAddLabel = useCallback(
    async (labelId: string) => {
      if (entityType === 'trade') {
        await addTradeLabel(entityId, labelId);
      } else {
        await addStrategyLabel(entityId, labelId);
      }
      await reload();
      // Don't close - allow selecting multiple labels
    },
    [entityType, entityId, reload]
  );

  // Remove a label
  const handleRemoveLabel = useCallback(
    async (labelId: string) => {
      if (entityType === 'trade') {
        const tradeLabel = tradeLabels.find((tl) => tl.label_id === labelId);
        if (tradeLabel) {
          await deleteTradeLabel(tradeLabel.id);
        }
      } else {
        const strategyLabel = strategyLabels.find((sl) => sl.label_id === labelId);
        if (strategyLabel) {
          await deleteStrategyLabel(strategyLabel.id);
        }
      }
      await reload();
    },
    [entityType, tradeLabels, strategyLabels, reload]
  );

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && searchText && !exactMatch && !matchesSuggestion) {
      handleCreateLabel(searchText);
    } else if (e.key === 'Escape') {
      setIsCreating(false);
      setSearchText('');
    }
  };

  return (
    <div className="relative inline-block" ref={containerRef}>
      {/* Applied labels + Add button */}
      <div className="flex flex-wrap items-center gap-1.5">
        {appliedLabels.map((label) => (
          <LabelPill
            key={label.id}
            name={label.name}
            color={label.color ?? undefined}
            size="xs"
            onRemove={() => handleRemoveLabel(label.id)}
          />
        ))}
        <button
          onClick={() => setIsOpen(!isOpen)}
          className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] rounded-full border border-dashed border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
        >
          <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
          </svg>
          Label
        </button>
      </div>

      {/* Dropdown */}
      {isOpen && (
        <div className="absolute top-full left-0 mt-1 min-w-[200px] bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-lg shadow-xl z-50 overflow-hidden">
          {!isCreating ? (
            <>
              {/* Available labels */}
              {filteredLabels.length > 0 && (
                <div className="py-1 max-h-40 overflow-auto">
                  {filteredLabels.map((label) => (
                    <button
                      key={label.id}
                      onClick={() => handleAddLabel(label.id)}
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

              {filteredLabels.length === 0 && (
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
              {/* Search/create input */}
              <input
                ref={inputRef}
                type="text"
                value={searchText}
                onChange={(e) => setSearchText(e.target.value)}
                onKeyDown={handleKeyDown}
                placeholder={entityType === 'trade' ? 'Search or create...' : 'Label name...'}
                className="w-full bg-[var(--color-bg-hover)] border border-[var(--color-border)] rounded px-2 py-1.5 text-xs text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)] focus:outline-none focus:border-[var(--color-border-focus)]"
              />

              {/* Suggestions (only for trades) */}
              {entityType === 'trade' && suggestions.length > 0 && (
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
                        className={`px-2 py-0.5 text-[11px] rounded-full border transition-colors disabled:opacity-50 ${
                          s.highlight
                            ? 'bg-[var(--color-info)]/20 border-[var(--color-info)]/50 text-[var(--color-info)]'
                            : 'border-[var(--color-border)] text-[var(--color-text-secondary)] hover:border-[var(--color-text-muted)]'
                        }`}
                        title={s.category}
                      >
                        {s.name}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Create custom */}
              {searchText && !exactMatch && !matchesSuggestion && (
                <button
                  onClick={() => handleCreateLabel(searchText)}
                  disabled={isSubmitting}
                  className="mt-2 w-full px-2 py-1.5 text-xs text-left text-[var(--color-info)] hover:bg-[var(--color-bg-hover)] rounded transition-colors disabled:opacity-50 flex items-center gap-2"
                >
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
                  </svg>
                  Create "{searchText}"
                </button>
              )}

              {exactMatch && (
                <p className="mt-2 text-xs text-[var(--color-text-muted)]">Label already exists</p>
              )}

              {/* Back button */}
              <button
                onClick={() => {
                  setIsCreating(false);
                  setSearchText('');
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
