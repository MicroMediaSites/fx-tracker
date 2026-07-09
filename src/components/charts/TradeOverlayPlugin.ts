import {
  ISeriesPrimitive,
  IPrimitivePaneView,
  IPrimitivePaneRenderer,
  SeriesAttachedParameter,
  Time,
  Coordinate,
} from 'lightweight-charts';
import { CanvasRenderingTarget2D } from 'fancy-canvas';

export interface TradeData {
  entryTime: number; // Unix timestamp in seconds
  exitTime: number;
  entryPrice: number;
  exitPrice: number;
  direction: 'long' | 'short';
  pnl: number;
}

interface ViewPoint {
  x: Coordinate | null;
  y: Coordinate | null;
}

interface TradeView {
  entry: ViewPoint;
  exit: ViewPoint;
  pnl: number;
  direction: 'long' | 'short';
}

class TradeOverlayRenderer implements IPrimitivePaneRenderer {
  private _trades: TradeView[] = [];

  update(trades: TradeView[]) {
    this._trades = trades;
  }

  draw(target: CanvasRenderingTarget2D) {
    target.useBitmapCoordinateSpace((scope) => {
      const ctx = scope.context;
      const horizontalPixelRatio = scope.horizontalPixelRatio;
      const verticalPixelRatio = scope.verticalPixelRatio;

      for (const trade of this._trades) {
        if (
          trade.entry.x === null ||
          trade.entry.y === null ||
          trade.exit.x === null ||
          trade.exit.y === null
        ) {
          continue;
        }

        // Scale coordinates to bitmap space
        const entryX = trade.entry.x * horizontalPixelRatio;
        const entryY = trade.entry.y * verticalPixelRatio;
        const exitX = trade.exit.x * horizontalPixelRatio;
        const exitY = trade.exit.y * verticalPixelRatio;

        // Determine colors based on P/L
        const isProfitable = trade.pnl >= 0;
        const lineColor = isProfitable ? 'rgba(34, 197, 94, 0.6)' : 'rgba(239, 68, 68, 0.6)';
        const fillColor = isProfitable ? 'rgba(34, 197, 94, 0.15)' : 'rgba(239, 68, 68, 0.15)';
        const markerColor = isProfitable ? '#22c55e' : '#ef4444';

        // Draw filled area between entry and exit (box showing trade duration)
        ctx.fillStyle = fillColor;
        ctx.beginPath();
        ctx.rect(
          Math.min(entryX, exitX),
          Math.min(entryY, exitY),
          Math.abs(exitX - entryX),
          Math.abs(exitY - entryY)
        );
        ctx.fill();

        // Draw connecting line from entry to exit
        ctx.strokeStyle = lineColor;
        ctx.lineWidth = 2 * horizontalPixelRatio;
        ctx.setLineDash([]);
        ctx.beginPath();
        ctx.moveTo(entryX, entryY);
        ctx.lineTo(exitX, exitY);
        ctx.stroke();

        // Draw entry marker (circle)
        const markerRadius = 5 * horizontalPixelRatio;
        ctx.fillStyle = trade.direction === 'long' ? '#22c55e' : '#ef4444';
        ctx.beginPath();
        ctx.arc(entryX, entryY, markerRadius, 0, 2 * Math.PI);
        ctx.fill();
        ctx.strokeStyle = '#ffffff';
        ctx.lineWidth = 1 * horizontalPixelRatio;
        ctx.stroke();

        // Draw exit marker (square)
        const squareSize = 8 * horizontalPixelRatio;
        ctx.fillStyle = markerColor;
        ctx.beginPath();
        ctx.rect(exitX - squareSize / 2, exitY - squareSize / 2, squareSize, squareSize);
        ctx.fill();
        ctx.strokeStyle = '#ffffff';
        ctx.lineWidth = 1 * horizontalPixelRatio;
        ctx.strokeRect(exitX - squareSize / 2, exitY - squareSize / 2, squareSize, squareSize);

        // Draw P/L label near exit
        const pnlText = `${trade.pnl >= 0 ? '+' : ''}${trade.pnl.toFixed(2)}`;
        const fontSize = 10 * horizontalPixelRatio;
        ctx.font = `${fontSize}px sans-serif`;
        ctx.fillStyle = markerColor;
        ctx.textAlign = 'left';
        ctx.fillText(pnlText, exitX + 8 * horizontalPixelRatio, exitY + 3 * verticalPixelRatio);
      }
    });
  }
}

class TradeOverlayPaneView implements IPrimitivePaneView {
  private _trades: TradeData[];
  private _series: SeriesAttachedParameter<Time>['series'] | null = null;
  private _chart: SeriesAttachedParameter<Time>['chart'] | null = null;
  private _renderer: TradeOverlayRenderer;
  private _tradeViews: TradeView[] = [];

  constructor(trades: TradeData[]) {
    this._trades = trades;
    this._renderer = new TradeOverlayRenderer();
  }

  setTrades(trades: TradeData[]) {
    this._trades = trades;
  }

  setSeriesAndChart(series: SeriesAttachedParameter<Time>['series'], chart: SeriesAttachedParameter<Time>['chart']) {
    this._series = series;
    this._chart = chart;
  }

  updateTrades(trades: TradeData[]) {
    this._trades = trades;
  }

  update() {
    if (!this._series || !this._chart) return;

    const timeScale = this._chart.timeScale();

    this._tradeViews = this._trades.map((trade) => {
      const entryX = timeScale.timeToCoordinate(trade.entryTime as Time);
      const exitX = timeScale.timeToCoordinate(trade.exitTime as Time);
      const entryY = this._series!.priceToCoordinate(trade.entryPrice);
      const exitY = this._series!.priceToCoordinate(trade.exitPrice);

      return {
        entry: { x: entryX, y: entryY },
        exit: { x: exitX, y: exitY },
        pnl: trade.pnl,
        direction: trade.direction,
      };
    });

    this._renderer.update(this._tradeViews);
  }

  renderer() {
    return this._renderer;
  }
}

export class TradeOverlayPlugin implements ISeriesPrimitive<Time> {
  private _paneView: TradeOverlayPaneView;
  private _requestUpdate?: () => void;
  private _pendingTrades?: TradeData[];

  constructor(trades: TradeData[]) {
    this._paneView = new TradeOverlayPaneView(trades);
  }

  attached(param: SeriesAttachedParameter<Time>) {
    this._paneView.setSeriesAndChart(param.series, param.chart);
    this._requestUpdate = param.requestUpdate;
    // Apply any trades that were queued before attachment
    if (this._pendingTrades) {
      this._paneView.updateTrades(this._pendingTrades);
      this._pendingTrades = undefined;
      this._requestUpdate();
    }
  }

  detached() {
    this._requestUpdate = undefined;
  }

  updateTrades(trades: TradeData[]) {
    this._paneView.updateTrades(trades);
    if (this._requestUpdate) {
      this._requestUpdate();
    } else {
      // Plugin not yet attached — queue for when attached() is called
      this._pendingTrades = trades;
    }
  }

  updateAllViews() {
    this._paneView.update();
  }

  paneViews() {
    return [this._paneView];
  }
}
