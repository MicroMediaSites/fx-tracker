import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useSettingsStore } from '../../stores/settingsStore';

interface OandaCredentials {
  environment: string;
  accountAlias?: string;
}

/**
 * Badge showing current environment (Mock/Demo/Live)
 * Fetches credentials to determine actual environment
 * Listens for environment-changed events to sync across windows
 */
export const EnvironmentBadge = () => {
  const dataSource = useSettingsStore((s) => s.dataSource);
  const [credentials, setCredentials] = useState<OandaCredentials | null>(null);

  // Fetch credentials initially and when dataSource changes
  useEffect(() => {
    invoke<OandaCredentials>('get_oanda_credentials')
      .then(setCredentials)
      .catch(() => setCredentials(null));
  }, [dataSource]);

  // Listen for environment changes from other windows
  useEffect(() => {
    const unlisten = listen('environment-changed', () => {
      invoke<OandaCredentials>('get_oanda_credentials')
        .then(setCredentials)
        .catch(() => setCredentials(null));
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const accountName = credentials?.accountAlias;

  if (credentials?.environment === 'live') {
    return (
      <span className="px-2 py-0.5 text-xs font-medium rounded bg-green-900/50 text-green-400 border border-green-600/30">
        {accountName ? `Live: ${accountName}` : 'Live'}
      </span>
    );
  }

  return (
    <span className="px-2 py-0.5 text-xs font-medium rounded bg-amber-900/50 text-amber-400 border border-amber-600/30">
      {accountName ? `Demo: ${accountName}` : 'Demo'}
    </span>
  );
}
