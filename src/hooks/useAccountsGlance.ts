/**
 * Rolling-window performance for every account configured in the wickd CLI,
 * via the `accounts_glance` command.
 *
 * Unlike the calendar/feed readers this one is NOT offline — it reaches OANDA
 * through the CLI and takes ~5s for a full fan-out. So the hook deliberately
 * keeps the last good value on screen while a refresh runs (`loading` is for a
 * subtle indicator, never for blanking the panel), and an error never clears
 * already-rendered data.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Backend TTL is 60s; polling faster just returns the same cached object. */
const REFRESH_INTERVAL_MS = 60 * 1000;

export interface OpenPosition {
  instrument: string;
  /** Net signed units as an exact decimal string (negative = short). */
  units: string;
  unrealized_pl: string;
}

export interface AccountGlance {
  account: string;
  names: string[];
  account_id: string | null;
  /** OANDA's own account name — what the broker dashboard shows. */
  alias?: string | null;
  /**
   * Open positions, [] when flat. Null/undefined when the fetch failed (or the
   * CLI predates the field) — fall back to the bare count, never render "flat".
   */
  open_positions?: OpenPosition[] | null;
  currency: string | null;
  nav: string | null;
  balance: string | null;
  unrealized_pl: string | null;
  open_trade_count: number | null;
  realized: string | null;
  trades: number | null;
  wins: number | null;
  losses: number | null;
  /** Null when nothing was decided in the window — render "—", not 0%. */
  win_rate: number | null;
  /**
   * This row's own window start (RFC3339), or null when the row is
   * unmeasured (`--since-baseline` for an account with no recorded baseline,
   * D3) or the CLI predates the field.
   */
  window_start: string | null;
  /** Which input decided this row's window. Null when the CLI predates the field. */
  window_source: 'baseline' | 'since' | 'days' | null;
  /**
   * Why the row is unmeasured, when it is (e.g. "no baseline recorded").
   * Null/absent on the ordinary path.
   */
  note?: string | null;
  error: string | null;
}

export interface AccountsGlance {
  environment: string;
  /** Null when an explicit `since`/`since_baseline` drove the window (i.e. "Today" or "Since baseline"). */
  days: number | null;
  /**
   * Null under `--since-baseline`: the window is per-account there (D2) —
   * each row's own `window_start` is authoritative and there is no single
   * shared start.
   */
  since: string | null;
  /** The shared exclusive upper bound (D4). Always present once the CLI supports `--to`. */
  to: string | null;
  generated_at: string;
  accounts: AccountGlance[];
}

/**
 * The window the panel is showing.
 *
 * "Today" is not `days: 1` — it is since the viewer's local midnight, which is
 * a different (and, mid-morning, much shorter) span than the last 24 hours.
 * "Was today profitable" is the question this panel exists to answer, so the
 * distinction is load-bearing rather than pedantic.
 *
 * "baseline" is per-account by construction — each account's start is its own
 * recorded baseline instant, resolved entirely by the CLI (D2); the app never
 * computes or reads a baseline itself.
 *
 * "range" is a closed window over two RFC3339 instants: `from` inclusive,
 * `to` exclusive (D4) — see `localDateRangeToInstants` for turning two local
 * calendar dates (what a date picker produces) into this shape.
 */
export type GlanceWindow =
  | { kind: 'today' }
  | { kind: 'days'; days: number }
  | { kind: 'baseline' }
  | { kind: 'range'; from: string; to: string };

/**
 * Start of the viewer's local day as an RFC3339 instant.
 *
 * Computed per fetch rather than once per mount: this app stays open for days
 * at a time, and a midnight captured at mount would silently keep reporting
 * yesterday's P&L as "today" after the date rolls.
 */
export const localMidnightIso = (now: Date = new Date()): string => {
  const midnight = new Date(now);
  midnight.setHours(0, 0, 0, 0);
  return midnight.toISOString();
};

/**
 * Two local calendar dates → the RFC3339 instants a `range` window needs (D4).
 *
 * `from` is local midnight of `startDate`. `to` is local midnight of the day
 * *after* `endDate` — the end date is inclusive for the human picking it,
 * exclusive for the machine consuming it, the same convention
 * `localMidnightIso` already uses for "today". Pure: does not mutate either
 * argument.
 *
 * This is the seam AGT-1132's date-range picker calls into; this ticket only
 * supplies and tests the conversion.
 */
export const localDateRangeToInstants = (
  startDate: Date,
  endDate: Date
): { from: string; to: string } => {
  const dayAfterEnd = new Date(endDate);
  dayAfterEnd.setDate(dayAfterEnd.getDate() + 1);
  return {
    from: localMidnightIso(startDate),
    to: localMidnightIso(dayAfterEnd),
  };
};

/**
 * Human label for a window — what the hero heading / picker show (D7).
 *
 * For `range`, `to` is the exclusive day-after-end instant (D4), so the
 * displayed end date is one local day before it — e.g. `from`/`to` spanning
 * Aug 1 00:00 → Aug 25 00:00 (local) reads "Aug 1 – Aug 24".
 */
export const windowLabel = (w: GlanceWindow): string => {
  switch (w.kind) {
    case 'baseline':
      return 'Since baseline';
    case 'today':
      return 'Today';
    case 'days':
      return w.days === 7 ? 'Last 7d' : w.days === 30 ? 'Last 30d' : `Last ${w.days}d`;
    case 'range': {
      const fmt = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });
      const start = new Date(w.from);
      const end = new Date(w.to);
      end.setDate(end.getDate() - 1);
      return `${fmt.format(start)} – ${fmt.format(end)}`;
    }
  }
};

const isFiniteNumber = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

/**
 * Runtime shape check for a value that claims to be a `GlanceWindow` — used
 * on anything read back from storage (or otherwise untrusted), since a
 * TypeScript type only holds at compile time.
 */
export const isGlanceWindow = (v: unknown): v is GlanceWindow => {
  if (typeof v !== 'object' || v === null) return false;
  const w = v as Record<string, unknown>;
  switch (w.kind) {
    case 'today':
    case 'baseline':
      return true;
    case 'days':
      return isFiniteNumber(w.days) && w.days > 0;
    case 'range':
      return (
        typeof w.from === 'string' &&
        typeof w.to === 'string' &&
        !Number.isNaN(Date.parse(w.from)) &&
        !Number.isNaN(Date.parse(w.to))
      );
    default:
      return false;
  }
};

/** localStorage key the section persists the selected window under (unchanged). */
export const WINDOW_STORAGE_KEY = 'wickd_accounts_window';

/**
 * Legacy string ids this key held before `GlanceWindow` grew the `baseline`/
 * `range` variants — migrated to their equivalent `GlanceWindow` on read
 * (D7); never written back out in this shape.
 */
const LEGACY_WINDOW_IDS: Readonly<Record<string, GlanceWindow>> = {
  today: { kind: 'today' },
  '7d': { kind: 'days', days: 7 },
  '30d': { kind: 'days', days: 30 },
};

export const serializeWindow = (w: GlanceWindow): string => JSON.stringify(w);

/**
 * Parse whatever `wickd_accounts_window` holds: a legacy `today|7d|30d` id, a
 * serialized `GlanceWindow`, or anything else. Unparseable/garbage input
 * reads as `null` rather than throwing — the caller falls back to its own
 * default (D6), same as if nothing had ever been persisted.
 */
export const parseStoredWindow = (raw: string | null): GlanceWindow | null => {
  if (raw === null) return null;
  if (Object.prototype.hasOwnProperty.call(LEGACY_WINDOW_IDS, raw)) {
    return LEGACY_WINDOW_IDS[raw];
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    return isGlanceWindow(parsed) ? parsed : null;
  } catch {
    return null;
  }
};

/**
 * Read the persisted window. Wrapped like every other `localStorage` read in
 * this app (private browsing / disabled storage can throw) — a thrown read
 * must not crash the panel, it just means "nothing persisted".
 */
export const readStoredWindow = (): GlanceWindow | null => {
  try {
    return parseStoredWindow(localStorage.getItem(WINDOW_STORAGE_KEY));
  } catch {
    return null;
  }
};

/** Persist the selected window, wrapped the same way as the read. */
export const persistWindow = (w: GlanceWindow): void => {
  try {
    localStorage.setItem(WINDOW_STORAGE_KEY, serializeWindow(w));
  } catch {
    // Best-effort: the section still works for this session without persistence.
  }
};

/**
 * True when at least one row reflects an actual recorded baseline — the
 * signal D6's cold-boot default hinges on ("the presence of a baseline is
 * the signal" that the standing question has shifted from "was today
 * profitable" to "how is each experiment doing").
 *
 * Meant to be checked against a `since_baseline` glance response; this
 * ticket supplies the pure predicate, AGT-1132 decides when/how to fetch one
 * (e.g. a boot-time probe, or whatever glance data is already in hand).
 */
export const hasAnyBaseline = (accounts: AccountGlance[]): boolean =>
  accounts.some((a) => a.window_source === 'baseline');

/**
 * D6: the cold-boot default is `baseline` when any configured account has one
 * recorded, else `today` — unchanged behaviour for baseline-less setups. A
 * persisted choice (`readStoredWindow`) always wins over this; callers should
 * only reach for `defaultWindow` when nothing is persisted.
 */
export const defaultWindow = (accounts: AccountGlance[]): GlanceWindow =>
  hasAnyBaseline(accounts) ? { kind: 'baseline' } : { kind: 'today' };

export interface UseAccountsGlance {
  data: AccountsGlance | null;
  /** Set only when there is nothing to show; a refresh failure keeps stale data. */
  error: string | null;
  loading: boolean;
  refresh: () => void;
}

// Param is `glanceWindow`, not `window` — shadowing the global would make a
// later `window.localStorage` in this hook fail in a very confusing way.
export const useAccountsGlance = (glanceWindow: GlanceWindow): UseAccountsGlance => {
  const [data, setData] = useState<AccountsGlance | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  // Read inside load() so a manual refresh doesn't need `data` as a dependency
  // (which would tear down the poll interval on every successful fetch).
  const hasData = useRef(false);

  // Depend on the window's primitive fields, not the object: a caller passing
  // an inline `{ kind: 'days', days: 7 }` creates a new object every render,
  // which would rebuild `load` and tear down the poll interval each time.
  const kind = glanceWindow.kind;
  const days = glanceWindow.kind === 'days' ? glanceWindow.days : null;
  const from = glanceWindow.kind === 'range' ? glanceWindow.from : null;
  const to = glanceWindow.kind === 'range' ? glanceWindow.to : null;

  const load = useCallback(
    async (force: boolean) => {
      setLoading(true);
      try {
        const result = await invoke<AccountsGlance>('accounts_glance', {
          days: kind === 'days' ? days : null,
          // Recomputed per call so a long-lived "today" window follows the
          // date over; `range`'s instants are caller-supplied and fixed.
          since: kind === 'today' ? localMidnightIso() : kind === 'range' ? from : null,
          to: kind === 'range' ? to : null,
          sinceBaseline: kind === 'baseline' ? true : null,
          refresh: force,
        });
        setData(result);
        hasData.current = true;
        setError(null);
      } catch (e) {
        // Keep the last good render; only surface the error on an empty panel.
        if (!hasData.current) setError(String(e));
      } finally {
        setLoading(false);
      }
    },
    [kind, days, from, to]
  );

  useEffect(() => {
    // Deliberate: changing the window discards the previous result rather than
    // holding it on screen while the new one loads. Keeping it would render the
    // 7d numbers underneath a "30d" label — briefly, but wrongly. A momentary
    // "Loading accounts…" is the honest render, and the backend caches per
    // (env, days, since, to, since_baseline), so switching back is instant
    // inside the TTL.
    hasData.current = false;
    void load(false);
    const interval = setInterval(() => void load(false), REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [load]);

  const refresh = useCallback(() => void load(true), [load]);

  return { data, error, loading, refresh };
};
