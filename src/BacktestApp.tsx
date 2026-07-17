import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEnvironmentSync } from './hooks/useEnvironmentSync';
import { Strategy, BacktestMethodology } from './types/strategy';
import { SimpleHistoricalFlow } from './components/backtest/SimpleHistoricalFlow';
import { WalkForwardFlow } from './components/backtest/WalkForwardFlow';
import { WindowHeader } from './components/ui/WindowHeader';
import { NotesModal } from './components/ui/NotesModal';
import { StrategyErrorRecovery } from './components/ui/StrategyErrorRecovery';
import { StrategyVersion } from './components/backtest/types';
import { isStoreStrategy } from './lib/strategyStore';
import {
  StrategyListPanel,
  StrategyHeaderBar,
  SourceViewerModal,
  ParameterResolutionModal,
  PromotionConfirmationModal,
  useStrategyVersions,
} from './components/backtest';
import type { SingleTestingParams, RangeTestingParams, UseDefaultParams } from './components/backtest/TestableParametersPanel';
import type { TestZone } from './components/backtest/TestZonesPanel';
import { useParsedStrategies } from './components/backtest/useParsedStrategies';
import { useStrategyMutations } from './components/backtest/useStrategyMutations';
import { useStrategyPromotion } from './components/backtest/useStrategyPromotion';
import { FirstRunTour } from './components/onboarding/FirstRunTour';
import { backtestTourSteps } from './lib/tourSteps';
import type { WalkForwardResult, WalkForwardPeriod } from './types/strategy';

/** Convert a Strategy object to JSON string for backend validation */
function strategyToJson(strategy: Strategy): string {
  return JSON.stringify({
    // Metadata fields required by StrategyDefinition
    id: strategy.id,
    user_id: strategy.user_id,
    version: strategy.version ?? 1,
    is_active: strategy.is_active ?? true,
    is_promoted: strategy.is_promoted ?? false,
    is_locked: strategy.is_locked ?? false,
    is_archived: strategy.is_archived ?? false,
    created_at: strategy.created_at ?? Date.now(),
    updated_at: strategy.updated_at ?? Date.now(),
    // Strategy definition fields
    schema_version: strategy.schema_version ?? 2,
    name: strategy.name,
    description: strategy.description,
    indicators: strategy.indicators || [],
    parameters: strategy.parameters || [],
    variables: strategy.variables || [],
    entry_rules: strategy.entry_rules || [],
    entry_logic: strategy.entry_logic || { mode: 'all' },
    exit_rules: strategy.exit_rules || [],
    risk_settings: strategy.risk_settings,
    pivot_config: strategy.pivot_config,
    strategy_type: strategy.strategy_type || 'rules',
    script_content: strategy.script_content,
  });
}

export const BacktestApp = () => {
  // BUG-024: Sync dataSource across windows when user switches accounts.
  // Backtesting uses market data (same across accounts) but the Zustand store
  // needs to stay in sync for the environment badge.
  useEnvironmentSync();

  // Settings modal state
  const [settingsOpen, setSettingsOpen] = useState(false);

  // Notes modal state
  const [notesModalOpen, setNotesModalOpen] = useState(false);

  // Read-only .rhai source viewer state (AGT-651 — the app is a
  // viewer/runner; the visual builder and its editing paths are gone).
  const [sourceViewerOpen, setSourceViewerOpen] = useState(false);

  // Strategy state
  const [selectedStrategyId, setSelectedStrategyId] = useState<string | null>(null);
  const [strategyListExpanded, setStrategyListExpanded] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  // Pending promotion from AI terminal - triggers promotion modal after strategy is selected
  const [pendingPromotionStrategyId, setPendingPromotionStrategyId] = useState<string | null>(null);

  // Parsed strategies hook (must come before useStrategyVersions)
  // Served from the wickd local store (AGT-645); refreshStrategies re-reads it
  // after every mutation since the store has no Zero-style reactivity.
  const { parsedStrategies, refreshStrategies } = useParsedStrategies({
    showArchived,
  });

  const selectedStrategy = parsedStrategies.find((s) => s.id === selectedStrategyId);

  // Version history hook - syncs with selectedStrategy from Zero
  const {
    versions,
    activeVersionId,
    activeVersion,
    workingCopy,
    workingCopyModified,
    initializeVersions,
    createNewVersion,
    updateActiveVersionStrategy,
  } = useStrategyVersions(selectedStrategy);

  // Backtest config state
  const [initialBalance] = useState(1000);
  const [methodology, setMethodology] = useState<BacktestMethodology | null>(null);

  // Result state (list-level errors surfaced by the panel)
  const [error] = useState<string | null>(null);

  // Strategy validation state (checked on select)
  const [strategyValidationError, setStrategyValidationError] = useState<string | null>(null);
  const [strategyValidating, setStrategyValidating] = useState(false);
  const validationRequestIdRef = useRef(0); // Track current validation to ignore stale responses

  // Testing parameter state (lifted from flow components)
  const [testingValues, setTestingValues] = useState<SingleTestingParams>({});
  const [rangeValues, setRangeValues] = useState<RangeTestingParams>({});
  const [useDefaultParams, setUseDefaultParams] = useState<UseDefaultParams>({});
  // Test zones for backtesting (separate from chart zones to avoid look-ahead bias)
  const [testZones, setTestZones] = useState<TestZone[]>([]);
  // Holdout results summary (children still report it; consumer removed with the chat terminal)
  const [, setHoldoutSummary] = useState<string | null>(null);
  // Current backtest job info (children still report it; consumer removed with the chat terminal)
  const [, setCurrentJobInfo] = useState<{
    jobId: string;
    hasResults: boolean;
    metricsSummary?: string;
  } | null>(null);
  // Walk-forward context (children still report it; consumer removed with the chat terminal)
  const [, setWfContext] = useState<{
    wfResult: WalkForwardResult | null;
    selectedWindow: WalkForwardPeriod | null;
  }>({ wfResult: null, selectedWindow: null });

  // Initialize versions when strategy is selected + validate against backend
  // BUG-022: Wrapped in useCallback so it can be safely included in effect dependency arrays.
  const selectStrategy = useCallback(async (id: string) => {
    // Increment request ID to invalidate any in-flight validation
    const requestId = ++validationRequestIdRef.current;

    setSelectedStrategyId(id);
    setStrategyValidationError(null);
    setStrategyValidating(true);

    const strategy = parsedStrategies.find((s) => s.id === id);
    if (strategy) {
      initializeVersions(strategy);
      setMethodology(null);
      setTestingValues({});
      setRangeValues({});
      setUseDefaultParams({});
      setTestZones([]);

      // Validate strategy against backend StrategyDefinition
      try {
        await invoke('validate_strategy_json', { strategyJson: strategyToJson(strategy) });
        // Only apply result if this is still the current request
        if (requestId !== validationRequestIdRef.current) return;
      } catch (err) {
        // Only apply error if this is still the current request
        if (requestId !== validationRequestIdRef.current) return;
        const errorMsg = err instanceof Error ? err.message : String(err);
        setStrategyValidationError(errorMsg);
      }
    }
    // Only update loading state if this is still the current request
    if (requestId === validationRequestIdRef.current) {
      setStrategyValidating(false);
    }
  }, [parsedStrategies, initializeVersions]);

  // Handle deep-link: read strategy query param and listen for select-strategy event
  const initialStrategyHandled = useRef(false);

  // Use refs for event handler to avoid stale closure issues
  const parsedStrategiesRef = useRef(parsedStrategies);
  const selectStrategyRef = useRef(selectStrategy);
  useEffect(() => {
    parsedStrategiesRef.current = parsedStrategies;
    selectStrategyRef.current = selectStrategy;
  }, [parsedStrategies, selectStrategy]);

  // Handle initial query param (separate effect, runs once strategies load)
  useEffect(() => {
    if (!initialStrategyHandled.current && parsedStrategies.length > 0) {
      const params = new URLSearchParams(window.location.search);
      const strategyId = params.get('strategy');
      if (strategyId && parsedStrategies.some(s => s.id === strategyId)) {
        initialStrategyHandled.current = true;
        selectStrategy(strategyId);
      }
    }
  }, [parsedStrategies, selectStrategy]);

  // Listen for select-strategy event (stable listener, uses refs)
  useEffect(() => {
    const unlisten = listen<string>('select-strategy', (event) => {
      const strategyId = event.payload;
      // Use refs to get latest values without recreating listener
      if (parsedStrategiesRef.current.some(s => s.id === strategyId)) {
        selectStrategyRef.current(strategyId);
      }
    });

    return () => {
      unlisten.then(fn => fn());
    };
  }, []); // Empty deps - listener stays stable, uses refs for current values

  // Listen for open-promotion-modal event from AI terminal
  useEffect(() => {
    const unlisten = listen<{ strategy_id: string; strategy_name: string }>(
      'open-promotion-modal',
      (event) => {
        const { strategy_id } = event.payload;
        // Find and select the strategy, mark as pending promotion
        const strategy = parsedStrategiesRef.current.find(s => s.id === strategy_id);
        if (strategy) {
          selectStrategyRef.current(strategy_id);
          // Set pending promotion - effect below will trigger modal once selection updates
          setPendingPromotionStrategyId(strategy_id);
        }
      }
    );

    return () => {
      unlisten.then(fn => fn());
    };
  }, []); // Empty deps - uses refs and setState

  // Initialize testing values from strategy defaults when strategy changes
  const initializeTestingValues = useCallback((strategy: Strategy) => {
    const singleVals: SingleTestingParams = {};
    const rangeVals: RangeTestingParams = {};
    for (const param of strategy.parameters || []) {
      singleVals[param.id] = param.default;
      rangeVals[param.id] = {
        min: param.min ?? Math.floor(param.default * 0.5),
        max: param.max ?? Math.ceil(param.default * 1.5),
        step: param.step ?? Math.max(1, Math.floor(param.default * 0.1)),
      };
    }
    setTestingValues(singleVals);
    setRangeValues(rangeVals);
  }, []);

  // Re-initialize testing values when workingCopy parameters change
  useEffect(() => {
    if (workingCopy && methodology) {
      const currentParams = workingCopy.parameters || [];
      const needsInit = currentParams.some(p => !(p.id in testingValues));
      if (needsInit || Object.keys(testingValues).length === 0) {
        initializeTestingValues(workingCopy);
      }
    }
  }, [workingCopy, methodology, testingValues, initializeTestingValues]);

  // Testing value callbacks
  const handleTestingValueChange = useCallback((paramId: string, value: number) => {
    setTestingValues(prev => ({ ...prev, [paramId]: value }));
  }, []);

  const handleRangeValueChange = useCallback((paramId: string, field: 'min' | 'max' | 'step', value: number) => {
    setRangeValues(prev => ({
      ...prev,
      [paramId]: { ...prev[paramId], [field]: value },
    }));
  }, []);

  const handleResetTestingValues = useCallback(() => {
    if (workingCopy) {
      initializeTestingValues(workingCopy);
      setUseDefaultParams({});
    }
  }, [workingCopy, initializeTestingValues]);

  // Handle "use default" toggle for a parameter (skip in optimization)
  const handleUseDefaultChange = useCallback((paramId: string, useDefault: boolean) => {
    setUseDefaultParams(prev => ({ ...prev, [paramId]: useDefault }));
  }, []);

  // Handle resolving a parameter - replace all $param refs with value and remove parameter
  const handleResolveParameter = useCallback((paramId: string, value: number) => {
    if (!workingCopy) return;

    // Deep clone the strategy to modify it
    const strategyJson = JSON.stringify(workingCopy);

    // Replace all { "$param": "paramId" } with the fixed value
    const paramRefRegex = new RegExp(`\\{\\s*"\\$param"\\s*:\\s*"${paramId}"\\s*\\}`, 'g');
    const updatedJson = strategyJson.replace(paramRefRegex, String(value));

    // Parse back and update parameters list
    const updatedStrategy = JSON.parse(updatedJson) as Strategy;
    updatedStrategy.parameters = (updatedStrategy.parameters || []).filter(p => p.id !== paramId);

    // Update the strategy
    updateActiveVersionStrategy(updatedStrategy);

    // Clean up local state for this param
    setTestingValues(prev => {
      const next = { ...prev };
      delete next[paramId];
      return next;
    });
    setRangeValues(prev => {
      const next = { ...prev };
      delete next[paramId];
      return next;
    });
    setUseDefaultParams(prev => {
      const next = { ...prev };
      delete next[paramId];
      return next;
    });

    console.log('[Resolve] Done');
  }, [workingCopy, updateActiveVersionStrategy]);

  // Check if testing values have been modified from defaults
  const testingValuesModified = useMemo(() => {
    if (!workingCopy) return false;
    for (const param of workingCopy.parameters || []) {
      if (testingValues[param.id] !== param.default) return true;
    }
    return false;
  }, [workingCopy, testingValues]);

  const rangeValuesModified = useMemo(() => {
    if (!workingCopy) return false;
    for (const param of workingCopy.parameters || []) {
      const range = rangeValues[param.id];
      if (!range) continue;
      const defaultMin = param.min ?? Math.floor(param.default * 0.5);
      const defaultMax = param.max ?? Math.ceil(param.default * 1.5);
      const defaultStep = param.step ?? Math.max(1, Math.floor(param.default * 0.1));
      if (range.min !== defaultMin || range.max !== defaultMax || range.step !== defaultStep) {
        return true;
      }
    }
    return false;
  }, [workingCopy, rangeValues]);

  // Save testing values to strategy (for Walk-Forward ranges)
  const handleSaveTestingValuesToStrategy = useCallback(() => {
    if (!workingCopy) return;
    const isWalkForward = methodology === 'walk_forward';

    if (isWalkForward) {
      const updatedParams = (workingCopy.parameters || []).map(p => ({
        ...p,
        min: rangeValues[p.id]?.min ?? p.min,
        max: rangeValues[p.id]?.max ?? p.max,
        step: rangeValues[p.id]?.step ?? p.step,
      }));
      updateActiveVersionStrategy({ ...workingCopy, parameters: updatedParams });
    } else {
      const updatedParams = (workingCopy.parameters || []).map(p => ({
        ...p,
        default: testingValues[p.id] ?? p.default,
      }));
      updateActiveVersionStrategy({ ...workingCopy, parameters: updatedParams });
    }
  }, [workingCopy, methodology, rangeValues, testingValues, updateActiveVersionStrategy]);

  // Auto-select first strategy if none selected
  useEffect(() => {
    if (!selectedStrategyId && parsedStrategies && parsedStrategies.length > 0) {
      selectStrategy(parsedStrategies[0].id);
    }
  }, [parsedStrategies, selectedStrategyId, selectStrategy]);

  // Strategy mutations hook (viewer/runner list management only — the
  // builder's create/edit/clone/save mutations were deleted with it, AGT-651)
  const { handleToggleArchive } = useStrategyMutations({
    refreshStrategies,
    selectedStrategy,
  });

  // Promotion hook
  const {
    showPromoteModal,
    setShowPromoteModal,
    promotionAcknowledgements,
    showParamResolutionModal,
    setShowParamResolutionModal,
    paramResolutionValues,
    handlePromoteClick,
    handleResolveParameters,
    handleConfirmPromote,
    handleParamValueChange,
    handleAcknowledgementChange,
  } = useStrategyPromotion({
    selectedStrategy,
    refreshStrategies,
  });

  // Trigger promotion modal when pending promotion matches selected strategy
  // This handles AI-initiated promotions after the strategy selection state updates
  useEffect(() => {
    if (pendingPromotionStrategyId && selectedStrategy?.id === pendingPromotionStrategyId) {
      // Clear pending state and trigger promotion flow
      setPendingPromotionStrategyId(null);
      handlePromoteClick();
    }
  }, [pendingPromotionStrategyId, selectedStrategy?.id, handlePromoteClick]);

  // Handle archive toggle with deselection logic
  const handleToggleArchiveWithDeselect = async () => {
    await handleToggleArchive();
    // If archiving and not showing archived, deselect
    if (selectedStrategy && !selectedStrategy.is_archived && !showArchived) {
      setSelectedStrategyId(null);
    }
  };

  // Reset versions for a strategy (used when methodology changes)
  const resetVersions = (strategy: Strategy): StrategyVersion[] => {
    return initializeVersions(strategy);
  };

  return (
    <div className="min-h-screen bg-[var(--color-bg-page)] text-[var(--color-text-primary)]">
      <WindowHeader
        title="Strategy Development"
        currentWindow="backtesting"
        settingsOpen={settingsOpen}
        onSettingsChange={setSettingsOpen}
      />

      <FirstRunTour windowType="backtest" steps={backtestTourSteps} />

      {/* Main content */}
      <main className="max-w-7xl mx-auto px-4 py-6">
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          {/* Strategy List Panel */}
          <div data-tour="strategy-list">
          <StrategyListPanel
            strategies={parsedStrategies}
            selectedStrategy={selectedStrategy}
            workingCopy={workingCopy}
            selectedStrategyId={selectedStrategyId}
            strategyListExpanded={strategyListExpanded}
            showArchived={showArchived}
            methodology={methodology}
            error={error}
            onSelectStrategy={selectStrategy}
            onToggleExpanded={() => setStrategyListExpanded(!strategyListExpanded)}
            onShowArchivedChange={setShowArchived}
            onMethodologyChange={setMethodology}
            onToggleArchive={handleToggleArchiveWithDeselect}
            onResetVersions={resetVersions}
            testingValues={testingValues}
            rangeValues={rangeValues}
            useDefaultParams={useDefaultParams}
            onTestingValueChange={handleTestingValueChange}
            onRangeValueChange={handleRangeValueChange}
            onUseDefaultChange={handleUseDefaultChange}
            onResolveParameter={handleResolveParameter}
            onResetTestingValues={handleResetTestingValues}
            onSaveTestingValuesToStrategy={handleSaveTestingValuesToStrategy}
            testingValuesModified={testingValuesModified}
            rangeValuesModified={rangeValuesModified}
            testZones={testZones}
            onTestZonesChange={setTestZones}
          />
          </div>

          {/* Results Panel */}
          <div className="lg:col-span-2 space-y-4" data-tour="backtest-results">
            {/* Strategy Header - thin bar with strategy name and actions */}
            {selectedStrategy && versions.length > 0 && (
              <StrategyHeaderBar
                selectedStrategy={selectedStrategy}
                workingCopyModified={workingCopyModified}
                activeVersionId={activeVersionId}
                activeVersion={activeVersion}
                // Store entries are managed by the wickd CLI; promotion only
                // applies to local-store rows (viewer/runner split, AGT-651).
                onPromoteClick={isStoreStrategy(selectedStrategy.id) ? undefined : handlePromoteClick}
                onViewSource={
                  selectedStrategy.strategy_type === 'scripted' && selectedStrategy.script_content
                    ? () => setSourceViewerOpen(true)
                    : undefined
                }
                onNotesClick={() => setNotesModalOpen(true)}
              />
            )}

            {/* Methodology Flow Rendering */}
            {selectedStrategy && methodology === 'simple' && !strategyValidationError && (
              <SimpleHistoricalFlow
                strategy={workingCopy || selectedStrategy}
                initialBalance={initialBalance}
                testingValues={testingValues}
                testZones={testZones}
                onStrategyFix={(correctedJson) => {
                  try {
                    const correctedStrategy = JSON.parse(correctedJson) as Strategy;
                    updateActiveVersionStrategy(correctedStrategy);
                  } catch (e) {
                    console.error('Failed to parse corrected strategy:', e);
                  }
                }}
                onStrategyFixAsCopy={(correctedJson) => {
                  try {
                    const correctedStrategy = JSON.parse(correctedJson) as Strategy;
                    createNewVersion(correctedStrategy);
                  } catch (e) {
                    console.error('Failed to parse corrected strategy:', e);
                  }
                }}
              />
            )}

            {selectedStrategy && methodology === 'walk_forward' && !strategyValidationError && (
              <WalkForwardFlow
                strategy={workingCopy || selectedStrategy}
                initialBalance={initialBalance}
                rangeValues={rangeValues}
                useDefaultParams={useDefaultParams}
                testZones={testZones}
                onHoldoutResultsChange={setHoldoutSummary}
                onJobInfoChange={setCurrentJobInfo}
                onWfContextChange={setWfContext}
                onStrategyFix={(correctedJson) => {
                  console.log('[BacktestApp] onStrategyFix called');
                  try {
                    const correctedStrategy = JSON.parse(correctedJson) as Strategy;
                    console.log('[BacktestApp] Parsed strategy:', correctedStrategy.name);
                    updateActiveVersionStrategy(correctedStrategy);
                    console.log('[BacktestApp] updateActiveVersionStrategy complete');
                  } catch (e) {
                    console.error('Failed to parse corrected strategy:', e);
                  }
                }}
                onStrategyFixAsCopy={(correctedJson) => {
                  console.log('[BacktestApp] onStrategyFixAsCopy called');
                  try {
                    const correctedStrategy = JSON.parse(correctedJson) as Strategy;
                    console.log('[BacktestApp] Parsed strategy:', correctedStrategy.name);
                    const newVersionId = createNewVersion(correctedStrategy);
                    console.log('[BacktestApp] Created new version:', newVersionId);
                  } catch (e) {
                    console.error('Failed to parse corrected strategy:', e);
                  }
                }}
              />
            )}

            {/* Strategy validation error - show recovery UI */}
            {selectedStrategy && strategyValidationError && (
              <StrategyErrorRecovery
                error={strategyValidationError}
                strategyJson={strategyToJson(selectedStrategy)}
                onApplyFix={(correctedJson) => {
                  try {
                    const correctedStrategy = JSON.parse(correctedJson) as Strategy;
                    updateActiveVersionStrategy(correctedStrategy);
                    setStrategyValidationError(null);
                  } catch (e) {
                    console.error('Failed to parse corrected strategy:', e);
                  }
                }}
                onApplyFixAsCopy={(correctedJson) => {
                  try {
                    const correctedStrategy = JSON.parse(correctedJson) as Strategy;
                    createNewVersion(correctedStrategy);
                    setStrategyValidationError(null);
                  } catch (e) {
                    console.error('Failed to parse corrected strategy:', e);
                  }
                }}
                onDismiss={() => setStrategyValidationError(null)}
              />
            )}

            {/* Validating strategy */}
            {selectedStrategy && strategyValidating && (
              <div className="bg-[var(--color-bg-elevated)] rounded-lg border border-[var(--color-border)] p-8 text-center">
                <div className="animate-spin h-8 w-8 border-2 border-[var(--color-info)] border-t-transparent rounded-full mx-auto mb-4" />
                <p className="text-[var(--color-text-muted)]">Validating strategy...</p>
              </div>
            )}

            {/* Empty state when no methodology (and no validation error) */}
            {selectedStrategy && !methodology && !strategyValidationError && !strategyValidating && (
              <div className="p-8 text-center">
                <svg className="h-12 w-12 text-[var(--color-text-muted)]/50 mx-auto mb-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
                </svg>
                <p className="text-[var(--color-text-muted)]">
                  Select a backtest methodology from the left panel to begin.
                </p>
              </div>
            )}

            {!selectedStrategy && (
              <div className="p-8 text-center">
                <p className="text-[var(--color-text-muted)]">
                  Select a strategy from the list to run a backtest.
                </p>
              </div>
            )}
          </div>
        </div>
      </main>

      {/* Dynamic Parameter Resolution Modal */}
      {showParamResolutionModal && selectedStrategy && (
        <ParameterResolutionModal
          strategy={selectedStrategy}
          paramResolutionValues={paramResolutionValues}
          onParamValueChange={handleParamValueChange}
          onResolve={handleResolveParameters}
          onCancel={() => setShowParamResolutionModal(false)}
        />
      )}

      {/* Promotion Confirmation Modal */}
      {showPromoteModal && selectedStrategy && (
        <PromotionConfirmationModal
          strategy={selectedStrategy}
          acknowledgements={promotionAcknowledgements}
          onAcknowledgementChange={handleAcknowledgementChange}
          onConfirm={handleConfirmPromote}
          onCancel={() => setShowPromoteModal(false)}
        />
      )}

      {/* Strategy Notes Modal */}
      {selectedStrategy && (
        <NotesModal
          isOpen={notesModalOpen}
          onClose={() => setNotesModalOpen(false)}
          entityType="strategy"
          entityId={selectedStrategy.id}
          title="Strategy Journal"
          subtitle={selectedStrategy.name}
        />
      )}

      {/* Read-only .rhai source viewer (AGT-651) */}
      {selectedStrategy?.script_content && (
        <SourceViewerModal
          isOpen={sourceViewerOpen}
          onClose={() => setSourceViewerOpen(false)}
          strategyName={selectedStrategy.name}
          script={selectedStrategy.script_content}
        />
      )}

    </div>
  );
};
