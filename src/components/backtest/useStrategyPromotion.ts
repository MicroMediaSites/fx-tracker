/**
 * useStrategyPromotion — promote/demote flow for the backtest window, writing
 * to the wickd local store (AGT-645; was `zero.mutate` before the migration).
 * Audit rows land in the local `promotion_audit` table (append-only).
 */
import { useState } from 'react';
import { emit } from '@tauri-apps/api/event';
import { Strategy } from '../../types/strategy';
import { PromotionAcknowledgements } from './types';
import { findDynamicParameters, resolveParams } from './strategyUtils';
import { recordPromotion, updateStrategy } from '../../lib/localStore';

interface UseStrategyPromotionProps {
  selectedStrategy: Strategy | undefined;
  refreshStrategies: () => Promise<void>;
}

export function useStrategyPromotion({
  selectedStrategy,
  refreshStrategies,
}: UseStrategyPromotionProps) {
  // Promotion confirmation modal state
  const [showPromoteModal, setShowPromoteModal] = useState(false);
  const [promotionAcknowledgements, setPromotionAcknowledgements] = useState<PromotionAcknowledgements>({
    ownLogic: false,
    independentChoices: false,
    noGuarantee: false,
    responsible: false,
  });

  // Dynamic parameter resolution modal state (shown before promotion if strategy has $param references)
  const [showParamResolutionModal, setShowParamResolutionModal] = useState(false);
  const [paramResolutionValues, setParamResolutionValues] = useState<Record<string, number>>({});

  // Handle promotion - show confirmation modal when promoting, direct action when demoting
  const handlePromoteClick = () => {
    if (!selectedStrategy) return;
    if (selectedStrategy.is_promoted) {
      // Demoting - no confirmation needed
      handleDemote();
    } else {
      // Check for dynamic parameters that need to be resolved
      const dynamicParams = findDynamicParameters(selectedStrategy);
      if (dynamicParams.length > 0) {
        // Initialize resolution values with defaults
        const initialValues: Record<string, number> = {};
        dynamicParams.forEach(p => {
          initialValues[p.id] = p.default;
        });
        setParamResolutionValues(initialValues);
        setShowParamResolutionModal(true);
      } else {
        // No dynamic params - proceed directly to promotion confirmation
        setPromotionAcknowledgements({
          ownLogic: false,
          independentChoices: false,
          noGuarantee: false,
          responsible: false,
        });
        setShowPromoteModal(true);
      }
    }
  };

  // Handle resolving dynamic parameters before promotion
  const handleResolveParameters = async () => {
    if (!selectedStrategy) return;

    // Update strategy with resolved values - include ALL fields that can contain $param refs
    await updateStrategy(selectedStrategy.id, {
      indicators: JSON.stringify(resolveParams(selectedStrategy.indicators, paramResolutionValues)),
      risk_settings: JSON.stringify(resolveParams(selectedStrategy.risk_settings, paramResolutionValues)),
      entry_rules: JSON.stringify(resolveParams(selectedStrategy.entry_rules, paramResolutionValues)),
      exit_rules: JSON.stringify(resolveParams(selectedStrategy.exit_rules, paramResolutionValues)),
      parameters: null, // Clear parameters since they're now resolved
    });
    await refreshStrategies();

    setShowParamResolutionModal(false);

    // Now proceed to promotion confirmation
    setPromotionAcknowledgements({
      ownLogic: false,
      independentChoices: false,
      noGuarantee: false,
      responsible: false,
    });
    setShowPromoteModal(true);
  };

  // Confirm promotion (after user acknowledgement)
  const handleConfirmPromote = async () => {
    if (!selectedStrategy) return;
    const now = Date.now();

    // Update strategy state
    await updateStrategy(selectedStrategy.id, {
      is_promoted: true,
      is_locked: true, // Lock strategy forever once promoted
    });

    // Create audit record for compliance
    await recordPromotion({
      id: crypto.randomUUID(),
      strategy_id: selectedStrategy.id,
      strategy_name: selectedStrategy.name,
      action: 'promote',
      created_at: now,
    });
    await refreshStrategies();

    // Emit event for AI terminal to display confirmation
    await emit('strategy-promoted', {
      strategy_id: selectedStrategy.id,
      strategy_name: selectedStrategy.name,
    });

    setShowPromoteModal(false);
  };

  // Deactivate strategy (remove from live trading)
  // Strategy stays locked - user can clone separately if they want to iterate
  const handleDemote = async () => {
    if (!selectedStrategy) return;
    const now = Date.now();

    // Update strategy state
    await updateStrategy(selectedStrategy.id, {
      is_promoted: false,
    });

    // Create audit record for compliance
    await recordPromotion({
      id: crypto.randomUUID(),
      strategy_id: selectedStrategy.id,
      strategy_name: selectedStrategy.name,
      action: 'demote',
      created_at: now,
    });
    await refreshStrategies();
  };

  const handleParamValueChange = (paramId: string, value: number) => {
    setParamResolutionValues(prev => ({
      ...prev,
      [paramId]: value,
    }));
  };

  const handleAcknowledgementChange = (key: keyof PromotionAcknowledgements, value: boolean) => {
    setPromotionAcknowledgements(prev => ({ ...prev, [key]: value }));
  };

  return {
    // Modal state
    showPromoteModal,
    setShowPromoteModal,
    promotionAcknowledgements,
    showParamResolutionModal,
    setShowParamResolutionModal,
    paramResolutionValues,

    // Handlers
    handlePromoteClick,
    handleResolveParameters,
    handleConfirmPromote,
    handleDemote,
    handleParamValueChange,
    handleAcknowledgementChange,
  };
}
