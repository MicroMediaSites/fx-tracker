/**
 * Accounts at a glance — one row per OANDA account the wickd CLI is logged
 * into, with a rolling-window performance summary. Rendered on the HOME window.
 *
 * Reads `accounts_glance`, which shells out to `wickd trade glance`. The CLI
 * owns credentials + OANDA; this only renders.
 *
 * Two honesty constraints drive the design:
 *
 *  1. The window number is REALIZED P&L only — there is no stored NAV time
 *     series anywhere in wickd, so a position opened before the window and
 *     still open contributes nothing to it (its unrealized P&L shows in the
 *     separate `open` figure, which is as-of-now, not as-of-window). The
 *     column header says "realized" and the footnote says so plainly rather
 *     than implying a true period return.
 *  2. A null win rate renders "—", never "0%": no decided trades in the window
 *     is not the same as losing every trade.
 */
import { useState } from 'react';
import { CollapsibleSection } from '../ui/CollapsibleSection';
import { AccountGlance, useAccountsGlance } from '../../hooks/useAccountsGlance';

// `wickd_` prefix, not the retired `candlesight_` brand — the surrounding
// CollapsibleSection keys are pre-rename and stay as they are (renaming them
// would silently drop everyone's saved collapse state); new keys use the
// current name.
const WINDOW_STORAGE_KEY = 'wickd_accounts_window_days';

const WINDOWS: { days: number; label: string }[] = [
  { days: 1, label: '24h' },
  { days: 7, label: '7d' },
  { days: 30, label: '30d' },
];

/**
 * Exact decimal strings cross the boundary from the CLI; parse to a number for
 * DISPLAY only (never to compute). A non-numeric/absent value renders as "—".
 */
const money = (value: string | null, currency: string | null, signed = false): string => {
  if (value === null) return '—';
  const n = Number(value);
  if (!Number.isFinite(n)) return '—';
  const formatted = new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: currency || 'USD',
    currencyDisplay: 'narrowSymbol',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(Math.abs(n));
  if (!signed) return formatted;
  // Explicit sign on the P&L figures: at a glance the sign matters more than
  // the magnitude, and a bare "-" is easy to miss against a currency symbol.
  return `${n > 0 ? '+' : n < 0 ? '−' : ''}${formatted}`;
};

/** Zero is neutral, not green — a flat window shouldn't read as a win. */
const pnlColor = (value: string | null): string => {
  const n = Number(value);
  if (value === null || !Number.isFinite(n) || n === 0) return 'text-[var(--color-text-muted)]';
  return n > 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]';
};

const percent = (rate: number | null): string =>
  rate === null ? '—' : `${Math.round(rate * 100)}%`;

const AccountRow = ({ acct }: { acct: AccountGlance }) => {
  const aliases = acct.names.slice(1);

  if (acct.error) {
    return (
      <div
        data-testid="account-row"
        className="flex items-center justify-between gap-3 px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
      >
        <span className="text-sm font-mono text-[var(--color-text-secondary)]">{acct.account}</span>
        <span
          className="text-xs text-[var(--color-sell)] truncate max-w-[60%]"
          title={acct.error}
          data-testid="account-error"
        >
          {acct.error}
        </span>
      </div>
    );
  }

  const openPl = Number(acct.unrealized_pl);
  const hasOpen = (acct.open_trade_count ?? 0) > 0 || (Number.isFinite(openPl) && openPl !== 0);

  return (
    <div
      data-testid="account-row"
      className="px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
    >
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-sm font-mono text-[var(--color-text-primary)] truncate">
          {acct.account}
          {aliases.length > 0 && (
            <span
              className="ml-1.5 text-xs text-[var(--color-text-faint)]"
              title={`Also configured as ${aliases.join(', ')} — same OANDA account`}
            >
              +{aliases.join(', ')}
            </span>
          )}
        </span>
        <span className="flex items-baseline gap-3 whitespace-nowrap">
          <span className="text-sm font-mono text-[var(--color-text-secondary)]" title="Net asset value">
            {money(acct.nav, acct.currency)}
          </span>
          <span
            data-testid="account-realized"
            className={`text-sm font-mono font-semibold ${pnlColor(acct.realized)}`}
            title="Realized P&L over the selected window"
          >
            {money(acct.realized, acct.currency, true)}
          </span>
        </span>
      </div>

      <div className="mt-1 flex flex-wrap items-center gap-x-2 text-xs text-[var(--color-text-muted)]">
        {hasOpen && (
          <>
            <span title="Unrealized P&L on currently open trades (as of now, not window-scoped)">
              open{' '}
              <span className={`font-mono ${pnlColor(acct.unrealized_pl)}`}>
                {money(acct.unrealized_pl, acct.currency, true)}
              </span>
              {(acct.open_trade_count ?? 0) > 0 && ` (${acct.open_trade_count})`}
            </span>
            <span aria-hidden="true">·</span>
          </>
        )}
        <span>
          {acct.trades ?? 0} {acct.trades === 1 ? 'trade' : 'trades'}
        </span>
        <span aria-hidden="true">·</span>
        <span title={`${acct.wins ?? 0} won, ${acct.losses ?? 0} lost`}>{percent(acct.win_rate)} W</span>
      </div>
    </div>
  );
};

export const AccountsSection = () => {
  const [days, setDays] = useState<number>(() => {
    const stored = Number(localStorage.getItem(WINDOW_STORAGE_KEY));
    return WINDOWS.some((w) => w.days === stored) ? stored : 7;
  });
  const { data, error, loading, refresh } = useAccountsGlance(days);

  const selectWindow = (next: number) => {
    setDays(next);
    localStorage.setItem(WINDOW_STORAGE_KEY, String(next));
  };

  const asOf = data
    ? new Date(data.generated_at).toLocaleTimeString(undefined, {
        hour: '2-digit',
        minute: '2-digit',
      })
    : null;

  return (
    <CollapsibleSection
      id="accounts_glance"
      title="Accounts"
      badge={
        data ? (
          <span data-testid="accounts-count" className="text-xs text-[var(--color-text-muted)]">
            {data.accounts.length}
          </span>
        ) : null
      }
      action={
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-0.5" role="group" aria-label="Performance window">
            {WINDOWS.map((w) => (
              <button
                key={w.days}
                data-testid={`accounts-window-${w.label}`}
                onClick={() => selectWindow(w.days)}
                aria-pressed={w.days === days}
                className={`px-1.5 py-0.5 text-xs rounded font-mono transition-colors ${
                  w.days === days
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
            {loading ? '…' : '↻'}
          </button>
        </div>
      }
    >
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
        <>
          <div className="space-y-1.5">
            {data.accounts.map((a) => (
              <AccountRow key={a.account_id ?? a.account} acct={a} />
            ))}
          </div>
          <p className="mt-2 text-xs text-[var(--color-text-faint)]">
            Realized P&amp;L from trades closed in the last{' '}
            {WINDOWS.find((w) => w.days === days)?.label}; open positions are counted separately and
            as of now. {asOf && `As of ${asOf}.`}
          </p>
        </>
      )}
    </CollapsibleSection>
  );
};
