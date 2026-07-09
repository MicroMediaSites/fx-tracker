import { Strategy } from '../../types/strategy';
import { DynamicParameter } from './types';
import { findDynamicParameters } from './strategyUtils';

interface ParameterResolutionModalProps {
  strategy: Strategy;
  paramResolutionValues: Record<string, number>;
  onParamValueChange: (paramId: string, value: number) => void;
  onResolve: () => void;
  onCancel: () => void;
}

export const ParameterResolutionModal = ({
  strategy,
  paramResolutionValues,
  onParamValueChange,
  onResolve,
  onCancel,
}: ParameterResolutionModalProps) => {
  const dynamicParams: DynamicParameter[] = findDynamicParameters(strategy);

  return (
    <div
      className="fixed inset-0 z-[150] flex items-center justify-center"
      onClick={onCancel}
    >
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-[var(--color-bg-elevated)] rounded-lg shadow-xl max-w-lg w-full mx-4 p-6"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-3 mb-4">
          <div className="p-2 bg-[var(--color-warning)]/20 rounded-full">
            <svg className="w-6 h-6 text-[var(--color-warning)]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
            </svg>
          </div>
          <div>
            <h2 className="text-lg font-semibold text-[var(--color-text-primary)]">Resolve Dynamic Parameters</h2>
            <p className="text-sm text-[var(--color-text-muted)]">Select fixed values before going live</p>
          </div>
        </div>

        <p className="text-sm text-[var(--color-text-secondary)] mb-4">
          This strategy has dynamic parameters configured for testing. Before promoting to live trading,
          please select the values you want to use:
        </p>

        <div className="space-y-4 mb-6 max-h-[40vh] overflow-y-auto pr-2">
          {dynamicParams.map((param) => (
            <div key={param.id} className="p-3 bg-[var(--color-bg-hover)] rounded-lg">
              <div className="flex items-center justify-between mb-2">
                <label className="text-sm font-medium text-[var(--color-text-primary)]">
                  {param.name}
                  <span className="ml-2 text-xs text-[var(--color-info-text)] font-mono">${param.id}</span>
                </label>
              </div>
              {param.type === 'boolean' ? (
                <>
                  <select
                    value={paramResolutionValues[param.id] ?? param.default}
                    onChange={(e) => onParamValueChange(param.id, parseFloat(e.target.value))}
                    className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-sm text-[var(--color-text-primary)] focus:outline-none focus:border-[var(--color-border-focus)]"
                  >
                    <option value={1}>true</option>
                    <option value={0}>false</option>
                  </select>
                  <p className="text-xs text-[var(--color-text-muted)] mt-1">
                    Default from testing: {param.default === 1 ? 'true' : 'false'}
                  </p>
                </>
              ) : (
                <>
                  <input
                    type="number"
                    value={paramResolutionValues[param.id] ?? param.default}
                    onChange={(e) => onParamValueChange(param.id, parseFloat(e.target.value) || 0)}
                    className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-sm text-[var(--color-text-primary)] focus:outline-none focus:border-[var(--color-border-focus)]"
                  />
                  <p className="text-xs text-[var(--color-text-muted)] mt-1">
                    Default from testing: {param.default}
                  </p>
                </>
              )}
            </div>
          ))}
        </div>

        <div className="bg-[var(--color-info)]/20 border border-[var(--color-info)]/50 rounded p-3 mb-6">
          <p className="text-xs text-[var(--color-info-text)]">
            <strong>Note:</strong> These values will replace the dynamic parameters permanently.
            The strategy will be locked after promotion.
          </p>
        </div>

        <div className="flex justify-end gap-3">
          <button
            onClick={onCancel}
            className="px-4 py-2 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={onResolve}
            className="px-4 py-2 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors font-medium"
          >
            Apply Values & Continue
          </button>
        </div>
      </div>
    </div>
  );
};
