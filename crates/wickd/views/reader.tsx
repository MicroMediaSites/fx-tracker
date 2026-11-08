import type { ViewProps } from "@openthink/ui-leaf/view";

/**
 * Placeholder page for the `wickd tui` news reader. wickd navigates this
 * window to each article over the DevTools protocol, so this page is only
 * visible for a moment while the first article loads.
 */
interface ReaderData {
  status?: string;
}

export default function Reader({ data }: ViewProps<ReaderData>) {
  return (
    <div
      style={{
        fontFamily: "-apple-system, BlinkMacSystemFont, sans-serif",
        background: "#151a22",
        color: "#9ca3af",
        height: "100vh",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        flexDirection: "column",
        gap: 8,
      }}
    >
      <div style={{ color: "#d1d5db", fontSize: 18 }}>wickd · news</div>
      <div style={{ fontSize: 13 }}>{data?.status ?? "opening article…"}</div>
      <div style={{ fontSize: 12, marginTop: 16 }}>
        j / k in the terminal moves between headlines
      </div>
    </div>
  );
}
