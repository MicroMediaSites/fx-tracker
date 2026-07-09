import { useState, useEffect } from 'react';

interface ScriptPanelProps {
  script: string;
  className?: string;
  editable?: boolean;
  onSave?: (script: string) => void;
}

export function ScriptPanel({ script, className, editable, onSave }: ScriptPanelProps) {
  const [editedScript, setEditedScript] = useState(script);
  const isModified = editedScript !== script;

  // Sync local state when the prop changes (e.g. after an external save)
  useEffect(() => {
    setEditedScript(script);
  }, [script]);

  return (
    <div className={`bg-[#1a1a2e] border border-white/10 rounded-lg overflow-auto ${className ?? ''}`}>
      <div className="flex items-center justify-between px-4 py-2 border-b border-white/10">
        <span className="text-xs text-white/50 uppercase tracking-wider">Rhai Script</span>
        {editable && (
          <div className="flex items-center gap-2">
            <button
              onClick={() => setEditedScript(script)}
              disabled={!isModified}
              className="px-2 py-1 text-xs rounded transition-colors disabled:opacity-30 text-white/60 hover:text-white hover:bg-white/10"
            >
              Reset
            </button>
            <button
              onClick={() => onSave?.(editedScript)}
              disabled={!isModified}
              className="px-2 py-1 text-xs rounded transition-colors disabled:opacity-30 bg-[var(--color-buy)] text-white hover:bg-[var(--color-buy)]/80"
            >
              Save
            </button>
          </div>
        )}
      </div>
      {editable ? (
        <textarea
          value={editedScript}
          onChange={(e) => setEditedScript(e.target.value)}
          maxLength={50000}
          className="w-full h-[500px] p-4 text-sm font-mono text-white/80 bg-transparent resize-y leading-relaxed outline-none"
          spellCheck={false}
        />
      ) : (
        <pre className="p-4 text-sm font-mono text-white/80 whitespace-pre overflow-x-auto leading-relaxed">
          {script}
        </pre>
      )}
    </div>
  );
}
