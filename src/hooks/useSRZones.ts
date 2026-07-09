import { useState, useCallback, useRef, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  listSRZones,
  saveSRZone,
  deleteSRZone,
  clearSRZones,
  type LocalSRZone,
} from '../lib/localStore';
import type { SRZone } from '../components/charts/SRZoneOverlay';
import type { PivotLevel } from '../components/charts/chartTypes';
import { getInstrumentPrecision } from '../components/charts/chartConstants';

interface UseSRZonesOptions {
  instrument: string;
  isMainChart: boolean;
  /** Candle series ref for clearing preview lines on Escape (Bug #5) */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  candleSeriesRef?: React.RefObject<any>;
}

interface UseSRZonesResult {
  // Zone data
  srZones: SRZone[];

  // Editing state
  srEditingMode: boolean;
  setSrEditingMode: (mode: boolean) => void;
  pendingZoneBoundary: number | null;
  setPendingZoneBoundary: (boundary: number | null) => void;
  secondBoundary: number | null;
  setSecondBoundary: (boundary: number | null) => void;
  selectedZoneId: string | null;
  setSelectedZoneId: (id: string | null) => void;
  editingZone: SRZone | null;
  setEditingZone: (zone: SRZone | null) => void;
  previewZone: { upperY: number; lowerY: number } | null;
  setPreviewZone: (zone: { upperY: number; lowerY: number } | null) => void;

  // Edge resizing state
  resizingEdge: { zoneId: string; edge: 'upper' | 'lower'; startPrice: number } | null;
  setResizingEdge: (edge: { zoneId: string; edge: 'upper' | 'lower'; startPrice: number } | null) => void;

  // Menu state
  srMenuOpen: boolean;
  setSrMenuOpen: (open: boolean) => void;
  confirmClearAll: boolean;
  setConfirmClearAll: (confirm: boolean) => void;

  // Preview line refs
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  previewLineRef: React.RefObject<any>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  firstBoundaryLineRef: React.RefObject<any>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  secondBoundaryLineRef: React.RefObject<any>;

  // Pivot import state
  importingPivots: boolean;

  // Error state
  error: string | null;
  setError: (error: string | null) => void;

  // Handlers
  handleDeleteSRZone: (zoneId: string) => Promise<void>;
  handleEditZone: (zone: SRZone) => void;
  handleUpdateZone: (zoneId: string, updates: { label?: string; color?: string; upper_price?: string; lower_price?: string }) => Promise<void>;
  handleClearAllZones: () => Promise<void>;
  handleImportPivots: (timeframe: 'daily' | 'weekly') => Promise<void>;
  handleSaveZone: () => Promise<void>;
  clearPreviewLines: (candleSeries: { removePriceLine: (line: unknown) => void } | null) => void;
}

/**
 * S/R zones for one instrument, served from the local store (AGT-646).
 *
 * Previously Zero-backed (`mySRZonesByInstrument` + `zero.mutate.sr_zone`);
 * now every read/write goes through `src/lib/localStore.ts`, so zones work
 * fully offline with no sign-in. Mutations reload the list from the store —
 * the store is the single source of truth, not optimistic local state.
 */
export const useSRZones = ({
  instrument,
  isMainChart,
  candleSeriesRef,
}: UseSRZonesOptions): UseSRZonesResult => {
  // Zone rows from the local store for the current instrument.
  const [zoneRows, setZoneRows] = useState<LocalSRZone[]>([]);

  const refreshZones = useCallback(async () => {
    try {
      setZoneRows(await listSRZones(instrument));
    } catch (err) {
      console.error('[useSRZones] Failed to load S/R zones:', err);
    }
  }, [instrument]);

  // Load zones on mount and whenever the instrument changes.
  useEffect(() => {
    setZoneRows([]);
    refreshZones();
  }, [refreshZones]);

  const srZones: SRZone[] = zoneRows.map(z => ({
    id: z.id,
    upper_price: parseFloat(z.upper_price),
    lower_price: parseFloat(z.lower_price),
    label: z.label || undefined,
    color: z.color || undefined,
  }));

  // Editing state
  const [srEditingMode, setSrEditingMode] = useState(false);
  const [pendingZoneBoundary, setPendingZoneBoundary] = useState<number | null>(null);
  const [secondBoundary, setSecondBoundary] = useState<number | null>(null);
  const [selectedZoneId, setSelectedZoneId] = useState<string | null>(null);
  const [editingZone, setEditingZone] = useState<SRZone | null>(null);
  const [previewZone, setPreviewZone] = useState<{ upperY: number; lowerY: number } | null>(null);

  // Edge resizing state
  const [resizingEdge, setResizingEdge] = useState<{ zoneId: string; edge: 'upper' | 'lower'; startPrice: number } | null>(null);

  // Menu state
  const [srMenuOpen, setSrMenuOpen] = useState(false);
  const [confirmClearAll, setConfirmClearAll] = useState(false);

  // Preview line refs
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const previewLineRef = useRef<any>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const firstBoundaryLineRef = useRef<any>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const secondBoundaryLineRef = useRef<any>(null);

  // Pivot import state
  const [importingPivots, setImportingPivots] = useState(false);

  // Error state
  const [error, setError] = useState<string | null>(null);

  // Handle S/R zone deletion
  const handleDeleteSRZone = useCallback(async (zoneId: string) => {
    try {
      await deleteSRZone(zoneId);
      await refreshZones();
    } catch (err) {
      console.error('Failed to delete S/R zone:', err);
    }
  }, [refreshZones]);

  // Handle S/R zone edit (open editor)
  const handleEditZone = useCallback((zone: SRZone) => {
    setEditingZone(zone);
  }, []);

  // Handle S/R zone update (save changes). The store upsert is whole-row, so
  // merge the partial updates over the current row before saving.
  const handleUpdateZone = useCallback(async (zoneId: string, updates: { label?: string; color?: string; upper_price?: string; lower_price?: string }) => {
    const current = zoneRows.find(z => z.id === zoneId);
    if (!current) return;
    try {
      await saveSRZone({
        ...current,
        ...(updates.label !== undefined && { label: updates.label }),
        ...(updates.color !== undefined && { color: updates.color }),
        ...(updates.upper_price !== undefined && { upper_price: updates.upper_price }),
        ...(updates.lower_price !== undefined && { lower_price: updates.lower_price }),
        updated_at: Date.now(),
      });
      await refreshZones();
    } catch (err) {
      console.error('Failed to update S/R zone:', err);
    }
  }, [zoneRows, refreshZones]);

  // Clear all S/R zones for this instrument
  const handleClearAllZones = useCallback(async () => {
    if (zoneRows.length === 0) return;
    try {
      await clearSRZones(instrument);
      await refreshZones();
      setConfirmClearAll(false);
      setSrMenuOpen(false);
    } catch (err) {
      console.error('Failed to clear S/R zones:', err);
    }
  }, [instrument, zoneRows.length, refreshZones]);

  // Import pivot point zones
  const handleImportPivots = useCallback(async (timeframe: 'daily' | 'weekly' = 'daily') => {
    if (!instrument || importingPivots) return;

    setImportingPivots(true);
    setError(null);

    try {
      const pivots = await invoke<PivotLevel[]>('calculate_pivot_points', {
        instrument,
        timeframe,
      });

      if (pivots.length === 0) {
        setError('No pivot points calculated');
        setImportingPivots(false);
        return;
      }

      // Create thin zones from pivot levels
      // Use a small buffer to make lines visible (approximately 0.5 pips for major pairs)
      const pipBuffer = 0.00005; // Half a pip buffer on each side
      const now = Date.now();

      for (const pivot of pivots) {
        // Color based on level type
        let color: string;
        if (pivot.level_type === 'pivot') {
          color = 'rgba(234, 179, 8, 0.25)';  // Yellow for pivot
        } else if (pivot.level_type === 'resistance') {
          color = 'rgba(239, 68, 68, 0.20)';  // Red for resistance
        } else {
          color = 'rgba(34, 197, 94, 0.20)';  // Green for support
        }

        await saveSRZone({
          id: crypto.randomUUID(),
          instrument,
          upper_price: (pivot.price + pipBuffer).toString(),
          lower_price: (pivot.price - pipBuffer).toString(),
          label: pivot.label ?? null,
          color,
          created_at: now,
          updated_at: now,
        });
      }
      await refreshZones();
    } catch (err) {
      console.error('Failed to import pivot points:', err);
      setError(`Failed to calculate pivots: ${err}`);
    } finally {
      setImportingPivots(false);
    }
  }, [instrument, importingPivots, refreshZones]);

  // Clear all preview lines and preview zone state
  const clearPreviewLines = useCallback((candleSeries: { removePriceLine: (line: unknown) => void } | null) => {
    if (candleSeries) {
      if (previewLineRef.current) {
        candleSeries.removePriceLine(previewLineRef.current);
        previewLineRef.current = null;
      }
      if (firstBoundaryLineRef.current) {
        candleSeries.removePriceLine(firstBoundaryLineRef.current);
        firstBoundaryLineRef.current = null;
      }
      if (secondBoundaryLineRef.current) {
        candleSeries.removePriceLine(secondBoundaryLineRef.current);
        secondBoundaryLineRef.current = null;
      }
    }
    // Clear preview zone state
    setPreviewZone(null);
  }, []);

  // Save a new S/R zone
  const handleSaveZone = useCallback(async () => {
    if (pendingZoneBoundary === null || secondBoundary === null) return;

    const upperPrice = Math.max(pendingZoneBoundary, secondBoundary);
    const lowerPrice = Math.min(pendingZoneBoundary, secondBoundary);
    const precision = getInstrumentPrecision(instrument);

    try {
      const now = Date.now();
      await saveSRZone({
        id: crypto.randomUUID(),
        instrument,
        upper_price: upperPrice.toFixed(precision),
        lower_price: lowerPrice.toFixed(precision),
        label: null,
        color: null,
        created_at: now,
        updated_at: now,
      });
      await refreshZones();

      // Reset editing state (caller needs to clear preview lines)
      setPendingZoneBoundary(null);
      setSecondBoundary(null);
      setSrEditingMode(false);
      setPreviewZone(null);
    } catch (err) {
      console.error('[useSRZones] Failed to save S/R zone:', err);
    }
  }, [instrument, pendingZoneBoundary, secondBoundary, refreshZones]);

  // Handle escape key to cancel drawing
  useEffect(() => {
    if (!isMainChart) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && srEditingMode) {
        // Clear preview lines directly if candleSeriesRef is available (Bug #5)
        if (candleSeriesRef?.current) {
          clearPreviewLines(candleSeriesRef.current);
        }
        setPendingZoneBoundary(null);
        setSecondBoundary(null);
        setSrEditingMode(false);
        setPreviewZone(null);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  // clearPreviewLines is stable (useCallback with no deps), candleSeriesRef is a ref (stable identity)
  }, [srEditingMode, isMainChart, clearPreviewLines, candleSeriesRef]);

  return {
    // Zone data
    srZones,

    // Editing state
    srEditingMode,
    setSrEditingMode,
    pendingZoneBoundary,
    setPendingZoneBoundary,
    secondBoundary,
    setSecondBoundary,
    selectedZoneId,
    setSelectedZoneId,
    editingZone,
    setEditingZone,
    previewZone,
    setPreviewZone,
    resizingEdge,
    setResizingEdge,

    // Menu state
    srMenuOpen,
    setSrMenuOpen,
    confirmClearAll,
    setConfirmClearAll,

    // Preview line refs
    previewLineRef,
    firstBoundaryLineRef,
    secondBoundaryLineRef,

    // Pivot import state
    importingPivots,

    // Error state
    error,
    setError,

    // Handlers
    handleDeleteSRZone,
    handleEditZone,
    handleUpdateZone,
    handleClearAllZones,
    handleImportPivots,
    handleSaveZone,
    clearPreviewLines,
  };
};
