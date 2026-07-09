import { useState, useMemo, useEffect, useRef } from 'react';
import { Strategy, normalizeStrategy, StrategyV2 } from '../../types/strategy';
import { StrategyVersion } from './types';

export const useStrategyVersions = (currentStrategy?: Strategy) => {
  const [versions, setVersions] = useState<StrategyVersion[]>([]);
  const [activeVersionId, setActiveVersionId] = useState<string>('original');

  // Track when the database strategy updates (via Zero sync)
  // This keeps 'original' version in sync with database changes
  const lastStrategyJsonRef = useRef<string | null>(null);

  useEffect(() => {
    if (!currentStrategy) return;

    const strategyJson = JSON.stringify(currentStrategy);

    // Skip if strategy hasn't actually changed (avoid infinite loops)
    if (strategyJson === lastStrategyJsonRef.current) return;
    lastStrategyJsonRef.current = strategyJson;

    // Update the 'original' version to reflect database changes
    setVersions((prev) => {
      if (prev.length === 0) return prev; // Not initialized yet

      const originalVersion = prev.find(v => v.id === 'original');

      // Check if this is a significant change (e.g., parameters resolved during promotion)
      // If parameters changed in the database, reset to original to reflect the change
      const dbParamsJson = JSON.stringify(currentStrategy.parameters || []);
      const localParamsJson = JSON.stringify(originalVersion?.strategy.parameters || []);
      const paramsChanged = dbParamsJson !== localParamsJson;

      if (paramsChanged) {
        // Significant database change - reset to just original version
        // This handles promotion, parameter resolution, etc.
        setActiveVersionId('original');
        return [{
          id: 'original',
          label: 'Original',
          strategy: normalizeStrategy(JSON.parse(strategyJson) as StrategyV2),
          runs: originalVersion?.runs || [],
        }];
      }

      return prev.map((v) => {
        if (v.id === 'original') {
          return {
            ...v,
            strategy: normalizeStrategy(JSON.parse(strategyJson) as StrategyV2),
          };
        }
        return v;
      });
    });
  }, [currentStrategy]);

  // Get the current active version
  const activeVersion = versions.find((v) => v.id === activeVersionId);
  const workingCopy = activeVersion?.strategy || null;

  // Check if current version is modified from original
  const originalVersion = versions.find((v) => v.id === 'original');
  const workingCopyModified = useMemo(() => {
    if (!workingCopy || !originalVersion) return false;
    return JSON.stringify({
      name: workingCopy.name,
      description: workingCopy.description,
      parameters: workingCopy.parameters,
      indicators: workingCopy.indicators,
      variables: workingCopy.variables,
      entry_rules: workingCopy.entry_rules,
      entry_logic: workingCopy.entry_logic,
      exit_rules: workingCopy.exit_rules,
      risk_settings: workingCopy.risk_settings,
    }) !== JSON.stringify({
      name: originalVersion.strategy.name,
      description: originalVersion.strategy.description,
      parameters: originalVersion.strategy.parameters,
      indicators: originalVersion.strategy.indicators,
      variables: originalVersion.strategy.variables,
      entry_rules: originalVersion.strategy.entry_rules,
      entry_logic: originalVersion.strategy.entry_logic,
      exit_rules: originalVersion.strategy.exit_rules,
      risk_settings: originalVersion.strategy.risk_settings,
    });
  }, [workingCopy, originalVersion]);

  // Initialize versions when strategy is selected
  const initializeVersions = (strategy: Strategy): StrategyVersion[] => {
    const newVersions: StrategyVersion[] = [{
      id: 'original',
      label: 'Original',
      strategy: JSON.parse(JSON.stringify(strategy)),
      runs: [],
    }];
    setVersions(newVersions);
    setActiveVersionId('original');
    return newVersions;
  };

  // Create a new version from the current working copy
  const createNewVersion = (updatedStrategy: Strategy): string => {
    const versionNum = versions.filter((v) => v.id !== 'original').length + 1;
    const newVersionId = `v${versionNum}`;
    const newVersion: StrategyVersion = {
      id: newVersionId,
      label: `v${versionNum}`,
      strategy: JSON.parse(JSON.stringify(updatedStrategy)),
      runs: [],
    };
    setVersions((prev) => [...prev, newVersion]);
    setActiveVersionId(newVersionId);
    return newVersionId;
  };

  // Update the active version's strategy
  // If on 'original', create a new version to preserve the pristine original
  const updateActiveVersionStrategy = (updatedStrategy: Strategy) => {
    if (activeVersionId === 'original') {
      // Don't modify original - create a new version instead
      createNewVersion(updatedStrategy);
      return;
    }

    if (activeVersionId) {
      setVersions((prev) =>
        prev.map((v) =>
          v.id === activeVersionId ? { ...v, strategy: updatedStrategy } : v
        )
      );
    }
  };

  return {
    versions,
    setVersions,
    activeVersionId,
    setActiveVersionId,
    activeVersion,
    workingCopy,
    workingCopyModified,
    originalVersion,
    initializeVersions,
    createNewVersion,
    updateActiveVersionStrategy,
  };
};
