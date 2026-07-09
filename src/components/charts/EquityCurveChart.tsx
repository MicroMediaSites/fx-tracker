import { useEffect, useRef, useState } from 'react';
import {
  createChart,
  LineSeries,
  type IChartApi,
  type Time,
} from 'lightweight-charts';

interface EquityPoint {
  time: string;
  balance: string;
}

interface EquityCurveChartProps {
  data: EquityPoint[];
  height?: number;
}

export const EquityCurveChart = ({ data, height = 200 }: EquityCurveChartProps) => {
  const chartContainerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!chartContainerRef.current || data.length === 0) return;

    try {
      // Clean up any existing chart
      if (chartRef.current) {
        chartRef.current.remove();
        chartRef.current = null;
      }

      // Create chart
      const chart = createChart(chartContainerRef.current, {
        layout: {
          background: { color: '#374151' },
          textColor: '#9ca3af',
          attributionLogo: false,
        },
        grid: {
          vertLines: { color: '#4b5563' },
          horzLines: { color: '#4b5563' },
        },
        rightPriceScale: {
          borderColor: '#4b5563',
        },
        timeScale: {
          borderColor: '#4b5563',
          timeVisible: true,
          secondsVisible: false,
        },
        height,
      });

      // Determine if overall profitable
      const startBalance = parseFloat(data[0]?.balance || '0');
      const endBalance = parseFloat(data[data.length - 1]?.balance || '0');
      const isProfitable = endBalance >= startBalance;

      // Add line series
      const lineSeries = chart.addSeries(LineSeries, {
        color: isProfitable ? '#22c55e' : '#ef4444',
        lineWidth: 2,
        priceLineVisible: false,
        lastValueVisible: true,
      });

      // Convert data to chart format - filter out invalid entries
      const chartData = data
        .map((point) => {
          const timestamp = new Date(point.time).getTime();
          if (isNaN(timestamp)) return null;
          return {
            time: (timestamp / 1000) as Time,
            value: parseFloat(point.balance),
          };
        })
        .filter((p): p is { time: Time; value: number } => p !== null && !isNaN(p.value));

      if (chartData.length > 0) {
        lineSeries.setData(chartData);
        chart.timeScale().fitContent();
      }

      chartRef.current = chart;

      // Handle resize
      const handleResize = () => {
        if (chartContainerRef.current && chartRef.current) {
          chartRef.current.applyOptions({
            width: chartContainerRef.current.clientWidth,
          });
        }
      };

      window.addEventListener('resize', handleResize);
      handleResize();

      return () => {
        window.removeEventListener('resize', handleResize);
        if (chartRef.current) {
          chartRef.current.remove();
          chartRef.current = null;
        }
      };
    } catch (err) {
      console.error('EquityCurveChart error:', err);
      setError(err instanceof Error ? err.message : 'Chart error');
    }
  }, [data, height]);

  if (error) {
    return (
      <div
        className="flex items-center justify-center bg-gray-700 rounded text-red-400"
        style={{ height }}
      >
        Chart error: {error}
      </div>
    );
  }

  if (data.length === 0) {
    return (
      <div
        className="flex items-center justify-center bg-gray-700 rounded text-gray-500"
        style={{ height }}
      >
        No equity data available
      </div>
    );
  }

  return <div ref={chartContainerRef} className="w-full rounded overflow-hidden" />;
};
