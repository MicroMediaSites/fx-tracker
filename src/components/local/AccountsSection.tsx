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
 */
import { useState } from 'react';
import {
  AccountGlance,
  GlanceWindow,
  useAccountsGlance,
} from '../../hooks/useAccountsGlance';
import { summarizeAccounts } from './accountsSummary';

// `wickd_` prefix, not the retired `candlesight_` brand.
const WINDOW_STORAGE_KEY = 'wickd_accounts_window';

/**
 * "Today" leads and is the default: the cold-boot question is "has today been
 * profitable, per account". Deliberately not `24h` — before mid-afternoon
 * those are very different spans, and the one you want is the calendar day.
 */
const WINDOWS: { id: string; label: string; window: GlanceWindow }[] = [
  { id: 'today', label: 'today', window: { kind: 'today' } },
  { id: '7d', label: '7d', window: { kind: 'days', days: 7 } },
  { id: '30d', label: '30d', window: { kind: 'days', days: 30 } },
];

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
const AccountTile = ({ acct }: { acct: AccountGlance }) => {
  const aliases = acct.names.slice(1);

  if (acct.error) {
    return (
      <div
        data-testid="account-tile"
        className="px-3 py-2.5 rounded-lg border border-[var(--color-sell)]/30 bg-[var(--color-bg-elevated)] min-w-0"
      >
        <div className="text-xs font-mono text-[var(--color-text-secondary)] truncate">
          {acct.account}
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
  const idle = isIdle(acct);

  return (
    <div
      data-testid="account-tile"
      data-idle={idle || undefined}
      className={`px-3 py-2.5 rounded-lg border bg-[var(--color-bg-elevated)] min-w-0 ${
        idle
          ? 'border-[var(--color-border)]/60 opacity-50'
          : 'border-[var(--color-border)]'
      }`}
    >
      <div className="flex items-baseline gap-1.5 min-w-0">
        <span className="text-xs font-mono text-[var(--color-text-secondary)] truncate">
          {acct.account}
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

      {/* The tile's headline. Tabular figures so a column of tiles aligns. */}
      <div
        data-testid="account-realized"
        className={`mt-0.5 text-lg font-semibold font-mono tabular-nums truncate ${pnlColor(realized)}`}
        title="Realized P&L over the selected window"
      >
        {money(realized, acct.currency, true)}
      </div>

      <div className="mt-0.5 text-[11px] text-[var(--color-text-muted)] truncate">
        {idle ? (
          'no activity'
        ) : (
          <>
            {acct.trades ?? 0}t · {percent(acct.win_rate)}
            {openPl !== null && openPl !== 0 && (
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
    </div>
  );
};

export const AccountsSection = () => {
  const [windowId, setWindowId] = useState<string>(() => {
    const stored = localStorage.getItem(WINDOW_STORAGE_KEY);
    return WINDOWS.some((w) => w.id === stored) ? (stored as string) : 'today';
  });
  const selected = WINDOWS.find((w) => w.id === windowId) ?? WINDOWS[0];
  const { data, error, loading, refresh } = useAccountsGlance(selected.window);

  const selectWindow = (next: string) => {
    setWindowId(next);
    localStorage.setItem(WINDOW_STORAGE_KEY, next);
  };

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
            <h2 className="text-[11px] font-medium uppercase tracking-wider text-[var(--color-text-muted)]">
              {selected.id === 'today' ? 'Today' : `Last ${selected.label}`}
            </h2>
            {loading && (
              <span className="text-[11px] text-[var(--color-text-faint)]">updating…</span>
            )}
          </div>

          {summary === null ? (
            <div className="mt-1 text-4xl font-semibold font-mono text-[var(--color-text-faint)]">
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
            <div className="mt-2 text-xs text-[var(--color-text-muted)]">
              realized across {summary.accounts}{' '}
              {summary.accounts === 1 ? 'account' : 'accounts'} · {summary.trades}{' '}
              {summary.trades === 1 ? 'trade' : 'trades'} · {percent(summary.winRate)} won
              {summary.openTrades > 0 && (
                <>
                  {' · '}
                  <span className={pnlColor(summary.openPl)}>
                    {money(summary.openPl, summary.currency, true)}
                  </span>{' '}
                  open ({summary.openTrades})
                </>
              )}
              {summary.errored > 0 && (
                <span className="text-[var(--color-sell)]">
                  {' · '}
                  {summary.errored} unavailable
                </span>
              )}
            </div>
          )}
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <div className="flex items-center gap-0.5" role="group" aria-label="Performance window">
            {WINDOWS.map((w) => (
              <button
                key={w.id}
                data-testid={`accounts-window-${w.id}`}
                onClick={() => selectWindow(w.id)}
                aria-pressed={w.id === windowId}
                className={`px-2 py-0.5 text-xs rounded font-mono transition-colors ${
                  w.id === windowId
                    ? 'bg-[var(--color-info)]/15 text-[var(--color-info)]'
                    : 'text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]'
                }`}
              >
                {w.label}
              </button>
            ))}
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
              <AccountTile key={a.account_id ?? a.account} acct={a} />
            ))}
          </div>
        )}
      </div>
    </section>
  );
};
