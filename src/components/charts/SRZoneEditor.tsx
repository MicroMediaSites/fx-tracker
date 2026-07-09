import { useState, useEffect } from 'react';
import type { SRZone } from './SRZoneOverlay';
import { getInstrumentPrecision } from './chartConstants';

interface SRZoneEditorProps {
  zone: SRZone | null;
  /** Current instrument for price precision (Bug #17) */
  instrument: string;
  onSave: (zoneId: string, updates: { label?: string; color?: string }) => void;
  onClose: () => void;
}

// Preset colors for zones
const PRESET_COLORS = [
  { name: 'Blue', value: 'rgba(59, 130, 246, 0.15)', hex: '#3b82f6' },
  { name: 'Green', value: 'rgba(34, 197, 94, 0.15)', hex: '#22c55e' },
  { name: 'Red', value: 'rgba(239, 68, 68, 0.15)', hex: '#ef4444' },
  { name: 'Purple', value: 'rgba(168, 85, 247, 0.15)', hex: '#a855f7' },
  { name: 'Orange', value: 'rgba(249, 115, 22, 0.15)', hex: '#f97316' },
  { name: 'Yellow', value: 'rgba(234, 179, 8, 0.15)', hex: '#eab308' },
  { name: 'Cyan', value: 'rgba(6, 182, 212, 0.15)', hex: '#06b6d4' },
  { name: 'Pink', value: 'rgba(236, 72, 153, 0.15)', hex: '#ec4899' },
];

// Convert hex to rgba with alpha
const hexToRgba = (hex: string, alpha: number): string => {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
};

// Extract hex from rgba string
const rgbaToHex = (rgba: string): string => {
  const match = rgba.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/);
  if (match) {
    const r = parseInt(match[1]).toString(16).padStart(2, '0');
    const g = parseInt(match[2]).toString(16).padStart(2, '0');
    const b = parseInt(match[3]).toString(16).padStart(2, '0');
    return `#${r}${g}${b}`;
  }
  return '#3b82f6'; // Default blue
};

export const SRZoneEditor = ({ zone, instrument, onSave, onClose }: SRZoneEditorProps) => {
  const [label, setLabel] = useState('');
  const [color, setColor] = useState('');

  useEffect(() => {
    if (zone) {
      setLabel(zone.label || '');
      setColor(zone.color || PRESET_COLORS[0].value);
    }
  }, [zone]);

  if (!zone) return null;

  const handleSave = () => {
    onSave(zone.id, {
      label: label.trim() || undefined,
      color: color || undefined,
    });
    onClose();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleSave();
    } else if (e.key === 'Escape') {
      onClose();
    }
  };

  return (
    <div
      className="fixed inset-0 bg-black/50 flex items-center justify-center z-[150]"
      onClick={onClose}
    >
      <div
        className="bg-[var(--color-bg-elevated)] rounded-lg p-4 w-80 shadow-lg"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <h3 className="text-lg font-medium mb-4 text-[var(--color-text-primary)]">Edit Zone</h3>

        {/* Label input */}
        <div className="mb-4">
          <label className="block text-sm text-[var(--color-text-muted)] mb-1">Label</label>
          <input
            type="text"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="e.g., Daily Support, Weekly Resistance"
            className="w-full bg-[var(--color-bg-card)] border border-[var(--color-border)] rounded px-3 py-2 text-sm focus:outline-none focus:border-[var(--color-info)]"
            autoFocus
          />
        </div>

        {/* Color picker */}
        <div className="mb-4">
          <label className="block text-sm text-[var(--color-text-muted)] mb-2">Color</label>
          <div className="grid grid-cols-4 gap-2 mb-3">
            {PRESET_COLORS.map((preset) => (
              <button
                key={preset.name}
                onClick={() => setColor(preset.value)}
                className={`h-8 rounded border-2 transition-all ${
                  color === preset.value
                    ? 'border-white scale-110'
                    : 'border-transparent hover:border-[var(--color-border)]'
                }`}
                style={{ backgroundColor: preset.value.replace('0.15', '0.4') }}
                title={preset.name}
              />
            ))}
          </div>
          {/* Custom color picker */}
          <div className="flex items-center gap-2">
            <input
              type="color"
              value={rgbaToHex(color)}
              onChange={(e) => setColor(hexToRgba(e.target.value, 0.15))}
              className="w-8 h-8 rounded cursor-pointer border border-[var(--color-border)] bg-transparent"
              title="Pick custom color"
            />
            <span className="text-xs text-[var(--color-text-muted)]">Custom color</span>
          </div>
        </div>

        {/* Price display */}
        <div className="mb-4 text-sm text-[var(--color-text-muted)]">
          <span>Upper: {zone.upper_price.toFixed(getInstrumentPrecision(instrument))}</span>
          <span className="mx-2">|</span>
          <span>Lower: {zone.lower_price.toFixed(getInstrumentPrecision(instrument))}</span>
        </div>

        {/* Actions */}
        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-sm bg-[var(--color-bg-card)] hover:bg-[var(--color-bg-elevated)] rounded text-[var(--color-text-secondary)]"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            className="px-3 py-1.5 text-sm bg-[var(--color-info)] hover:bg-[var(--color-info)]/80 rounded"
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
