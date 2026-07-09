import { useEffect, useRef, useState } from "react";
import type { ViewProps } from "@openthink/ui-leaf/view";

/**
 * FX execution ticket — the TradingTicketApp execution surface, rebuilt on
 * wickd's ui-leaf substrate.
 *
 * Live quotes arrive as `data` updates pushed by `wickd view ticket` (hub or
 * direct OANDA feed); orders go back through the `place_order` mutation into
 * the same guarded path as `wickd trade place`. Paper by default; the Live
 * toggle routes through the AGT-613 keystroke confirmation in the launching
 * terminal — this view can propose a live order but can never arm one.
 */

interface TicketData {
  instrument: string;
  env: string;
  bid?: string;
  ask?: string;
  spread?: string;
  time?: string;
  tradeable?: boolean;
  /** Quote source: "connecting" | "hub" | "direct" | "none". */
  feed: string;
  feedError?: string;
  /** Persisted historical spread extremes (~/.wickd/spreads.db, decayed). */
  minSpread?: string;
  maxSpread?: string;
  /** Latest strategy-signal proposal for this instrument (load to prefill). */
  proposal?: Proposal;
  [key: string]: unknown;
}

/** A strategy proposal pushed by wickd — auto-fill, never auto-fire. */
interface Proposal {
  id: string;
  strategy: string;
  side: string; // "long" | "short"
  units: number;
  suggestedUnits?: number;
  sl?: string;
  tp?: string;
  entryPrice?: string;
  reason: string;
  ts: string;
  source: string; // "launch" auto-loads once; "live" waits for a click
}

/** Shape of the guarded place path's JSON result (paper or live). */
interface OrderResult {
  ok?: boolean;
  mode?: string;
  submitted?: boolean;
  outcome?: string;
  units?: number | string;
  price?: string | null;
  trade_id?: string | null;
  order_id?: string | null;
  reason?: string;
  signal_id?: string;
  consumed?: boolean;
  [key: string]: unknown;
}

type Side = "buy" | "sell";
type OrderType = "market" | "limit" | "stop";
type RiskMode = "pips" | "price" | "%";
type ClickMode = "1-click" | "2-click";

// ---- FX math (ported from CandleSight's priceCalculations) ----

const pipMultiplier = (isJpy: boolean) => (isJpy ? 100 : 10000);
const decimalsFor = (isJpy: boolean) => (isJpy ? 3 : 5);

function riskPrice(
  value: string,
  mode: RiskMode,
  currentPrice: number,
  side: Side,
  isJpy: boolean,
  kind: "sl" | "tp",
): string | null {
  if (!currentPrice || !value) return null;
  const val = parseFloat(value);
  if (isNaN(val)) return null;
  const decimals = decimalsFor(isJpy);
  if (mode === "price") return val.toFixed(decimals);
  const distance =
    mode === "%" ? currentPrice * (val / 100) : val / pipMultiplier(isJpy);
  // SL sits against the trade, TP with it.
  const below = kind === "sl" ? side === "buy" : side === "sell";
  return (below ? currentPrice - distance : currentPrice + distance).toFixed(decimals);
}

interface PriceParts {
  top: string;
  big: string;
  small: string;
}

function priceParts(price: number, isJpy: boolean): PriceParts {
  if (!price || isNaN(price)) return { top: "—", big: "——", small: "—" };
  const s = price.toFixed(isJpy ? 3 : 5);
  const [whole, dec] = s.split(".");
  return isJpy
    ? { top: `${whole}.`, big: dec.slice(0, 2), small: dec.slice(2) }
    : { top: `${whole}.${dec.slice(0, 2)}`, big: dec.slice(2, 4), small: dec.slice(4) };
}

// ---- palette ----

const MUTED = "#888";
const BUY = "#0a7d32";
const SELL = "#c0271a";
const BORDER = "#ddd";
const PAGE_BG = "#ffffff";
const MONO = "ui-monospace, monospace";

// ---- price flash (ported from usePriceFlash) ----

type Flash = "up" | "down" | null;

function useFlash(value?: string): Flash {
  const prev = useRef<string | undefined>(undefined);
  const [dir, setDir] = useState<Flash>(null);
  useEffect(() => {
    const before = prev.current;
    prev.current = value;
    if (before === undefined || value === undefined || value === before) return;
    setDir(parseFloat(value) > parseFloat(before) ? "up" : "down");
    const t = setTimeout(() => setDir(null), 400);
    return () => clearTimeout(t);
  }, [value]);
  return dir;
}

const flashColor = (f: Flash) => (f === "up" ? BUY : f === "down" ? SELL : "#1a1a1a");

// ---- spread coloring (ported from PriceWindow's calculateSpreadColor) ----
//
// Grades the live spread against the persisted historical extremes wickd
// maintains in ~/.wickd/spreads.db (EMA-decayed min/max, contributed by
// `wickd stream` and by this ticket's own quotes). Green = historically low,
// yellow = average, red = high. Purple = no history for this instrument yet,
// positioned within a generic per-pair-type default range — same semantics
// as the original.

function spreadColor(
  spread: number,
  minSpread: string | undefined,
  maxSpread: string | undefined,
  isJpy: boolean,
): string {
  const min = minSpread ? parseFloat(minSpread) : NaN;
  const max = maxSpread ? parseFloat(maxSpread) : NaN;
  if (isNaN(min) || isNaN(max) || max <= min) {
    const range = isJpy ? { min: 0.006, max: 0.05 } : { min: 0.00006, max: 0.0005 };
    const pct = Math.max(0, Math.min(1, (spread - range.min) / (range.max - range.min)));
    return `hsl(${280 + pct * 40}, 60%, 50%)`; // purple = no history yet
  }
  const pct = Math.max(0, Math.min(1, (spread - min) / (max - min)));
  return `hsl(${120 - pct * 120}, 70%, 45%)`; // green (low) → red (high)
}

// ---- the price window: sell/buy cells, center-out spread bars, notch ----

function PriceCell({
  label,
  parts,
  flash,
  align,
}: {
  label: string;
  parts: PriceParts;
  flash: Flash;
  align: "left" | "right";
}) {
  return (
    <div
      style={{
        flex: 1,
        padding: "0.2rem 0.6rem 0.9rem",
        border: `1px solid ${BORDER}`,
        borderBottom: "none",
        borderRadius: align === "left" ? "6px 0 0 0" : "0 6px 0 0",
        borderLeft: align === "right" ? "none" : undefined,
      }}
    >
      <div style={{ fontSize: "0.6rem", color: MUTED, textAlign: align, marginBottom: "0.15rem" }}>
        {label}
      </div>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center" }}>
        <span style={{ fontSize: "0.68rem", color: MUTED, fontFamily: MONO, transform: "translateX(-1.1rem)" }}>
          {parts.top}
        </span>
        <div style={{ display: "flex", alignItems: "baseline" }}>
          <span
            style={{
              fontSize: "1.7rem",
              fontFamily: MONO,
              fontWeight: 600,
              color: flashColor(flash),
              transition: "color 0.3s",
            }}
          >
            {parts.big}
          </span>
          <span style={{ fontSize: "0.95rem", color: "#555", fontFamily: MONO, alignSelf: "flex-start" }}>
            {parts.small}
          </span>
        </div>
      </div>
    </div>
  );
}

function PriceWindow({
  instrument,
  bid,
  ask,
  isJpy,
  bidFlash,
  askFlash,
  minSpread,
  maxSpread,
}: {
  instrument: string;
  bid: number;
  ask: number;
  isJpy: boolean;
  bidFlash: Flash;
  askFlash: Flash;
  minSpread?: string;
  maxSpread?: string;
}) {
  const [baseCurrency] = instrument.split("_");
  const spread = ask - bid;
  const spreadPips = (spread * pipMultiplier(isJpy)).toFixed(1);

  // Bar width: logarithmic — handles tight and wide spreads gracefully.
  // 0 pips = 0%, ~1 pip = 40%, ~5 pips = 70%, ~20 pips = 90%, 50+ = 100%.
  const spreadNum = parseFloat(spreadPips);
  const barWidth =
    spreadNum <= 0 ? 0 : Math.min(100, (Math.log10(spreadNum + 1) / Math.log10(51)) * 100);
  const barColor = spreadColor(spread, minSpread, maxSpread, isJpy);
  const bar = (justify: "flex-start" | "flex-end", corner: "left" | "right") => (
    <div
      style={{
        height: "6px",
        border: `1px solid ${BORDER}`,
        borderTop: "none",
        borderLeft: corner === "right" ? "none" : undefined,
        borderRadius: corner === "left" ? "0 0 0 6px" : "0 0 6px 0",
        marginTop: "-1px",
        display: "flex",
        justifyContent: justify,
      }}
    >
      <div style={{ height: "100%", width: `${barWidth}%`, background: barColor, transition: "all 0.3s" }} />
    </div>
  );

  return (
    <div style={{ display: "flex", alignItems: "stretch", position: "relative" }}>
      {/* Sell side: spread bar grows from the center seam, leftward */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column" }}>
        <PriceCell label={`Sell ${baseCurrency}`} parts={priceParts(bid, isJpy)} flash={bidFlash} align="left" />
        {bar("flex-end", "left")}
      </div>

      {/* Notch triangle — two stacked CSS triangles for a bordered notch,
          with the spread (pips) inside. */}
      <div style={{ position: "absolute", bottom: 0, left: "50%", transform: "translateX(-50%)", zIndex: 1 }}>
        <div
          style={{
            width: 0,
            height: 0,
            borderLeft: "25px solid transparent",
            borderRight: "25px solid transparent",
            borderBottom: `21px solid ${BORDER}`,
          }}
        />
        <div
          style={{
            position: "absolute",
            bottom: 0,
            left: "50%",
            transform: "translateX(-50%)",
            width: 0,
            height: 0,
            borderLeft: "24px solid transparent",
            borderRight: "24px solid transparent",
            borderBottom: `20px solid ${PAGE_BG}`,
          }}
        />
        <span
          style={{
            position: "absolute",
            left: "50%",
            transform: "translateX(-50%)",
            bottom: "-2px",
            fontSize: "9px",
            color: MUTED,
            fontFamily: MONO,
          }}
        >
          {spreadPips}
        </span>
      </div>

      {/* Buy side: spread bar grows from the center seam, rightward */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column" }}>
        <PriceCell label={`Buy ${baseCurrency}`} parts={priceParts(ask, isJpy)} flash={askFlash} align="right" />
        {bar("flex-start", "right")}
      </div>
    </div>
  );
}

// ---- small styled pieces ----

/** Cycle through options on click — the CycleSelector pattern. */
function Cycle<T extends string>({
  options,
  value,
  onChange,
  accent,
}: {
  options: readonly T[];
  value: T;
  onChange: (v: T) => void;
  accent?: string;
}) {
  return (
    <button
      type="button"
      onClick={() => onChange(options[(options.indexOf(value) + 1) % options.length])}
      style={{
        background: "none",
        border: `1px solid ${BORDER}`,
        borderRadius: "4px",
        padding: "0.15rem 0.5rem",
        fontSize: "0.7rem",
        letterSpacing: "0.04em",
        color: accent ?? "#555",
        cursor: "pointer",
        textTransform: "capitalize",
      }}
    >
      {value}
    </button>
  );
}

function RiskInput({
  label,
  modes,
  mode,
  onMode,
  value,
  onValue,
  calculated,
  accent,
}: {
  label: string;
  modes: readonly RiskMode[];
  mode: RiskMode;
  onMode: (m: RiskMode) => void;
  value: string;
  onValue: (v: string) => void;
  /** Computed price per side — pips/% distances land on opposite sides for buy vs sell. */
  calculated: { buy: string | null; sell: string | null };
  accent: string;
}) {
  const preview =
    calculated.buy && calculated.sell
      ? calculated.buy === calculated.sell
        ? calculated.buy
        : `B ${calculated.buy} · S ${calculated.sell}`
      : (calculated.buy ?? calculated.sell);
  return (
    <div style={{ flex: 1 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "0.2rem" }}>
        <span style={{ fontSize: "0.6rem", color: MUTED, letterSpacing: "0.06em", textTransform: "uppercase" }}>
          {label}
        </span>
        <Cycle options={modes} value={mode} onChange={onMode} />
      </div>
      <input
        type="text"
        value={value}
        onChange={(e) => onValue(e.target.value)}
        placeholder="—"
        style={{
          width: "100%",
          boxSizing: "border-box",
          border: `1px solid ${BORDER}`,
          borderRadius: "4px",
          padding: "0.3rem 0.45rem",
          fontFamily: MONO,
          fontSize: "0.8rem",
        }}
      />
      <div style={{ fontSize: "0.62rem", color: preview ? accent : "#bbb", marginTop: "0.15rem", fontFamily: MONO, whiteSpace: "nowrap", overflow: "hidden" }}>
        {preview ?? " "}
      </div>
    </div>
  );
}

// ---- the ticket ----

export default function Ticket({ data, mutate }: ViewProps<TicketData>) {
  const instrument = data.instrument ?? "EUR_USD";
  const isJpy = instrument.includes("JPY");
  const decimals = decimalsFor(isJpy);

  const bid = data.bid ? parseFloat(data.bid) : NaN;
  const ask = data.ask ? parseFloat(data.ask) : NaN;
  const hasQuote = !isNaN(bid) && !isNaN(ask);
  const mid = hasQuote ? (bid + ask) / 2 : NaN;
  const marketClosed = data.tradeable === false;

  const bidFlash = useFlash(data.bid);
  const askFlash = useFlash(data.ask);

  const [orderType, setOrderType] = useState<OrderType>("market");
  const [clickMode, setClickMode] = useState<ClickMode>("2-click");
  const [units, setUnits] = useState("1000");
  const [entryPrice, setEntryPrice] = useState("");
  const [slMode, setSlMode] = useState<RiskMode>("pips");
  const [slValue, setSlValue] = useState("");
  const [tpMode, setTpMode] = useState<RiskMode>("pips");
  const [tpValue, setTpValue] = useState("");
  const [live, setLive] = useState(false);

  const [pendingClick, setPendingClick] = useState<{ side: Side; at: number } | null>(null);
  const [inFlight, setInFlight] = useState<null | { side: Side; live: boolean }>(null);
  const [error, setError] = useState<string | null>(null);
  const [overlay, setOverlay] = useState<null | { side: Side; result: OrderResult; fading: boolean }>(null);
  const overlayTimers = useRef<ReturnType<typeof setTimeout>[]>([]);

  // The strategy signal the form currently executes, when a proposal was
  // loaded. The signal id rides along on the order ONLY when the clicked
  // side matches the proposal's side — an opposite-side order is a manual
  // trade, not an execution of the signal.
  const [linkedSignal, setLinkedSignal] = useState<null | { id: string; side: Side; strategy: string }>(null);

  const loadProposal = (p: Proposal) => {
    const side: Side = p.side === "short" ? "sell" : "buy";
    const size = Math.abs(p.suggestedUnits ?? p.units);
    setUnits(String(size));
    setOrderType("market");
    if (p.sl) {
      setSlMode("price");
      setSlValue(p.sl);
    }
    if (p.tp) {
      setTpMode("price");
      setTpValue(p.tp);
    }
    setLinkedSignal({ id: p.id, side, strategy: p.strategy });
    // Clear the chip host-side; failures are harmless (chip lingers).
    mutate("dismiss_proposal").catch(() => {});
  };

  // `--pending <id>` seeds a launch proposal: load it into the form once.
  const autoloaded = useRef(false);
  useEffect(() => {
    if (autoloaded.current) return;
    if (data.proposal?.source === "launch") {
      autoloaded.current = true;
      loadProposal(data.proposal);
    }
  }, [data.proposal]);

  // Dismiss the confirmation overlay on any key press.
  useEffect(() => {
    if (!overlay) return;
    const onKey = () => setOverlay(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [overlay]);
  useEffect(() => () => overlayTimers.current.forEach(clearTimeout), []);

  // SL/TP computed against the side being priced: buys lift the ask, sells hit the bid.
  const refPrice = (side: Side) => (side === "buy" ? ask : bid);
  const calcSl = (side: Side) => riskPrice(slValue, slMode, refPrice(side), side, isJpy, "sl");
  const calcTp = (side: Side) => riskPrice(tpValue, tpMode, refPrice(side), side, isJpy, "tp");

  const submit = async (side: Side) => {
    if (inFlight) return;
    if (clickMode === "2-click") {
      const now = Date.now();
      if (!(pendingClick && pendingClick.side === side && now - pendingClick.at < 1500)) {
        setPendingClick({ side, at: now });
        setTimeout(() => setPendingClick((p) => (p && p.side === side ? null : p)), 1500);
        return;
      }
      setPendingClick(null);
    }
    setError(null);

    const unitsNum = parseInt(units, 10);
    if (isNaN(unitsNum) || unitsNum <= 0) {
      setError("Units must be a positive number");
      return;
    }

    // The signal link only applies when this order IS the proposal's trade.
    const signalId = linkedSignal && linkedSignal.side === side ? linkedSignal.id : undefined;

    setInFlight({ side, live });
    try {
      const result = await mutate<OrderResult>("place_order", {
        units: side === "sell" ? -unitsNum : unitsNum,
        type: orderType,
        price: orderType === "market" ? undefined : entryPrice || undefined,
        sl: calcSl(side) ?? undefined,
        tp: calcTp(side) ?? undefined,
        live,
        signal_id: signalId,
      });
      if (result && result.ok === false) {
        setError(result.reason ?? "order rejected");
      } else {
        if (signalId) setLinkedSignal(null); // signal consumed by this order
        setOverlay({ side, result: result ?? {}, fading: false });
        overlayTimers.current.push(
          setTimeout(() => setOverlay((o) => (o ? { ...o, fading: true } : null)), 2500),
          setTimeout(() => setOverlay(null), 3000),
        );
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setInFlight(null);
    }
  };

  const feedBadge = (() => {
    switch (data.feed) {
      case "hub":
        return { text: "hub", color: BUY };
      case "direct":
        return { text: "direct", color: BUY };
      case "none":
        return { text: "no feed", color: SELL };
      default:
        return { text: "connecting…", color: MUTED };
    }
  })();

  const sideButton = (side: Side) => {
    const color = side === "buy" ? BUY : SELL;
    const armed = pendingClick?.side === side;
    const disabled = marketClosed || inFlight !== null;
    return (
      <button
        type="button"
        onClick={() => submit(side)}
        disabled={disabled}
        style={{
          flex: 1,
          minHeight: "52px",
          fontSize: "0.82rem",
          fontWeight: 600,
          letterSpacing: "0.04em",
          border: `1px solid ${armed ? color : BORDER}`,
          borderRadius: "6px",
          background: armed ? `${color}22` : "white",
          color: disabled ? "#bbb" : color,
          cursor: disabled ? "not-allowed" : "pointer",
        }}
      >
        {armed ? "CONFIRM" : side === "buy" ? "BUY" : "SELL"}
        {live && (
          <span style={{ display: "block", fontSize: "0.58rem", fontWeight: 400, color: SELL }}>
            LIVE
          </span>
        )}
      </button>
    );
  };

  return (
    <div
      style={{
        fontFamily: "system-ui, -apple-system, sans-serif",
        color: "#1a1a1a",
        padding: "0.6rem 0.7rem 1rem",
      }}
    >
      {/* Header: instrument, env, feed */}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginBottom: "0.5rem" }}>
        <h1 style={{ fontSize: "1rem", margin: 0 }}>{instrument.replace("_", "/")}</h1>
        <div style={{ display: "flex", gap: "0.5rem", alignItems: "baseline" }}>
          <span style={{ fontSize: "0.62rem", color: data.env === "live" ? SELL : MUTED, letterSpacing: "0.06em" }}>
            {data.env}
          </span>
          <span style={{ fontSize: "0.62rem", color: feedBadge.color, letterSpacing: "0.06em" }}>
            {feedBadge.text}
          </span>
        </div>
      </div>
      {data.feedError && (
        <p style={{ fontSize: "0.68rem", color: "#a36a00", background: "#fff7e6", border: "1px solid #f0dcb0", borderRadius: "4px", padding: "0.35rem 0.5rem", margin: "0 0 0.5rem" }}>
          {data.feedError}
        </p>
      )}

      {/* Strategy proposal chip — Load prefills the form, never places */}
      {data.proposal && data.proposal.source === "live" && (() => {
        const p = data.proposal;
        const side: Side = p.side === "short" ? "sell" : "buy";
        const color = side === "buy" ? BUY : SELL;
        return (
          <div style={{ display: "flex", alignItems: "center", gap: "0.45rem", border: `1px solid ${color}55`, background: `${color}0d`, borderRadius: "6px", padding: "0.35rem 0.5rem", margin: "0 0 0.5rem" }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: "0.68rem", fontWeight: 600, color }}>
                {p.strategy} · {side.toUpperCase()}
                {p.suggestedUnits != null && (
                  <span style={{ fontWeight: 400, color: "#555" }}>
                    {" "}· sized {Math.abs(p.suggestedUnits).toLocaleString()}
                  </span>
                )}
              </div>
              <div style={{ fontSize: "0.6rem", color: MUTED, fontFamily: MONO, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                {p.sl ? `SL ${p.sl}` : ""}{p.tp ? ` · TP ${p.tp}` : ""}{p.entryPrice ? ` · @ ${p.entryPrice}` : ""}
              </div>
            </div>
            <button
              type="button"
              onClick={() => loadProposal(p)}
              style={{ border: `1px solid ${color}`, color, background: "white", borderRadius: "4px", fontSize: "0.68rem", fontWeight: 600, padding: "0.25rem 0.55rem", cursor: "pointer" }}
            >
              Load
            </button>
            <button
              type="button"
              onClick={() => mutate("dismiss_proposal").catch(() => {})}
              style={{ border: "none", background: "none", color: MUTED, fontSize: "0.8rem", cursor: "pointer", padding: "0.1rem" }}
            >
              ✕
            </button>
          </div>
        );
      })()}

      {linkedSignal && (
        <p style={{ fontSize: "0.62rem", color: MUTED, margin: "0 0 0.4rem", display: "flex", justifyContent: "space-between" }}>
          <span>
            executing <span style={{ fontWeight: 600 }}>{linkedSignal.strategy}</span> signal — place with{" "}
            <span style={{ color: linkedSignal.side === "buy" ? BUY : SELL, fontWeight: 600 }}>
              {linkedSignal.side.toUpperCase()}
            </span>
          </span>
          <span style={{ cursor: "pointer" }} onClick={() => setLinkedSignal(null)}>unlink ✕</span>
        </p>
      )}

      {/* Price window: sell/buy cells, center-out spread bars, notch */}
      {hasQuote ? (
        <PriceWindow
          instrument={instrument}
          bid={bid}
          ask={ask}
          isJpy={isJpy}
          bidFlash={bidFlash}
          askFlash={askFlash}
          minSpread={data.minSpread}
          maxSpread={data.maxSpread}
        />
      ) : (
        <div style={{ height: "72px", display: "flex", alignItems: "center", justifyContent: "center", color: "#bbb", fontSize: "0.8rem", border: `1px solid ${BORDER}`, borderRadius: "6px" }}>
          waiting for quotes…
        </div>
      )}

      {marketClosed && (
        <p style={{ textAlign: "center", fontSize: "0.7rem", color: "#a36a00", background: "#fff7e6", border: "1px solid #f0dcb0", borderRadius: "4px", padding: "0.3rem", margin: "0.4rem 0 0" }}>
          Market closed
        </p>
      )}

      {/* Order type + click mode */}
      <div style={{ display: "flex", justifyContent: "space-between", margin: "0.5rem 0 0.4rem" }}>
        <Cycle options={["market", "limit", "stop"] as const} value={orderType} onChange={setOrderType} />
        <Cycle options={["2-click", "1-click"] as const} value={clickMode} onChange={setClickMode} accent={clickMode === "1-click" ? SELL : undefined} />
      </div>

      {/* Units + entry price */}
      <div style={{ display: "flex", gap: "0.6rem", marginBottom: "0.5rem" }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: "0.6rem", color: MUTED, letterSpacing: "0.06em", marginBottom: "0.2rem" }}>UNITS</div>
          <input
            type="text"
            value={units}
            onChange={(e) => setUnits(e.target.value)}
            style={{ width: "100%", boxSizing: "border-box", border: `1px solid ${BORDER}`, borderRadius: "4px", padding: "0.3rem 0.45rem", fontFamily: MONO, fontSize: "0.8rem" }}
          />
          <div style={{ display: "flex", gap: "0.25rem", marginTop: "0.2rem" }}>
            {["1000", "10000", "100000"].map((u) => (
              <button
                key={u}
                type="button"
                onClick={() => setUnits(u)}
                style={{ fontSize: "0.58rem", border: `1px solid ${BORDER}`, borderRadius: "3px", background: units === u ? "#f0f0f0" : "none", padding: "0.08rem 0.3rem", cursor: "pointer", color: "#666" }}
              >
                {parseInt(u, 10).toLocaleString()}
              </button>
            ))}
          </div>
        </div>
        {orderType !== "market" && (
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: "0.6rem", color: MUTED, letterSpacing: "0.06em", marginBottom: "0.2rem" }}>
              {orderType.toUpperCase()} PRICE
            </div>
            <input
              type="text"
              value={entryPrice}
              onChange={(e) => setEntryPrice(e.target.value)}
              placeholder={hasQuote ? mid.toFixed(decimals) : ""}
              style={{ width: "100%", boxSizing: "border-box", border: `1px solid ${BORDER}`, borderRadius: "4px", padding: "0.3rem 0.45rem", fontFamily: MONO, fontSize: "0.8rem" }}
            />
          </div>
        )}
      </div>

      {/* SL / TP */}
      <div style={{ display: "flex", gap: "0.6rem", marginBottom: "0.5rem" }}>
        <RiskInput
          label="Stop loss"
          modes={["pips", "price", "%"] as const}
          mode={slMode}
          onMode={setSlMode}
          value={slValue}
          onValue={setSlValue}
          calculated={{ buy: calcSl("buy"), sell: calcSl("sell") }}
          accent={SELL}
        />
        <RiskInput
          label="Take profit"
          modes={["pips", "price", "%"] as const}
          mode={tpMode}
          onMode={setTpMode}
          value={tpValue}
          onValue={setTpValue}
          calculated={{ buy: calcTp("buy"), sell: calcTp("sell") }}
          accent={BUY}
        />
      </div>

      {/* Live toggle */}
      <label style={{ display: "flex", alignItems: "center", gap: "0.4rem", marginBottom: "0.55rem", cursor: "pointer", padding: "0.3rem 0.5rem", border: `1px solid ${live ? SELL : BORDER}`, borderRadius: "6px", background: live ? `${SELL}0d` : "none" }}>
        <input type="checkbox" checked={live} onChange={(e) => setLive(e.target.checked)} />
        <span style={{ fontSize: "0.68rem", color: live ? SELL : "#555" }}>
          {live
            ? "Live — type “yes” in the terminal to submit"
            : "Paper order (dry-run; nothing is submitted)"}
        </span>
      </label>

      {/* Sell / mid / Buy — mid sits between the buttons, like the original */}
      <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
        {sideButton("sell")}
        <span style={{ fontFamily: MONO, color: MUTED, fontSize: "0.62rem", display: "inline-flex", alignItems: "baseline", minWidth: "3.2rem", justifyContent: "center" }}>
          {hasQuote ? (
            (() => {
              const p = priceParts(mid, isJpy);
              return (
                <>
                  <span>{p.top}</span>
                  <span style={{ fontSize: "0.85rem", fontWeight: 600, color: "#555" }}>{p.big}</span>
                  <span>{p.small}</span>
                </>
              );
            })()
          ) : (
            "—"
          )}
        </span>
        {sideButton("buy")}
      </div>

      {inFlight && (
        <p style={{ textAlign: "center", fontSize: "0.72rem", color: inFlight.live ? SELL : MUTED, marginTop: "0.45rem", marginBottom: 0 }}>
          {inFlight.live
            ? "Waiting for keystroke confirmation in the terminal…"
            : "Submitting paper order…"}
        </p>
      )}

      {error && (
        <div
          onClick={() => setError(null)}
          style={{ marginTop: "0.5rem", display: "flex", justifyContent: "space-between", gap: "0.4rem", background: "#fdecea", border: "1px solid #f5c6c0", borderRadius: "4px", color: "#a31910", fontSize: "0.72rem", padding: "0.4rem 0.5rem", cursor: "pointer" }}
        >
          <span>{error}</span>
          <span style={{ color: MUTED }}>✕</span>
        </div>
      )}

      {/* Confirmation overlay — press any key to dismiss */}
      {overlay && (
        <div
          onClick={() => setOverlay(null)}
          style={{
            position: "fixed",
            inset: 0,
            zIndex: 100,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            gap: "0.4rem",
            background: overlay.side === "buy" ? BUY : SELL,
            color: "white",
            opacity: overlay.fading ? 0 : 1,
            transition: "opacity 0.5s",
            cursor: "pointer",
          }}
        >
          <div style={{ fontSize: "2.4rem", fontWeight: 700 }}>
            {overlay.side === "buy" ? "BUY" : "SELL"}
          </div>
          <div style={{ fontSize: "1rem" }}>{instrument.replace("_", "/")}</div>
          <div style={{ fontFamily: MONO, fontSize: "0.85rem", opacity: 0.85 }}>
            {overlay.result.mode === "paper"
              ? "paper — not submitted"
              : `${overlay.result.outcome ?? "submitted"}${overlay.result.price ? ` @ ${overlay.result.price}` : ""}`}
          </div>
          {overlay.result.trade_id && (
            <div style={{ fontFamily: MONO, fontSize: "0.65rem", opacity: 0.6 }}>
              trade {overlay.result.trade_id}
            </div>
          )}
          {overlay.result.consumed === true && (
            <div style={{ fontSize: "0.65rem", opacity: 0.7 }}>strategy signal consumed</div>
          )}
          <div style={{ position: "absolute", bottom: "0.8rem", fontSize: "0.6rem", opacity: 0.5 }}>
            press any key to dismiss
          </div>
        </div>
      )}
    </div>
  );
}
