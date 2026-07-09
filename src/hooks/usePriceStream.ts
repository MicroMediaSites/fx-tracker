import { useEffect, useCallback, useRef } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { usePriceStore, PriceUpdate, StreamError } from '../stores/priceStore';

// Default instruments to stream
const DEFAULT_INSTRUMENTS = [
  'EUR_USD',
  'GBP_USD',
  'USD_JPY',
  'AUD_USD',
  'USD_CAD',
];

/**
 * Hook for subscribing to price updates for multiple instruments.
 *
 * Uses the centralized PriceStreamManager - subscribing to the same instrument
 * from multiple components will share a single stream to OANDA.
 *
 * @param instruments - Array of instruments to subscribe to
 * @returns Object with streaming state and control functions
 */
export const usePriceStream = (instruments: string[] = DEFAULT_INSTRUMENTS) => {
  const { updatePrice, setStreaming, setError, streaming } = usePriceStore();
  const subscribedRef = useRef<Set<string>>(new Set());
  const instrumentsRef = useRef<string[]>(instruments);

  // Keep ref in sync with prop
  instrumentsRef.current = instruments;

  const startStream = useCallback(async () => {
    if (streaming) return;

    try {
      setError(null);

      for (const instrument of instrumentsRef.current) {
        if (!subscribedRef.current.has(instrument)) {
          await invoke('subscribe_to_prices', { instrument });
          subscribedRef.current.add(instrument);
        }
      }

      setStreaming(true);
    } catch (err) {
      setError({
        errorType: 'connection_lost',
        message: err instanceof Error ? err.message : String(err),
      });
    }
  }, [streaming, setStreaming, setError]);

  const stopStream = useCallback(async () => {
    const allSubscribed = Array.from(subscribedRef.current);
    for (const instrument of allSubscribed) {
      try {
        await invoke('unsubscribe_from_prices', { instrument });
        subscribedRef.current.delete(instrument);
      } catch (err) {
        console.error('[usePriceStream] Failed to unsubscribe from:', instrument, err);
      }
    }
    setStreaming(false);
  }, [setStreaming]);

  // Set up event listeners
  useEffect(() => {
    let cancelled = false;
    let priceUnlisten: UnlistenFn | null = null;
    let errorUnlisten: UnlistenFn | null = null;

    const setup = async () => {
      // Listen for price updates
      const priceFn = await listen<PriceUpdate>('price-update', (event) => {
        if (cancelled) return;
        updatePrice(event.payload);
      });

      if (cancelled) {
        priceFn();
        return;
      }
      priceUnlisten = priceFn;

      // Listen for stream errors
      const errorFn = await listen<StreamError>('stream-error', (event) => {
        if (cancelled) return;
        setError(event.payload);
        setStreaming(false);
      });

      if (cancelled) {
        errorFn();
        return;
      }
      errorUnlisten = errorFn;
    };

    setup();

    return () => {
      cancelled = true;
      priceUnlisten?.();
      errorUnlisten?.();
    };
  }, [updatePrice, setError, setStreaming]);

  // Handle instrument changes - use a stable stringified version to prevent loops
  const instrumentsKey = [...instruments].sort().join(',');

  useEffect(() => {
    const currentInstruments = new Set(instruments);
    const subscribed = subscribedRef.current;

    // Subscribe to new instruments
    const subscribe = async () => {
      for (const instrument of instruments) {
        if (!subscribed.has(instrument)) {
          try {
            await invoke('subscribe_to_prices', { instrument });
            subscribedRef.current.add(instrument);
            setStreaming(true);
          } catch (err) {
            console.error('[usePriceStream] Failed to subscribe to:', instrument, err);
          }
        }
      }

      // Unsubscribe from removed instruments
      for (const instrument of Array.from(subscribed)) {
        if (!currentInstruments.has(instrument)) {
          try {
            await invoke('unsubscribe_from_prices', { instrument });
            subscribedRef.current.delete(instrument);
          } catch (err) {
            console.error('[usePriceStream] Failed to unsubscribe from:', instrument, err);
          }
        }
      }
    };

    subscribe();

    // Cleanup on unmount - unsubscribe from all
    return () => {
      const allSubscribed = Array.from(subscribedRef.current);
      for (const instrument of allSubscribed) {
        invoke('unsubscribe_from_prices', { instrument }).catch((err) => {
          console.error('[usePriceStream] Cleanup - failed to unsubscribe from:', instrument, err);
        });
      }
      subscribedRef.current.clear();
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instrumentsKey, setStreaming]);

  return {
    streaming,
    startStream,
    stopStream,
  };
}
