/**
 * useStrategyMutations — strategy list management for the backtest window.
 *
 * AGT-651 deleted the visual builder and every strategy create/edit/clone/
 * save path with it (the app is a viewer/runner; authoring happens through
 * the wickd CLI against the unified `.rhai` store). What remains is list
 * management over local-store rows: archive/unarchive. File-store entries
 * (`rhai:` ids) are read-only and never reach these mutations — callers
 * hide the affordances for them.
 */
import { Strategy } from '../../types/strategy';
import { updateStrategy } from '../../lib/localStore';
import { isStoreStrategy } from '../../lib/strategyStore';

interface UseStrategyMutationsProps {
  refreshStrategies: () => Promise<void>;
  selectedStrategy: Strategy | undefined;
}

export function useStrategyMutations({
  refreshStrategies,
  selectedStrategy,
}: UseStrategyMutationsProps) {
  const handleToggleArchive = async () => {
    if (!selectedStrategy || isStoreStrategy(selectedStrategy.id)) return;
    await updateStrategy(selectedStrategy.id, {
      is_archived: !selectedStrategy.is_archived,
    });
    await refreshStrategies();
  };

  return { handleToggleArchive };
}
