/**
 * Price calculation utilities for FX trading
 *
 * Provides functions for calculating stop loss, take profit, and other
 * price-related values based on different input modes (pips, price, %).
 */

export type RiskInputMode = 'pips' | 'price' | '%';
export type TradeDirection = 'buy' | 'sell';

export interface PriceCalcParams {
  /** The input value (pips, price, or percentage) */
  value: string;
  /** The input mode */
  mode: RiskInputMode;
  /** Current market price */
  currentPrice: number;
  /** Trade direction */
  direction: TradeDirection;
  /** Whether this is a JPY pair (affects pip calculation) */
  isJpy: boolean;
}

/**
 * Gets the pip multiplier for a currency pair
 */
export function getPipMultiplier(isJpy: boolean): number {
  return isJpy ? 100 : 10000;
}

/**
 * Gets the decimal places for price display
 */
export function getDecimals(isJpy: boolean): number {
  return isJpy ? 3 : 5;
}

/**
 * Calculates stop loss price based on input mode
 *
 * For buy orders: SL is placed BELOW current price
 * For sell orders: SL is placed ABOVE current price
 */
export function calculateStopLoss(params: PriceCalcParams): string | null {
  const { value, mode, currentPrice, direction, isJpy } = params;

  if (!currentPrice || !value) return null;

  const val = parseFloat(value);
  if (isNaN(val)) return null;

  const decimals = getDecimals(isJpy);
  const pipMultiplier = getPipMultiplier(isJpy);

  switch (mode) {
    case 'price':
      return val.toFixed(decimals);

    case '%': {
      const distance = currentPrice * (val / 100);
      return direction === 'buy'
        ? (currentPrice - distance).toFixed(decimals)
        : (currentPrice + distance).toFixed(decimals);
    }

    case 'pips':
    default: {
      const pips = val / pipMultiplier;
      return direction === 'buy'
        ? (currentPrice - pips).toFixed(decimals)
        : (currentPrice + pips).toFixed(decimals);
    }
  }
}

/**
 * Calculates take profit price based on input mode
 *
 * For buy orders: TP is placed ABOVE current price
 * For sell orders: TP is placed BELOW current price
 */
export function calculateTakeProfit(params: PriceCalcParams): string | null {
  const { value, mode, currentPrice, direction, isJpy } = params;

  if (!currentPrice || !value) return null;

  const val = parseFloat(value);
  if (isNaN(val)) return null;

  const decimals = getDecimals(isJpy);
  const pipMultiplier = getPipMultiplier(isJpy);

  switch (mode) {
    case 'price':
      return val.toFixed(decimals);

    case '%': {
      const distance = currentPrice * (val / 100);
      return direction === 'buy'
        ? (currentPrice + distance).toFixed(decimals)
        : (currentPrice - distance).toFixed(decimals);
    }

    case 'pips':
    default: {
      const pips = val / pipMultiplier;
      return direction === 'buy'
        ? (currentPrice + pips).toFixed(decimals)
        : (currentPrice - pips).toFixed(decimals);
    }
  }
}

/**
 * Converts pips to price distance for a given pair
 */
export function pipsToPrice(pips: number, isJpy: boolean): number {
  return pips / getPipMultiplier(isJpy);
}

/**
 * Converts price distance to pips for a given pair
 */
export function priceToPips(priceDistance: number, isJpy: boolean): number {
  return priceDistance * getPipMultiplier(isJpy);
}

/**
 * Price parts for FX display formatting
 */
export interface PriceParts {
  /** Big figure (e.g., "1.09" or "109.") */
  top: string;
  /** Pips - the emphasized digits (e.g., "23") */
  big: string;
  /** Pipette - fractional pip (e.g., "4") */
  small: string;
}

/**
 * Splits a price into display parts for the standard FX format:
 * - top: big figure (e.g., "1.09" or "109.")
 * - big: pips (e.g., "23")
 * - small: pipette (e.g., "4")
 *
 * @param price - The numeric price (or string that parses to number)
 * @param isJpy - Whether this is a JPY pair (3 decimals vs 5)
 */
export function formatPriceParts(price: number | string, isJpy: boolean): PriceParts {
  const priceNum = typeof price === 'string' ? parseFloat(price) : price;

  if (!priceNum || isNaN(priceNum)) {
    return { top: '—', big: '——', small: '—' };
  }

  const priceStr = priceNum.toFixed(isJpy ? 3 : 5);

  if (isJpy) {
    // JPY: 109.973 -> top: "109.", big: "97", small: "3"
    const [whole, decimal] = priceStr.split('.');
    return {
      top: whole + '.',
      big: decimal.slice(0, 2),
      small: decimal.slice(2),
    };
  } else {
    // Standard: 1.09973 -> top: "1.09", big: "97", small: "3"
    const [whole, decimal] = priceStr.split('.');
    return {
      top: whole + '.' + decimal.slice(0, 2),
      big: decimal.slice(2, 4),
      small: decimal.slice(4),
    };
  }
}

/**
 * Formats spread value in pips for display
 */
export function formatSpreadPips(spread: number | string, isJpy: boolean): string {
  const spreadNum = typeof spread === 'string' ? parseFloat(spread) : spread;
  const pipMultiplier = getPipMultiplier(isJpy);
  return (spreadNum * pipMultiplier).toFixed(1);
}
