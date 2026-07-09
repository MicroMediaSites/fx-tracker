import { useState, useCallback } from 'react';

interface LabsSettings {
  scriptedStrategies: boolean;
}

const STORAGE_KEY = 'candlesight-labs';
const DEFAULT_SETTINGS: LabsSettings = { scriptedStrategies: false };

export function useLabsSettings() {
  const [settings, setSettings] = useState<LabsSettings>(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      return stored ? { ...DEFAULT_SETTINGS, ...JSON.parse(stored) } : DEFAULT_SETTINGS;
    } catch {
      return DEFAULT_SETTINGS;
    }
  });

  const toggle = useCallback((key: keyof LabsSettings) => {
    setSettings(prev => {
      const next = { ...prev, [key]: !prev[key] };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  return { settings, toggle };
}
