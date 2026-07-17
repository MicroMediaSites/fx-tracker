/**
 * Economic-event markers as a custom series primitive.
 *
 * Replaces the v1 `createSeriesMarkers` dots for two reasons (Matt's
 * 2026-07-16 chart feedback):
 *
 * 1. FUTURE events: scheduled releases must render in the empty right
 *    margin where the next candles will appear. That margin is a
 *    `rightOffset` — pure visual space with no data points — so series
 *    markers (keyed by bar time) cannot reach it. This primitive places
 *    markers by LOGICAL index (`logicalToCoordinate`), which extends
 *    seamlessly past the last bar.
 * 2. Hover detail lives outside this file: ChartApp keys events by the
 *    same logical index and resolves crosshair moves against that map —
 *    the primitive only draws.
 *
 * Visual language: filled dot = released (past), hollow ring = scheduled
 * (future); red = the slot contains a high-impact event, neutral gray =
 * medium only. Currency letters render under the dot in a small muted
 * label, matching the muted-palette rule for working UIs.
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
  /** Logical bar index — past events snap to their candle's index; future
   * events continue past the last bar into the right offset. */
  logical: number;
  /** Any event in this slot is high impact. */
  high: boolean;
  /** The slot holds scheduled (not yet released) events. */
  upcoming: boolean;
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
  private _chart: SeriesAttachedParameter<Time>['chart'] | null = null;
  private _renderer = new EventMarkersRenderer();

  setChart(chart: SeriesAttachedParameter<Time>['chart']) {
    this._chart = chart;
  }

  setMarkers(markers: EventMarker[]) {
    this._markers = markers;
  }

  update() {
    if (!this._chart) return;
    const timeScale = this._chart.timeScale();
    this._renderer.update(
      this._markers.map((m) => ({
        x: timeScale.logicalToCoordinate(m.logical as Logical),
        high: m.high,
        upcoming: m.upcoming,
        label: m.label,
      }))
    );
  }

  renderer() {
    return this._renderer;
  }
}

export class EventMarkersPlugin implements ISeriesPrimitive<Time> {
  private _paneView = new EventMarkersPaneView();
  private _requestUpdate?: () => void;
  private _pendingMarkers?: EventMarker[];

  attached(param: SeriesAttachedParameter<Time>) {
    this._paneView.setChart(param.chart);
    this._requestUpdate = param.requestUpdate;
    if (this._pendingMarkers) {
      this._paneView.setMarkers(this._pendingMarkers);
      this._pendingMarkers = undefined;
      this._requestUpdate();
    }
  }

  detached() {
    this._requestUpdate = undefined;
  }

  setMarkers(markers: EventMarker[]) {
    this._paneView.setMarkers(markers);
    if (this._requestUpdate) {
      this._requestUpdate();
    } else {
      this._pendingMarkers = markers;
    }
  }

  updateAllViews() {
    this._paneView.update();
  }

  paneViews() {
    return [this._paneView];
  }
}
