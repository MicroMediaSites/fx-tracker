import { useRef, useEffect, useState } from 'react';

export type PriceDirection = 'up' | 'down' | 'neutral';

/**
 * Hook to track price changes and return a direction for color flashing.
 * Returns 'up' when price increases, 'down' when price decreases, 'neutral' if unchanged.
 *
 * Accepts string prices to avoid floating point precision issues.
 * Uses state + timer to ensure flash is visible for a minimum duration.
 */
export function usePriceFlash(price: string | null | undefined, flashDuration = 500): PriceDirection {
  const [direction, setDirection] = useState<PriceDirection>('neutral');
  const prevPriceRef = useRef<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const currentPrice = price ?? null;
    const prevPrice = prevPriceRef.current;

    // Clear any pending reset timer
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }

    // Check for price change (string comparison ensures all changes detected)
    if (currentPrice !== null && prevPrice !== null && currentPrice !== prevPrice) {
      const curr = parseFloat(currentPrice);
      const prev = parseFloat(prevPrice);
      const newDirection = curr > prev ? 'up' : 'down';
      setDirection(newDirection);

      // Reset to neutral after flash duration
      timerRef.current = setTimeout(() => {
        setDirection('neutral');
      }, flashDuration);
    }

    // Update ref for next comparison
    prevPriceRef.current = currentPrice;

    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
      }
    };
  }, [price, flashDuration]);

  return direction;
}

/**
 * Returns Tailwind color classes based on price direction.
 * Include 'transition-colors duration-300' on the element for smooth fade back to neutral.
 */
export function getPriceColorClass(
  direction: PriceDirection,
  neutralColor = 'text-gray-300'
): string {
  switch (direction) {
    case 'up':
      return 'text-green-400';
    case 'down':
      return 'text-red-400';
    default:
      return neutralColor;
  }
}
