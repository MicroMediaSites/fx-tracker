import { useEffect, useRef, useCallback } from 'react';
import type { IChartApi, ISeriesApi } from 'lightweight-charts';

export interface SRZone {
  id: string;
  upper_price: number;
  lower_price: number;
  label?: string;
  color?: string;
}

interface PreviewZone {
  upperY: number;
  lowerY: number;
}

interface SRZoneOverlayProps {
  chart: IChartApi | null;
  series: ISeriesApi<'Candlestick'> | null;
  zones: SRZone[];
  previewZone: PreviewZone | null;
  selectedZoneId: string | null;
  showEditButton?: boolean;
  containerRef: React.RefObject<HTMLDivElement | null>;
}

// Colors
const DEFAULT_ZONE_COLOR = 'rgba(59, 130, 246, 0.15)';
const DEFAULT_BORDER_COLOR = 'rgba(59, 130, 246, 0.5)';
const PREVIEW_ZONE_COLOR = 'rgba(96, 165, 250, 0.2)';
const PREVIEW_BORDER_COLOR = 'rgba(96, 165, 250, 0.6)';

// Helper to derive border color from fill color by increasing alpha
const getBorderColor = (fillColor: string): string => {
  // Parse rgba(r, g, b, a) and increase alpha for border
  const match = fillColor.match(/rgba?\((\d+),\s*(\d+),\s*(\d+),?\s*([\d.]+)?\)/);
  if (match) {
    const [, r, g, b, a] = match;
    const alpha = parseFloat(a || '0.15');
    // Increase alpha for border (0.15 -> 0.5, 0.25 -> 0.7)
    const borderAlpha = Math.min(alpha + 0.35, 1);
    return `rgba(${r}, ${g}, ${b}, ${borderAlpha})`;
  }
  return DEFAULT_BORDER_COLOR;
};

// Get selected variant of a color (slightly higher alpha)
const getSelectedColor = (fillColor: string): string => {
  const match = fillColor.match(/rgba?\((\d+),\s*(\d+),\s*(\d+),?\s*([\d.]+)?\)/);
  if (match) {
    const [, r, g, b, a] = match;
    const alpha = parseFloat(a || '0.15');
    // Increase alpha for selection (0.15 -> 0.25)
    const selectedAlpha = Math.min(alpha + 0.1, 0.4);
    return `rgba(${r}, ${g}, ${b}, ${selectedAlpha})`;
  }
  return 'rgba(59, 130, 246, 0.25)';
};

export const SRZoneOverlay = ({
  chart,
  series,
  zones,
  previewZone,
  selectedZoneId,
  showEditButton,
  containerRef,
}: SRZoneOverlayProps) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animationFrameRef = useRef<number | null>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container || !chart || !series) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    // Get chart dimensions
    const rect = container.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;

    // Resize canvas if needed
    if (canvas.width !== rect.width * dpr || canvas.height !== rect.height * dpr) {
      canvas.width = rect.width * dpr;
      canvas.height = rect.height * dpr;
      canvas.style.width = `${rect.width}px`;
      canvas.style.height = `${rect.height}px`;
      ctx.scale(dpr, dpr);
    }

    // Clear canvas
    ctx.clearRect(0, 0, rect.width, rect.height);

    const chartWidth = chart.timeScale().width();

    // Minimum visual height for single-line zones (for rendering and hit detection)
    const MIN_ZONE_HEIGHT = 10;

    // Draw saved zones
    for (const zone of zones) {
      const upperY = series.priceToCoordinate(zone.upper_price);
      const lowerY = series.priceToCoordinate(zone.lower_price);

      if (upperY === null || lowerY === null) continue;

      const minY = Math.min(upperY, lowerY);
      const maxY = Math.max(upperY, lowerY);
      const rawHeight = maxY - minY;
      // Use minimum height for single-line zones
      const height = Math.max(rawHeight, MIN_ZONE_HEIGHT);
      const drawMinY = rawHeight < MIN_ZONE_HEIGHT ? minY - MIN_ZONE_HEIGHT / 2 : minY;

      const isSelected = zone.id === selectedZoneId;
      const baseColor = zone.color || DEFAULT_ZONE_COLOR;
      const fillColor = isSelected ? getSelectedColor(baseColor) : baseColor;
      const borderColor = isSelected ? getBorderColor(getSelectedColor(baseColor)) : getBorderColor(baseColor);

      // Draw filled zone
      ctx.fillStyle = fillColor;
      ctx.fillRect(0, drawMinY, chartWidth, height);

      // Draw border lines
      ctx.strokeStyle = borderColor;
      ctx.lineWidth = isSelected ? 2 : 1;

      // For single-line zones, draw one thicker line at the price level
      if (rawHeight < MIN_ZONE_HEIGHT) {
        ctx.lineWidth = isSelected ? 3 : 2;
        ctx.beginPath();
        ctx.moveTo(0, minY);
        ctx.lineTo(chartWidth, minY);
        ctx.stroke();
      } else {
        ctx.beginPath();
        ctx.moveTo(0, minY);
        ctx.lineTo(chartWidth, minY);
        ctx.stroke();

        ctx.beginPath();
        ctx.moveTo(0, maxY);
        ctx.lineTo(chartWidth, maxY);
        ctx.stroke();
      }

      // Draw label if present
      if (zone.label) {
        ctx.font = '11px Inter, system-ui, sans-serif';
        ctx.fillStyle = 'rgba(255, 255, 255, 0.9)';
        ctx.textBaseline = 'middle';

        // Draw label background pill
        const labelText = zone.label;
        const textMetrics = ctx.measureText(labelText);
        const labelPadding = 6;
        const labelHeight = 18;
        const labelWidth = textMetrics.width + labelPadding * 2;
        const labelX = 8;
        const labelY = drawMinY + (height / 2) - (labelHeight / 2);

        // Background pill
        ctx.fillStyle = 'rgba(0, 0, 0, 0.6)';
        ctx.beginPath();
        ctx.roundRect(labelX, labelY, labelWidth, labelHeight, 4);
        ctx.fill();

        // Label text
        ctx.fillStyle = 'rgba(255, 255, 255, 0.9)';
        ctx.fillText(labelText, labelX + labelPadding, drawMinY + (height / 2));
      }

      // Draw edit and delete buttons when selected
      if (isSelected) {
        const buttonSize = 20;
        const buttonSpacing = 6;
        const deleteButtonX = chartWidth - buttonSize - 10;
        const editButtonX = deleteButtonX - buttonSize - buttonSpacing;
        // Center buttons on the zone
        const buttonY = drawMinY + (height / 2) - (buttonSize / 2);

        // Edit button (if showEditButton is true)
        if (showEditButton) {
          ctx.fillStyle = 'rgba(59, 130, 246, 0.9)';
          ctx.beginPath();
          ctx.arc(editButtonX + buttonSize / 2, buttonY + buttonSize / 2, buttonSize / 2, 0, Math.PI * 2);
          ctx.fill();

          // Pencil icon
          ctx.strokeStyle = 'white';
          ctx.lineWidth = 1.5;
          ctx.beginPath();
          // Pencil body (diagonal line)
          ctx.moveTo(editButtonX + 6, buttonY + 14);
          ctx.lineTo(editButtonX + 14, buttonY + 6);
          ctx.stroke();
          // Pencil tip
          ctx.beginPath();
          ctx.moveTo(editButtonX + 5, buttonY + 15);
          ctx.lineTo(editButtonX + 6, buttonY + 14);
          ctx.lineTo(editButtonX + 7, buttonY + 15);
          ctx.closePath();
          ctx.fillStyle = 'white';
          ctx.fill();
        }

        // Delete button
        ctx.fillStyle = 'rgba(239, 68, 68, 0.9)';
        ctx.beginPath();
        ctx.arc(deleteButtonX + buttonSize / 2, buttonY + buttonSize / 2, buttonSize / 2, 0, Math.PI * 2);
        ctx.fill();

        // X icon
        ctx.strokeStyle = 'white';
        ctx.lineWidth = 2;
        const padding = 6;
        ctx.beginPath();
        ctx.moveTo(deleteButtonX + padding, buttonY + padding);
        ctx.lineTo(deleteButtonX + buttonSize - padding, buttonY + buttonSize - padding);
        ctx.moveTo(deleteButtonX + buttonSize - padding, buttonY + padding);
        ctx.lineTo(deleteButtonX + padding, buttonY + buttonSize - padding);
        ctx.stroke();
      }
    }

    // Draw preview zone
    if (previewZone) {
      const minY = Math.min(previewZone.upperY, previewZone.lowerY);
      const maxY = Math.max(previewZone.upperY, previewZone.lowerY);
      const height = maxY - minY;

      // Draw filled preview zone
      ctx.fillStyle = PREVIEW_ZONE_COLOR;
      ctx.fillRect(0, minY, chartWidth, height);

      // Draw border lines (dashed)
      ctx.strokeStyle = PREVIEW_BORDER_COLOR;
      ctx.lineWidth = 2;
      ctx.setLineDash([5, 5]);

      ctx.beginPath();
      ctx.moveTo(0, minY);
      ctx.lineTo(chartWidth, minY);
      ctx.stroke();

      ctx.beginPath();
      ctx.moveTo(0, maxY);
      ctx.lineTo(chartWidth, maxY);
      ctx.stroke();

      ctx.setLineDash([]);
    }
  }, [chart, series, zones, previewZone, selectedZoneId, showEditButton, containerRef]);

  // Redraw on data changes
  useEffect(() => {
    draw();
  }, [draw]);

  // Subscribe to chart updates for continuous redraw during scroll/zoom
  useEffect(() => {
    if (!chart) return;

    const handleTimeScaleChange = () => {
      if (animationFrameRef.current) {
        cancelAnimationFrame(animationFrameRef.current);
      }
      animationFrameRef.current = requestAnimationFrame(draw);
    };

    // Subscribe to time scale changes
    chart.timeScale().subscribeVisibleTimeRangeChange(handleTimeScaleChange);
    chart.timeScale().subscribeVisibleLogicalRangeChange(handleTimeScaleChange);

    return () => {
      chart.timeScale().unsubscribeVisibleTimeRangeChange(handleTimeScaleChange);
      chart.timeScale().unsubscribeVisibleLogicalRangeChange(handleTimeScaleChange);
      if (animationFrameRef.current) {
        cancelAnimationFrame(animationFrameRef.current);
      }
    };
  }, [chart, draw]);

  return (
    <canvas
      ref={canvasRef}
      style={{
        position: 'absolute',
        top: 0,
        left: 0,
        // Let all events pass through to chart for zoom/pan - we detect clicks on parent container
        pointerEvents: 'none',
        zIndex: 10,
      }}
    />
  );
};
