/**
 * Economic-event markers as a custom series primitive.
 *
 * Placement contract (learned the hard way — see the 2026-07-16 misplot):
 *
 * - PAST releases are keyed by their candle's BUSINESS TIME and resolved
 *   with `timeToCoordinate`. Never by raw bar index: the time scale's
 *   logical 0 is its earliest point across ALL series, and indicator
 *   series fetch deeper history than the candles (SMA warmup etc.), so
 *   candle index ≠ logical index.
 * - FUTURE (scheduled) releases render in the right-offset margin, which
 *   has no data points, so time keys can't reach it. They are placed as
 *   logical offsets from the LAST BAR, whose true logical position is
 *   derived from the time scale itself:
 *   `coordinateToLogical(timeToCoordinate(anchorBusinessTime))`.
 *
 * Visual language: filled dot = released (past), hollow ring = scheduled
 * (future); red = the slot contains a high-impact event, neutral gray =
 * medium only. Currency letters render under the dot in a small muted
 * label. Hover detail lives in ChartApp (crosshair-driven tooltip); the
 * primitive only draws.
 */
import {
  ISeriesPrimitive,
  IPrimitivePaneView,
  IPrimitivePaneRenderer,
  SeriesAttachedParameter,
  Time,
  Logical,
} from 'lightweight-charts';
import { CanvasRenderingTarget2D } from 'fancy-canvas';

export interface EventMarker {
  /** Business time of the candle containing a PAST release. Mutually
   * exclusive with `futureSlots`. */
  businessTime?: number;
  /** Whole granularity-slots ahead of the last bar for a SCHEDULED
   * release. Mutually exclusive with `businessTime`. */
  futureSlots?: number;
  /** Any event in this slot is high impact. */
  high: boolean;
  /** Space-joined currency legs, e.g. "GBP USD". */
  label: string;
}

const MARKER_Y_PX = 14;
const LABEL_Y_PX = 30;
const RADIUS_PX = 4;
const HIGH_COLOR = '#f23645';
const MED_COLOR = '#787b86';

interface MarkerView {
  x: number | null;
  high: boolean;
  upcoming: boolean;
  label: string;
}

class EventMarkersRenderer implements IPrimitivePaneRenderer {
  private _views: MarkerView[] = [];

  update(views: MarkerView[]) {
    this._views = views;
  }

  draw(target: CanvasRenderingTarget2D) {
    target.useBitmapCoordinateSpace((scope) => {
      const ctx = scope.context;
      const hr = scope.horizontalPixelRatio;
      const vr = scope.verticalPixelRatio;

      for (const v of this._views) {
        if (v.x === null) continue;
        const x = v.x * hr;
        const y = MARKER_Y_PX * vr;
        const color = v.high ? HIGH_COLOR : MED_COLOR;

        ctx.beginPath();
        ctx.arc(x, y, RADIUS_PX * hr, 0, 2 * Math.PI);
        if (v.upcoming) {
          ctx.strokeStyle = color;
          ctx.lineWidth = 1.5 * hr;
          ctx.stroke();
        } else {
          ctx.fillStyle = color;
          ctx.fill();
        }

        ctx.font = `${Math.round(9 * vr)}px -apple-system, sans-serif`;
        ctx.fillStyle = MED_COLOR;
        ctx.textAlign = 'center';
        ctx.fillText(v.label, x, LABEL_Y_PX * vr);
      }
    });
  }
}

class EventMarkersPaneView implements IPrimitivePaneView {
  private _markers: EventMarker[] = [];
  private _anchorBusinessTime: number | null = null;
  private _chart: SeriesAttachedParameter<Time>['chart'] | null = null;
  private _renderer = new EventMarkersRenderer();

  setChart(chart: SeriesAttachedParameter<Time>['chart']) {
    this._chart = chart;
  }

  setMarkers(markers: EventMarker[], anchorBusinessTime: number | null) {
    this._markers = markers;
    this._anchorBusinessTime = anchorBusinessTime;
  }

  update() {
    if (!this._chart) return;
    const timeScale = this._chart.timeScale();

    // Anchor for future offsets: the last bar's TRUE logical position,
    // asked of the time scale (never assumed from array indices).
    let anchorLogical: number | null = null;
    if (this._anchorBusinessTime !== null) {
      const anchorX = timeScale.timeToCoordinate(this._anchorBusinessTime as Time);
      if (anchorX !== null) {
        const logical = timeScale.coordinateToLogical(anchorX);
        if (logical !== null) anchorLogical = Math.round(logical as number);
      }
    }

    this._renderer.update(
      this._markers.map((m) => {
        let x: number | null = null;
        if (m.businessTime !== undefined) {
          x = timeScale.timeToCoordinate(m.businessTime as Time);
        } else if (m.futureSlots !== undefined && anchorLogical !== null) {
          x = timeScale.logicalToCoordinate((anchorLogical + m.futureSlots) as Logical);
        }
        return {
          x,
          high: m.high,
          upcoming: m.futureSlots !== undefined,
          label: m.label,
        };
      })
    );
  }

  renderer() {
    return this._renderer;
  }
}

export class EventMarkersPlugin implements ISeriesPrimitive<Time> {
  private _paneView = new EventMarkersPaneView();
  private _requestUpdate?: () => void;
  private _pending?: { markers: EventMarker[]; anchor: number | null };

  attached(param: SeriesAttachedParameter<Time>) {
    this._paneView.setChart(param.chart);
    this._requestUpdate = param.requestUpdate;
    if (this._pending) {
      this._paneView.setMarkers(this._pending.markers, this._pending.anchor);
      this._pending = undefined;
      this._requestUpdate();
    }
  }

  detached() {
    this._requestUpdate = undefined;
  }

  setMarkers(markers: EventMarker[], anchorBusinessTime: number | null) {
    this._paneView.setMarkers(markers, anchorBusinessTime);
    if (this._requestUpdate) {
      this._requestUpdate();
    } else {
      this._pending = { markers, anchor: anchorBusinessTime };
    }
  }

  updateAllViews() {
    this._paneView.update();
  }

  paneViews() {
    return [this._paneView];
  }
}
