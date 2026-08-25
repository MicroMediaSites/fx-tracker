/**
 * Accounts — the dashboard's lead block on the HOME window.
 *
 * Shape is deliberate, and deliberately NOT a list. The window is opened to
 * answer one question ("was today profitable?"), so that answer is a hero
 * figure you perceive rather than read, and the per-account breakdown is a
 * tile grid underneath it — six tiles across two rows scan in one saccade,
 * where six full-width rows have to be read top to bottom.
 *
 * Reads `accounts_glance`, which shells out to `wickd trade glance`. The CLI
 * owns credentials + OANDA; this only renders.
 *
 * Two honesty constraints drive the numbers:
 *
 *  1. The window figure is REALIZED P&L only — wickd stores no NAV time
 *     series, so a position opened before the window and still open
 *     contributes nothing to it. Open P&L is reported separately and is
 *     as-of-now, never folded into the window total.
 *  2. A null win rate renders "—", never "0%": nothing decided is not the
 *     same as losing everything.
 *  3. An account the window cannot measure renders "no baseline", never
 *     "$0.00" (D3). $0.00 means "traded flat"; this account is unmeasured.
 */
import { Fragment, ReactNode, useEffect, useState } from 'react';
import { AccountHistoryModal } from './AccountHistoryModal';
import {
  AccountGlance,
  GlanceWindow,
  defaultWindow,
  localDateRangeToInstants,
  persistWindow,
  readStoredWindow,
  useAccountsGlance,
  windowLabel,
} from '../../hooks/useAccountsGlance';
import { summarizeAccounts } from './accountsSummary';

/**
 * Picker order is D7's: since baseline · today · 7d · 30d · (custom…, which
 * is not a preset — it reveals the two date inputs below).
 *
 * "Today" is deliberately not `24h`: before mid-afternoon those are very
 * different spans, and the one you want is the calendar day.
 */
const PRESETS: { id: string; label: string; window: GlanceWindow; title: string }[] = [
  {
    id: 'baseline',
    label: 'since baseline',
    window: { kind: 'baseline' },
    title: "Each account from its own recorded baseline (wickd trade baseline)",
  },
  { id: 'today', label: 'today', window: { kind: 'today' }, title: 'Since your local midnight' },
  { id: '7d', label: '7d', window: { kind: 'days', days: 7 }, title: 'The last 7 days' },
  { id: '30d', label: '30d', window: { kind: 'days', days: 30 }, title: 'The last 30 days' },
];

/** Which picker button reads as pressed for a given window (`custom` for a range). */
export const presetId = (w: GlanceWindow): string => {
  switch (w.kind) {
    case 'baseline':
      return 'baseline';
    case 'today':
      return 'today';
    case 'days':
      return `${w.days}d`;
    case 'range':
      return 'custom';
  }
};

/**
 * D6, first half: what the section shows before any glance has landed.
 *
 * A persisted choice always wins. With nothing persisted we open on
 * `baseline` — the answer to "does any account actually have one" only exists
 * in a since-baseline response, so the first fetch doubles as the probe and
 * `defaultWindow` decides from its rows (see the resolution effect below).
 */
export const initialWindow = (stored: GlanceWindow | null): GlanceWindow =>
  stored ?? { kind: 'baseline' };

/**
 * D3: a healthy row the window has no figure for — under `--since-baseline`,
 * an account with no baseline recorded. The CLI pairs this with a `note`
 * ("no baseline recorded"); the null realized is the load-bearing signal,
 * because it is the same one the hero total excludes on, and the tile and the
 * hero must never disagree about which accounts count.
 */
export const isUnmeasured = (a: AccountGlance): boolean => !a.error && a.realized === null;

/** The one-line fix a "no baseline" tile tells you to run (D3). */
export const baselineHint = (account: string): string =>
  `wickd trade baseline set --account ${account}`;

const dayMonth = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });

/**
 * A tile's own window start as "since Aug 25" (D5) — under `baseline` every
 * tile's start differs, which is the point of the footer. Null when the row
 * carries no start (unmeasured, or a CLI that predates the field), so the
 * footer is omitted rather than guessed at.
 */
export const sinceLabel = (windowStart: string | null | undefined): string | null => {
  if (!windowStart) return null;
  const at = new Date(windowStart);
  return Number.isNaN(at.getTime()) ? null : `since ${dayMonth.format(at)}`;
};

/** A `Date` as the local `YYYY-MM-DD` an `<input type="date">` holds. */
export const toDateInput = (d: Date): string => {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

/**
 * `YYYY-MM-DD` → local midnight of that day, or null if it isn't a real date.
 *
 * Deliberately not `new Date(value)`: the bare-date form is specified to
 * parse as UTC, which lands on the previous calendar day for every viewer
 * west of Greenwich — exactly the off-by-one D4 exists to avoid.
 */
export const parseDateInput = (value: string): Date | null => {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  const [year, month, day] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const date = new Date(year, month - 1, day);
  // Rejects the overflow `new Date` is happy to roll over (2026-02-31 → Mar 3).
  return date.getMonth() === month - 1 && date.getDate() === day ? date : null;
};

/**
 * The two date inputs → a `range` window, or null when they don't make one
 * (either is unparseable, or the start is after the end). The instants come
 * from `localDateRangeToInstants`, so the end date stays inclusive for the
 * human who picked it (D4).
 */
export const rangeFromInputs = (start: string, end: string): GlanceWindow | null => {
  const from = parseDateInput(start);
  const to = parseDateInput(end);
  if (!from || !to || from.getTime() > to.getTime()) return null;
  return { kind: 'range', ...localDateRangeToInstants(from, to) };
};

/**
 * The inverse: the two local dates a `range` window came from, so reopening
 * `custom…` shows what is actually on screen. `to` is the exclusive
 * day-after-end instant, so the end input is one local day before it.
 */
export const rangeToInputs = (w: GlanceWindow): { start: string; end: string } | null => {
  if (w.kind !== 'range') return null;
  const end = new Date(w.to);
  end.setDate(end.getDate() - 1);
  return { start: toDateInput(new Date(w.from)), end: toDateInput(end) };
};

/** A week ending today — what `custom…` opens on before anything is picked. */
const defaultRangeInputs = (): { start: string; end: string } => {
  const end = new Date();
  const start = new Date();
  start.setDate(start.getDate() - 6);
  return { start: toDateInput(start), end: toDateInput(end) };
};

/** Interleave " · " between the parts of the hero's count line that apply. */
const separated = (parts: ReactNode[]): ReactNode[] =>
  parts.map((part, i) => (
    <Fragment key={i}>
      {i > 0 && ' · '}
      {part}
    </Fragment>
  ));

/**
 * Exact decimal strings cross from the CLI; parsed to numbers for DISPLAY
 * only, never to compute anything that is stored or reconciled.
 */
const money = (value: number | null, currency: string | null, signed = false): string => {
  if (value === null || !Number.isFinite(value)) return '—';
  const formatted = new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: currency || 'USD',
    currencyDisplay: 'narrowSymbol',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(Math.abs(value));
  if (!signed) return formatted;
  // Explicit sign: at a glance the direction matters more than the magnitude,
  // and a bare "-" is easy to miss against a currency symbol.
  return `${value > 0 ? '+' : value < 0 ? '−' : ''}${formatted}`;
};

const parse = (v: string | null): number | null => {
  if (v === null) return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
};

/** Zero is neutral, not green — a flat window shouldn't read as a win. */
const pnlColor = (value: number | null): string => {
  if (value === null || !Number.isFinite(value) || value === 0) {
    return 'text-[var(--color-text-muted)]';
  }
  return value > 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]';
};

const percent = (rate: number | null): string =>
  rate === null ? '—' : `${Math.round(rate * 100)}%`;

/**
 * Last three digits of the OANDA account id — the sub-account number Matt
 * thinks in (`003`, `004`, …). OANDA ids look like `101-001-26151603-005`; the
 * final hyphen-group is the sub-account. Falls back to the whole tail if the
 * shape is unexpected rather than guessing.
 */
export const accountSuffix = (accountId: string | null): string | null => {
  if (!accountId) return null;
  const last = accountId.split('-').pop() ?? accountId;
  return last.slice(-3);
};

/** True when the account neither traded in the window nor holds anything open. */
export const isIdle = (a: AccountGlance): boolean => {
  const openPl = parse(a.unrealized_pl);
  const hasOpen = (a.open_trade_count ?? 0) > 0 || (openPl !== null && openPl !== 0);
  return (a.trades ?? 0) === 0 && !hasOpen;
};

/**
 * Accounts that did something first, then idle ones, each group keeping the
 * CLI's stable order. Errored rows rank with the active group — a broken
 * account is something to look at, not something to bury.
 */
export const orderedAccounts = (accounts: AccountGlance[]): AccountGlance[] => {
  const rank = (a: AccountGlance) => (a.error ? 0 : isIdle(a) ? 1 : 0);
  return [...accounts].sort((a, b) => rank(a) - rank(b));
};

/**
 * One account as a compact tile. The P&L is the tile's largest element —
 * scanning the grid should surface which accounts moved without reading any
 * labels.
 */
/** `USD_JPY` → `USD/JPY`, matching how OANDA's dashboard writes pairs. */
export const pairLabel = (instrument: string): string => instrument.replace('_', '/');

/**
 * `+2k` / `−2k` from an exact units string — direction at a glance without
 * spending tile width on the word "long"/"short".
 */
export const unitsLabel = (units: string): string => {
  const n = Number(units);
  if (!Number.isFinite(n) || n === 0) return '';
  const abs = Math.abs(n);
  const compact = abs >= 1000 && abs % 100 === 0 ? `${abs / 1000}k` : String(abs);
  return `${n > 0 ? '+' : '−'}${compact}`;
};

const AccountTile = ({ acct, onOpen }: { acct: AccountGlance; onOpen: (account: string) => void }) => {
  const aliases = acct.names.slice(1);
  const suffix = accountSuffix(acct.account_id);

  if (acct.error) {
    return (
      <div
        data-testid="account-tile"
        className="px-3 py-2.5 rounded-lg border border-[var(--color-sell)]/30 bg-[var(--color-bg-elevated)] min-w-0"
      >
        <div className="flex items-baseline gap-1.5">
          <span className="text-xs font-mono text-[var(--color-text-secondary)] truncate">
            {acct.account}
          </span>
          {suffix && (
            <span className="text-[10px] font-mono text-[var(--color-text-faint)] shrink-0">
              {suffix}
            </span>
          )}
        </div>
        <div className="mt-1 text-sm text-[var(--color-sell)]" title={acct.error}>
          unavailable
        </div>
        <div
          className="text-[11px] text-[var(--color-text-faint)] truncate"
          data-testid="account-error"
          title={acct.error}
        >
          {acct.error}
        </div>
      </div>
    );
  }

  const realized = parse(acct.realized);
  const openPl = parse(acct.unrealized_pl);
  const unmeasured = isUnmeasured(acct);
  const idle = !unmeasured && isIdle(acct);
  // Both states recede the same way: neither has a number worth scanning for.
  const muted = idle || unmeasured;
  const since = sinceLabel(acct.window_start);

  return (
    <button
      type="button"
      data-testid="account-tile"
      data-idle={idle || undefined}
      data-unmeasured={unmeasured || undefined}
      onClick={() => onOpen(acct.account)}
      title="View trade history"
      className={`text-left px-3 py-2.5 rounded-lg border bg-[var(--color-bg-elevated)] min-w-0 transition-colors hover:border-[var(--color-info)]/50 hover:bg-[var(--color-bg-elevated)]/80 focus:outline-none focus:border-[var(--color-info)] ${
        muted
          ? 'border-[var(--color-border)]/60 opacity-50 hover:opacity-80'
          : 'border-[var(--color-border)]'
      }`}
    >
      {/* OANDA's own account name leads — it is what the broker dashboard
          shows, so the tile and the OANDA UI can be matched by eye. The
          internal config name drops to the small mono line below it. */}
      <div className="flex items-baseline gap-1.5 min-w-0">
        <span
          data-testid="account-title"
          className="text-xs text-[var(--color-text-secondary)] truncate"
          title={acct.alias ? `OANDA account name: ${acct.alias}` : undefined}
        >
          {acct.alias || acct.account}
        </span>
        {aliases.length > 0 && (
          <span
            className="text-[11px] text-[var(--color-text-faint)] shrink-0"
            title={`Also configured as ${aliases.join(', ')} — same OANDA account`}
          >
            +{aliases.length}
          </span>
        )}
      </div>
      <div className="flex items-baseline gap-1.5 min-w-0">
        <span className="text-[10px] font-mono text-[var(--color-text-faint)] truncate">
          {acct.alias ? acct.account : ''}
        </span>
        {suffix && (
          <span
            className="text-[10px] font-mono text-[var(--color-text-faint)] shrink-0"
            title={`OANDA sub-account ${suffix}`}
          >
            {suffix}
          </span>
        )}
      </div>

      {/* The tile's headline. Tabular figures so a column of tiles aligns.
          Unmeasured accounts get words instead: there is no figure, and
          "$0.00" would claim the account traded flat (D3). */}
      {unmeasured ? (
        <div
          data-testid="account-no-baseline"
          className="mt-0.5 text-lg font-semibold text-[var(--color-text-muted)] truncate"
          title={`run ${baselineHint(acct.account)}`}
        >
          no baseline
        </div>
      ) : (
        <div
          data-testid="account-realized"
          className={`mt-0.5 text-lg font-semibold font-mono tabular-nums truncate ${pnlColor(realized)}`}
          title="Realized P&L over the selected window"
        >
          {money(realized, acct.currency, true)}
        </div>
      )}

      <div className="mt-0.5 text-[11px] text-[var(--color-text-muted)] truncate">
        {unmeasured ? (
          <span className="font-mono" title={`run ${baselineHint(acct.account)}`}>
            {baselineHint(acct.account)}
          </span>
        ) : idle ? (
          'no activity'
        ) : (
          <>
            {acct.trades ?? 0}t · {percent(acct.win_rate)}
            {/* Fallback when the CLI couldn't list instruments (null) but the
                account does hold something: the old bare open P&L. */}
            {!acct.open_positions?.length && openPl !== null && openPl !== 0 && (
              <>
                {' · '}
                <span className={pnlColor(openPl)}>
                  {money(openPl, acct.currency, true)} open
                </span>
              </>
            )}
          </>
        )}
      </div>

      {/* Open positions by name — "open (2)" in the hero is findable here. */}
      {(acct.open_positions?.length ?? 0) > 0 && (
        <div data-testid="account-open-positions" className="mt-0.5 space-y-px">
          {acct.open_positions!.slice(0, 2).map((p) => (
            <div
              key={p.instrument}
              className="text-[11px] font-mono tabular-nums truncate text-[var(--color-text-muted)]"
            >
              {unitsLabel(p.units)} {pairLabel(p.instrument)}
              {' · '}
              <span className={pnlColor(parse(p.unrealized_pl))}>
                {money(parse(p.unrealized_pl), acct.currency, true)}
              </span>
            </div>
          ))}
          {acct.open_positions!.length > 2 && (
            <div className="text-[10px] text-[var(--color-text-faint)]">
              +{acct.open_positions!.length - 2} more open
            </div>
          )}
        </div>
      )}

      {/* This tile's OWN window start (D5). Under "since baseline" every tile
          starts somewhere different, so the section label alone can't say
          what a given figure covers — the exact instant is on hover. */}
      {since && (
        <div
          data-testid="account-since"
          className="mt-1 text-[10px] text-[var(--color-text-faint)] truncate"
          title={acct.window_start ?? undefined}
        >
          {since}
        </div>
      )}
    </button>
  );
};

export const AccountsSection = () => {
  // Read storage once, on mount: the same value decides both the opening
  // window and whether the D6 default still needs resolving.
  const [stored] = useState<GlanceWindow | null>(readStoredWindow);
  const opening = initialWindow(stored);

  const [selected, setSelected] = useState<GlanceWindow>(opening);
  const [defaultResolved, setDefaultResolved] = useState(stored !== null);

  const { data, error, loading, refresh } = useAccountsGlance(selected);

  // D6, second half. Whether any account has a baseline is only knowable from
  // a since-baseline response, so the opening fetch doubles as the probe and
  // `defaultWindow` drops back to `today` when its rows show none. Deliberately
  // NOT persisted — this is a derived default, not a choice, so a later boot
  // re-derives it against whatever baselines exist by then.
  const accounts = data?.accounts;
  useEffect(() => {
    if (defaultResolved || !accounts) return;
    setSelected(defaultWindow(accounts));
    setDefaultResolved(true);
  }, [defaultResolved, accounts]);

  const selectWindow = (next: GlanceWindow) => {
    setSelected(next);
    persistWindow(next);
    // An explicit choice always wins — including one made before the probe
    // lands, which would otherwise be overwritten by the derived default.
    setDefaultResolved(true);
  };

  // The custom-range editor: open when a range is already selected, so
  // reopening the section shows the dates actually on screen.
  const [customOpen, setCustomOpen] = useState(opening.kind === 'range');
  const [rangeInputs, setRangeInputs] = useState(
    () => rangeToInputs(opening) ?? defaultRangeInputs()
  );
  const pendingRange = rangeFromInputs(rangeInputs.start, rangeInputs.end);
  const activeId = presetId(selected);

  // Which account's trade-history drill-down is open (null = none).
  const [openAccount, setOpenAccount] = useState<string | null>(null);

  const summary = data ? summarizeAccounts(data.accounts) : null;
  const asOf = data
    ? new Date(data.generated_at).toLocaleTimeString(undefined, {
        hour: '2-digit',
        minute: '2-digit',
      })
    : null;

  return (
    <section data-testid="accounts-dashboard">
      {/* ── Hero: the one number the window exists to show ───────────────── */}
      <div className="flex items-start justify-between gap-4 flex-wrap">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h2
              data-testid="accounts-window-label"
              className="text-[11px] font-medium uppercase tracking-wider text-[var(--color-text-muted)]"
            >
              {windowLabel(selected)}
            </h2>
            {loading && (
              <span className="text-[11px] text-[var(--color-text-faint)]">updating…</span>
            )}
          </div>

          {/* No measured account means no total to show. "$0.00" across zero
              contributing accounts would read as a flat day rather than as
              nothing to add up (D3). */}
          {summary === null || summary.measured === 0 ? (
            <div
              data-testid="accounts-hero-none"
              className="mt-1 text-4xl font-semibold font-mono text-[var(--color-text-faint)]"
            >
              —
            </div>
          ) : summary.mixedCurrency ? (
            // Never invent a total across currencies: adding USD to JPY yields
            // a confident, meaningless number. Say why instead.
            <div
              data-testid="accounts-hero-mixed"
              className="mt-1 text-lg text-[var(--color-text-muted)]"
            >
              accounts report different currencies — see each below
            </div>
          ) : (
            <div
              data-testid="accounts-hero"
              className={`mt-1 text-5xl font-semibold font-mono tabular-nums leading-none ${pnlColor(summary.realized)}`}
            >
              {money(summary.realized, summary.currency, true)}
            </div>
          )}

          {summary && !summary.mixedCurrency && (
            <div
              data-testid="accounts-summary-line"
              className="mt-2 text-xs text-[var(--color-text-muted)]"
            >
              {separated([
                // Only the accounts actually behind the total are named here —
                // an unmeasured or unreachable account contributed nothing to
                // it, so counting them would overstate what the figure covers.
                ...(summary.measured > 0
                  ? [
                      <>
                        realized across {summary.measured}{' '}
                        {summary.measured === 1 ? 'account' : 'accounts'}
                      </>,
                      <>
                        {summary.trades} {summary.trades === 1 ? 'trade' : 'trades'}
                      </>,
                      <>{percent(summary.winRate)} won</>,
                    ]
                  : []),
                ...(summary.openTrades > 0
                  ? [
                      <span
                        data-testid="accounts-open-summary"
                        className="cursor-help"
                        title={
                          data?.accounts
                            .flatMap((a) =>
                              (a.open_positions ?? []).map(
                                (p) =>
                                  `${a.alias || a.account}: ${unitsLabel(p.units)} ${pairLabel(p.instrument)} (${p.unrealized_pl})`
                              )
                            )
                            .join('\n') || undefined
                        }
                      >
                        <span className={pnlColor(summary.openPl)}>
                          {money(summary.openPl, summary.currency, true)}
                        </span>{' '}
                        open ({summary.openTrades})
                      </span>,
                    ]
                  : []),
                ...(summary.unmeasured > 0
                  ? [
                      <span
                        data-testid="accounts-unmeasured-summary"
                        className="cursor-help"
                        title="No baseline recorded — this window has no figure for them, so they are excluded from the total"
                      >
                        {summary.unmeasured} no baseline
                      </span>,
                    ]
                  : []),
                ...(summary.errored > 0
                  ? [
                      <span className="text-[var(--color-sell)]">
                        {summary.errored} unavailable
                      </span>,
                    ]
                  : []),
              ])}
            </div>
          )}
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <div className="flex items-center gap-0.5" role="group" aria-label="Performance window">
            {PRESETS.map((w) => (
              <button
                key={w.id}
                data-testid={`accounts-window-${w.id}`}
                onClick={() => {
                  setCustomOpen(false);
                  selectWindow(w.window);
                }}
                aria-pressed={w.id === activeId}
                title={w.title}
                className={`px-2 py-0.5 text-xs rounded font-mono transition-colors ${
                  w.id === activeId
                    ? 'bg-[var(--color-info)]/15 text-[var(--color-info)]'
                    : 'text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]'
                }`}
              >
                {w.label}
              </button>
            ))}
            {/* Not a preset: it reveals the two date inputs below rather than
                selecting a window, because a range needs both ends first. */}
            <button
              data-testid="accounts-window-custom"
              onClick={() => setCustomOpen((open) => !open)}
              aria-pressed={activeId === 'custom'}
              aria-expanded={customOpen}
              title="A custom date range"
              className={`px-2 py-0.5 text-xs rounded font-mono transition-colors ${
                activeId === 'custom'
                  ? 'bg-[var(--color-info)]/15 text-[var(--color-info)]'
                  : 'text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]'
              }`}
            >
              custom…
            </button>
          </div>
          <button
            data-testid="accounts-refresh"
            onClick={refresh}
            disabled={loading}
            title={asOf ? `As of ${asOf} — click to refresh` : 'Refresh'}
            className="px-1.5 py-0.5 text-xs rounded text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] disabled:opacity-50 transition-colors"
          >
            ↻
          </button>
        </div>
      </div>

      {/* ── Custom range: two local calendar dates, end inclusive (D4) ────── */}
      {customOpen && (
        <div
          data-testid="accounts-range"
          className="mt-3 flex items-center gap-2 flex-wrap text-xs text-[var(--color-text-muted)]"
        >
          <label className="flex items-center gap-1.5">
            <span>from</span>
            <input
              type="date"
              data-testid="accounts-range-start"
              aria-label="Range start date"
              value={rangeInputs.start}
              onChange={(e) => setRangeInputs((r) => ({ ...r, start: e.target.value }))}
              className="px-1.5 py-0.5 rounded font-mono bg-[var(--color-bg-elevated)] border border-[var(--color-border)] text-[var(--color-text-secondary)]"
            />
          </label>
          <label className="flex items-center gap-1.5">
            <span title="Inclusive — the whole of this day counts">to</span>
            <input
              type="date"
              data-testid="accounts-range-end"
              aria-label="Range end date (inclusive)"
              value={rangeInputs.end}
              onChange={(e) => setRangeInputs((r) => ({ ...r, end: e.target.value }))}
              className="px-1.5 py-0.5 rounded font-mono bg-[var(--color-bg-elevated)] border border-[var(--color-border)] text-[var(--color-text-secondary)]"
            />
          </label>
          <button
            data-testid="accounts-range-apply"
            onClick={() => pendingRange && selectWindow(pendingRange)}
            disabled={pendingRange === null}
            className="px-2 py-0.5 rounded font-mono text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] disabled:opacity-40 disabled:hover:text-[var(--color-text-muted)] transition-colors"
          >
            apply
          </button>
          {pendingRange === null && (
            <span data-testid="accounts-range-invalid" className="text-[var(--color-text-faint)]">
              pick a start on or before the end
            </span>
          )}
        </div>
      )}

      {/* ── Per-account breakdown: a grid, not a list ─────────────────────── */}
      <div className="mt-3">
        {error ? (
          <p data-testid="accounts-error" className="text-xs text-[var(--color-text-muted)]">
            {error}
          </p>
        ) : data === null ? (
          <p className="text-xs text-[var(--color-text-muted)]">Loading accounts…</p>
        ) : data.accounts.length === 0 ? (
          <p className="text-xs text-[var(--color-text-muted)]">
            No accounts configured — run <span className="font-mono">wickd login</span> to add one.
          </p>
        ) : (
          <div
            data-testid="accounts-grid"
            className="grid gap-2 grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-6"
          >
            {orderedAccounts(data.accounts).map((a) => (
              <AccountTile
                key={a.account_id ?? a.account}
                acct={a}
                onOpen={setOpenAccount}
              />
            ))}
          </div>
        )}
      </div>

      {/* D8 (AGT-1133): the drill-down honours the same window the tiles show.
          `selected` is the section's active GlanceWindow, set by the picker
          above; passing it here is what keeps the modal and the tiles in
          agreement. */}
      <AccountHistoryModal
        account={openAccount}
        glanceWindow={selected}
        onClose={() => setOpenAccount(null)}
      />
    </section>
  );
};
