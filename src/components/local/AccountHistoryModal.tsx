/**
 * AccountHistoryModal — the drill-down behind an account tile.
 *
 * Shows every closed trade since the account's experiment start (its baseline),
 * each as an entry → exit row with side, size, prices, P&L, duration, and
 * strategy. Opened by clicking a tile; reaches OANDA through the CLI, so it
 * loads on open rather than on a timer.
 *
 * Two honesty affordances, both driven by real backend flags rather than
 * cosmetics:
 *
 *  - A trade with more than one exit is marked "avg" — its exit price is
 *    OANDA's blended `averageClosePrice`, not a single fill, and must not be
 *    shown as one. (`wickd` never partially closes today, so this is rare, but
 *    the data can't lie once it happens.)
 *  - When OANDA's fetch cap is hit, a banner says the history may be
 *    incomplete rather than letting a partial list read as the whole record.
 */
import { useEffect } from 'react';
import { HistoryTrade, useAccountHistory } from '../../hooks/useAccountHistory';

const money = (value: string | null, signed = false): string => {
  if (value === null) return '—';
  const n = Number(value);
  if (!Number.isFinite(n)) return '—';
  const abs = Math.abs(n).toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
  if (!signed) return abs;
  return `${n > 0 ? '+' : n < 0 ? '−' : ''}$${abs}`;
};

const price = (value: string | null): string => value ?? '—';

const pnlColor = (value: string): string => {
  const n = Number(value);
  if (!Number.isFinite(n) || n === 0) return 'text-[var(--color-text-muted)]';
  return n > 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]';
};

const timeLabel = (iso: string | null): string => {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
};

/** "3m" / "2h 14m" / "1d 3h" — trade hold time from its duration in seconds. */
export const formatDuration = (secs: number | null): string => {
  if (secs === null || secs < 0) return '—';
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const remM = m % 60;
  if (h < 24) return remM > 0 ? `${h}h ${remM}m` : `${h}h`;
  const d = Math.floor(h / 24);
  const remH = h % 24;
  return remH > 0 ? `${d}d ${remH}h` : `${d}d`;
};

const TradeRow = ({ trade }: { trade: HistoryTrade }) => {
  const long = trade.side === 'long';
  return (
    <div
      data-testid="history-trade-row"
      className="grid grid-cols-[auto_1fr_auto] items-center gap-x-3 gap-y-0.5 px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
    >
      {/* Left: instrument + direction pill */}
      <div className="flex items-center gap-2 min-w-0">
        <span
          className={`px-1.5 py-0.5 text-[10px] font-semibold rounded uppercase ${
            long
              ? 'bg-[var(--color-buy)]/15 text-[var(--color-buy)]'
              : 'bg-[var(--color-sell)]/15 text-[var(--color-sell)]'
          }`}
        >
          {trade.side}
        </span>
        <span className="text-sm font-mono text-[var(--color-text-primary)]">
          {trade.instrument}
        </span>
        <span className="text-xs text-[var(--color-text-faint)] font-mono">{trade.units}u</span>
      </div>

      {/* Middle: entry → exit prices, on their own line under the header on narrow */}
      <div className="flex items-center gap-2 min-w-0 text-xs font-mono text-[var(--color-text-secondary)]">
        <span title={`Entry ${timeLabel(trade.entry.time)}`}>{price(trade.entry.price)}</span>
        <span className="text-[var(--color-text-faint)]">→</span>
        <span title={`Exit ${timeLabel(trade.exit.time)}`}>{price(trade.exit.price)}</span>
        {trade.exit.blended && (
          <span
            data-testid="history-blended"
            className="px-1 py-0.5 text-[10px] rounded bg-[var(--color-warning)]/15 text-[var(--color-warning)]"
            title={`${trade.exit.count} separate exits — this price is their average, not a single fill`}
          >
            {trade.exit.count} exits · avg
          </span>
        )}
      </div>

      {/* Right: realized P&L */}
      <div
        data-testid="history-trade-pnl"
        className={`text-sm font-mono font-semibold text-right tabular-nums ${pnlColor(trade.realized_pl)}`}
      >
        {money(trade.realized_pl, true)}
      </div>

      {/* Second row: meta spanning full width */}
      <div className="col-span-3 flex items-center gap-x-2 text-[11px] text-[var(--color-text-muted)]">
        <span>{timeLabel(trade.entry.time)}</span>
        <span aria-hidden="true">·</span>
        <span>held {formatDuration(trade.duration_secs)}</span>
        {trade.strategy && (
          <>
            <span aria-hidden="true">·</span>
            <span className="font-mono">{trade.strategy}</span>
          </>
        )}
      </div>
    </div>
  );
};

interface Props {
  /** The account name to show; `null` closes the modal. */
  account: string | null;
  onClose: () => void;
}

export const AccountHistoryModal = ({ account, onClose }: Props) => {
  const { data, error, loading } = useAccountHistory(account);

  useEffect(() => {
    if (account === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [account, onClose]);

  if (account === null) return null;

  return (
    <div
      className="fixed inset-0 z-[150] flex items-center justify-center"
      onClick={onClose}
      data-testid="account-history-modal"
    >
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-[var(--color-bg-card)] rounded-lg shadow-xl max-w-2xl w-full mx-4 max-h-[85vh] flex flex-col border border-[var(--color-border)]"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={`Trade history for ${account}`}
      >
        <div className="flex items-baseline justify-between px-5 py-3 border-b border-[var(--color-border)] gap-3">
          <div className="min-w-0">
            <h3 className="text-base font-semibold font-mono">{account}</h3>
            {data && (
              <p className="text-xs text-[var(--color-text-muted)] mt-0.5">
                {data.count} {data.count === 1 ? 'trade' : 'trades'}
                {data.since ? ' since experiment start' : ' (all available)'} ·{' '}
                <span className={pnlColor(data.realized)}>{money(data.realized, true)}</span> realized
              </p>
            )}
          </div>
          <button
            onClick={onClose}
            className="shrink-0 text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="overflow-y-auto px-5 py-4 flex-1">
          {loading && data === null ? (
            <p className="text-sm text-[var(--color-text-muted)]">Loading trade history…</p>
          ) : error ? (
            <p data-testid="history-error" className="text-sm text-[var(--color-sell)]">
              {error}
            </p>
          ) : data === null || data.count === 0 ? (
            <p className="text-sm text-[var(--color-text-muted)]">
              No closed trades{data?.since ? ' since this account started' : ' yet'}.
            </p>
          ) : (
            <>
              {data.truncated && (
                <p
                  data-testid="history-truncated"
                  className="mb-3 px-3 py-2 rounded text-xs text-[var(--color-warning)] bg-[var(--color-warning)]/10 border border-[var(--color-warning)]/30"
                >
                  Showing the most recent {data.count} trades — OANDA's history cap was reached, so
                  earlier trades in this window aren't included.
                </p>
              )}
              {data.blended_exits > 0 && (
                <p className="mb-3 text-xs text-[var(--color-text-muted)]">
                  {data.blended_exits} {data.blended_exits === 1 ? 'trade' : 'trades'} closed in
                  multiple exits; those exit prices are averages (marked "avg").
                </p>
              )}
              <div className="space-y-1.5">
                {data.trades.map((t) => (
                  <TradeRow key={t.id} trade={t} />
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
