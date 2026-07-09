import {
  ISeriesPrimitive,
  IPrimitivePaneView,
  IPrimitivePaneRenderer,
  SeriesAttachedParameter,
  Time,
  Coordinate,
} from 'lightweight-charts';
import { CanvasRenderingTarget2D } from 'fancy-canvas';

export interface CloudPoint {
  time: number; // Unix timestamp in seconds
  senkou_a: number;
  senkou_b: number;
}

interface CloudViewPoint {
  x: Coordinate | null;
  y_a: Coordinate | null;
  y_b: Coordinate | null;
  isBullish: boolean;
}

class IchimokuCloudRenderer implements IPrimitivePaneRenderer {
  private _points: CloudViewPoint[] = [];

  update(points: CloudViewPoint[]) {
    this._points = points;
  }

  draw(target: CanvasRenderingTarget2D) {
    target.useBitmapCoordinateSpace((scope) => {
      const ctx = scope.context;
      const horizontalPixelRatio = scope.horizontalPixelRatio;
      const verticalPixelRatio = scope.verticalPixelRatio;

      if (this._points.length < 2) return;

      // Filter out points with null coordinates
      const validPoints = this._points.filter(
        (p) => p.x !== null && p.y_a !== null && p.y_b !== null
      );

      if (validPoints.length < 2) return;

      // Draw cloud segments - we need to handle color changes at crossovers
      let segmentStart = 0;

      for (let i = 1; i <= validPoints.length; i++) {
        // Check if we need to end a segment (color change or end of data)
        const endSegment =
          i === validPoints.length ||
          validPoints[i].isBullish !== validPoints[segmentStart].isBullish;

        if (endSegment && i > segmentStart) {
          const segmentPoints = validPoints.slice(segmentStart, i + (i < validPoints.length ? 1 : 0));
          const isBullish = validPoints[segmentStart].isBullish;

          // Draw filled cloud for this segment (OANDA-style muted colors)
          ctx.beginPath();
          ctx.fillStyle = isBullish
            ? 'rgba(118, 168, 126, 0.4)' // Muted green with transparency
            : 'rgba(194, 120, 120, 0.4)'; // Muted red/pink with transparency

          // Draw top edge (senkou_a or senkou_b, whichever is higher)
          const firstPoint = segmentPoints[0];
          const topY = isBullish ? firstPoint.y_a! : firstPoint.y_b!;
          ctx.moveTo(firstPoint.x! * horizontalPixelRatio, topY * verticalPixelRatio);

          for (let j = 1; j < segmentPoints.length; j++) {
            const p = segmentPoints[j];
            const y = isBullish ? p.y_a! : p.y_b!;
            ctx.lineTo(p.x! * horizontalPixelRatio, y * verticalPixelRatio);
          }

          // Draw bottom edge in reverse (senkou_b or senkou_a, whichever is lower)
          for (let j = segmentPoints.length - 1; j >= 0; j--) {
            const p = segmentPoints[j];
            const y = isBullish ? p.y_b! : p.y_a!;
            ctx.lineTo(p.x! * horizontalPixelRatio, y * verticalPixelRatio);
          }

          ctx.closePath();
          ctx.fill();

          // Start new segment
          if (i < validPoints.length) {
            segmentStart = i;
          }
        }
      }
    });
  }
}

class IchimokuCloudPaneView implements IPrimitivePaneView {
  private _cloudPoints: CloudPoint[];
  private _series: SeriesAttachedParameter<Time>['series'] | null = null;
  private _chart: SeriesAttachedParameter<Time>['chart'] | null = null;
  private _renderer: IchimokuCloudRenderer;

  constructor(cloudPoints: CloudPoint[]) {
    this._cloudPoints = cloudPoints;
    this._renderer = new IchimokuCloudRenderer();
  }

  setSeriesAndChart(
    series: SeriesAttachedParameter<Time>['series'],
    chart: SeriesAttachedParameter<Time>['chart']
  ) {
    this._series = series;
    this._chart = chart;
  }

  clear() {
    this._cloudPoints = [];
    this._renderer.update([]);
  }

  update() {
    if (!this._series || !this._chart) return;

    const timeScale = this._chart.timeScale();

    const viewPoints: CloudViewPoint[] = this._cloudPoints.map((point) => {
      const x = timeScale.timeToCoordinate(point.time as Time);
      const y_a = this._series!.priceToCoordinate(point.senkou_a);
      const y_b = this._series!.priceToCoordinate(point.senkou_b);

      return {
        x,
        y_a,
        y_b,
        isBullish: point.senkou_a >= point.senkou_b,
      };
    });

    this._renderer.update(viewPoints);
  }

  renderer() {
    return this._renderer;
  }
}

export class IchimokuCloudPlugin implements ISeriesPrimitive<Time> {
  private _paneView: IchimokuCloudPaneView;
  private _requestUpdate?: () => void;

  constructor(cloudPoints: CloudPoint[]) {
    this._paneView = new IchimokuCloudPaneView(cloudPoints);
  }

  attached(param: SeriesAttachedParameter<Time>) {
    this._paneView.setSeriesAndChart(param.series, param.chart);
    this._requestUpdate = param.requestUpdate;
  }

  detached() {
    this._requestUpdate = undefined;
    // Clear cloud points to ensure nothing renders if detachment fails
    this._paneView.clear();
  }

  updateAllViews() {
    this._paneView.update();
  }

  paneViews() {
    return [this._paneView];
  }

  requestUpdate() {
    this._requestUpdate?.();
  }
}
