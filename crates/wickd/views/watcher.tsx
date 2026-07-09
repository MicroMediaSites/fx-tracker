import type { ViewProps } from "@openthink/ui-leaf/view";

interface SignalRow {
  /** Wall-clock time the signal was observed (HH:MM:SS). */
  time?: string;
  /** Coarse category: "match" | "status" | "error". */
  kind?: string;
  /** Instrument the signal is about. */
  instrument?: string;
  /** Human-readable detail (reason / message / status text). */
  label?: string;
  /** "long" | "short" when the signal carries a direction. */
  direction?: string | null;
}

interface TickInfo {
  time?: string;
  close?: string;
  signal?: string;
}

interface WatcherData {
  strategy?: string;
  instrument?: string;
  granularity?: string;
  /** "starting" | "running" | "stopped" | a daemon-reported status. */
  status?: string;
  monitoring?: boolean;
  signals?: SignalRow[];
  lastTick?: TickInfo | null;
  lastError?: string | null;
  tickCount?: number;
  matchCount?: number;
  [key: string]: unknown;
}

const KIND_COLOR: Record<string, string> = {
  match: "#0a7d32",
  status: "#3358cc",
  error: "#c0271a",
};

/**
 * Live signal monitor — a ui-leaf view over the `wickd watch` daemon (AGT-593).
 *
 * wickd pushes a fresh `SignalState` (via the stdio protocol's `update`
 * message) on every NDJSON signal the daemon emits — pattern matches, ticks,
 * status, and errors — so this view re-renders live. It is a read-only monitor:
 * no orders, no mutations. Data is supplied entirely by the CLI.
 */
export default function Watcher({ data }: ViewProps<WatcherData>) {
  const strategy = data.strategy ?? "strategy";
  const instrument = (data.instrument ?? "EUR_USD").replace("_", "/");
  const granularity = data.granularity ?? "";
  const status = data.status ?? "starting";
  const monitoring = data.monitoring ?? true;
  const signals = Array.isArray(data.signals) ? data.signals : [];
  const tick = data.lastTick ?? null;
  const lastError = data.lastError ?? null;
  const tickCount = data.tickCount ?? 0;
  const matchCount = data.matchCount ?? 0;

  const pillColor = monitoring ? "#0a7d32" : "#888";

  return (
    <div
      style={{
        fontFamily: "system-ui, -apple-system, sans-serif",
        padding: "1.5rem",
        maxWidth: "44rem",
        margin: "0 auto",
        color: "#1a1a1a",
      }}
    >
      <header
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "baseline",
          marginBottom: "0.25rem",
        }}
      >
        <h1 style={{ fontSize: "1.4rem", margin: 0, letterSpacing: "0.02em" }}>
          Monitoring {instrument}
        </h1>
        <span
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "0.4rem",
            fontSize: "0.78rem",
            fontWeight: 700,
            textTransform: "uppercase",
            color: pillColor,
          }}
        >
          <span
            style={{
              width: "0.55rem",
              height: "0.55rem",
              borderRadius: "50%",
              background: pillColor,
              boxShadow: monitoring ? `0 0 0 0.18rem ${pillColor}33` : "none",
            }}
          />
          {monitoring ? status : status || "stopped"}
        </span>
      </header>

      <p style={{ color: "#777", fontSize: "0.85rem", margin: "0 0 1rem" }}>
        {strategy}
        {granularity ? ` · ${granularity}` : ""} · live signals from the wickd watch daemon
      </p>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(3, 1fr)",
          gap: "0.5rem",
          marginBottom: "1.25rem",
        }}
      >
        <Stat label="TICKS" value={String(tickCount)} accent="#555" />
        <Stat label="MATCHES" value={String(matchCount)} accent="#0a7d32" />
        <Stat
          label="LAST CLOSE"
          value={tick?.close ? tick.close : "—"}
          accent="#3358cc"
          sub={tick?.signal ? `signal: ${tick.signal}` : undefined}
        />
      </div>

      {lastError && (
        <div
          style={{
            background: "#fdecea",
            border: "1px solid #f5c2bd",
            color: "#a31910",
            borderRadius: "0.4rem",
            padding: "0.5rem 0.75rem",
            fontSize: "0.82rem",
            marginBottom: "1rem",
          }}
        >
          {lastError}
        </div>
      )}

      <h2 style={{ fontSize: "0.8rem", color: "#999", letterSpacing: "0.08em", margin: "0 0 0.5rem" }}>
        SIGNAL LOG
      </h2>

      {signals.length === 0 ? (
        <p style={{ color: "#aaa", fontSize: "0.9rem", padding: "1.5rem 0", textAlign: "center" }}>
          {monitoring ? "Waiting for signals…" : "No signals received."}
        </p>
      ) : (
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: "0.85rem" }}>
          <thead>
            <tr style={{ textAlign: "left", color: "#999", fontSize: "0.7rem", letterSpacing: "0.05em" }}>
              <th style={th}>TIME</th>
              <th style={th}>TYPE</th>
              <th style={th}>INSTRUMENT</th>
              <th style={th}>SIGNAL</th>
            </tr>
          </thead>
          <tbody>
            {signals.map((s, i) => (
              <tr key={i} style={{ borderTop: "1px solid #eee" }}>
                <td style={{ ...td, fontVariantNumeric: "tabular-nums", color: "#888" }}>
                  {s.time ?? ""}
                </td>
                <td style={td}>
                  <span
                    style={{
                      fontSize: "0.7rem",
                      fontWeight: 700,
                      textTransform: "uppercase",
                      color: KIND_COLOR[s.kind ?? ""] ?? "#555",
                    }}
                  >
                    {s.kind ?? "signal"}
                  </span>
                </td>
                <td style={{ ...td, fontWeight: 600 }}>
                  {(s.instrument ?? "").replace("_", "/")}
                </td>
                <td style={td}>
                  {s.direction ? (
                    <span
                      style={{
                        fontWeight: 700,
                        textTransform: "uppercase",
                        color: s.direction === "long" ? "#0a7d32" : "#c0271a",
                        marginRight: "0.4rem",
                      }}
                    >
                      {s.direction}
                    </span>
                  ) : null}
                  <span style={{ color: "#444" }}>{s.label ?? ""}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <p style={{ color: "#bbb", fontSize: "0.72rem", marginTop: "1.25rem", textAlign: "center" }}>
        Read-only monitor — no orders are placed. Close this tab or press Ctrl-C to stop.
      </p>
    </div>
  );
}

function Stat({
  label,
  value,
  accent,
  sub,
}: {
  label: string;
  value: string;
  accent: string;
  sub?: string;
}) {
  return (
    <div style={{ background: "#f7f7f7", borderRadius: "0.4rem", padding: "0.6rem 0.5rem", textAlign: "center" }}>
      <div style={{ fontSize: "0.65rem", color: "#999", letterSpacing: "0.08em" }}>{label}</div>
      <div style={{ fontVariantNumeric: "tabular-nums", fontWeight: 700, fontSize: "1.05rem", color: accent }}>
        {value}
      </div>
      {sub && <div style={{ fontSize: "0.66rem", color: "#aaa", marginTop: "0.1rem" }}>{sub}</div>}
    </div>
  );
}

const th: React.CSSProperties = { padding: "0.3rem 0.4rem", fontWeight: 600 };
const td: React.CSSProperties = { padding: "0.45rem 0.4rem", verticalAlign: "top" };
