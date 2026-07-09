import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useSettingsStore } from '../../stores/settingsStore';

type LogEntry = {
  id: number;
  timestamp: string;
  tag: string;
  message: string;
  data?: unknown;
};

// Global log store that persists across renders
const logStore: LogEntry[] = [];
let logId = 0;
let listeners: Set<() => void> = new Set();

// Subscribe to log updates
const subscribe = (callback: () => void) => {
  listeners.add(callback);
  return () => { listeners.delete(callback); };
};

// Add a log entry (called from debugLog)
export const addDebugLog = (tag: string, message: string, data?: unknown) => {
  const timestamp = new Date().toISOString().split('T')[1].slice(0, 12); // HH:MM:SS.mmm
  logStore.push({ id: logId++, timestamp, tag, message, data });

  // Keep only last 100 logs
  if (logStore.length > 100) {
    logStore.shift();
  }

  // Notify listeners
  listeners.forEach(cb => cb());
};

// Color coding for different tags
const tagColors: Record<string, string> = {
  'AUTH_FN': 'text-yellow-400',
  'INTERVAL': 'text-blue-400',
  'SESSION': 'text-purple-400',
  'MOUNT': 'text-green-400',
  'VISIBILITY': 'text-cyan-400',
  'FOCUS': 'text-cyan-400',
  'CANDLES': 'text-orange-400',
  'AI_MODEL': 'text-pink-400',
  'RECOVERY': 'text-red-400',
  'ZERO_CONNECTION': 'text-red-500',
  'ZERO_ERROR': 'text-red-400',
  'QUERY_STATE': 'text-amber-400',
  'USER_ID': 'text-lime-400',
};

// Clear Zero/Replicache IndexedDB databases and reload
const clearZeroCache = async () => {
  addDebugLog('RECOVERY', 'Clearing Zero cache...');
  try {
    const databases = await indexedDB.databases();
    const zeroDbNames = databases
      .map(db => db.name)
      .filter((name): name is string => !!name && name.startsWith('rep'));

    addDebugLog('RECOVERY', `Found ${zeroDbNames.length} Zero databases`, zeroDbNames);

    await Promise.all(
      zeroDbNames.map(name =>
        new Promise<void>((resolve) => {
          const request = indexedDB.deleteDatabase(name);
          request.onsuccess = () => resolve();
          request.onerror = () => resolve();
          request.onblocked = () => resolve();
        })
      )
    );

    addDebugLog('RECOVERY', 'Databases cleared, reloading...');
    window.location.reload();
  } catch (e) {
    addDebugLog('RECOVERY', 'Error clearing cache', e);
  }
};

export const DebugOverlay = () => {
  const [isOpen, setIsOpen] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([...logStore]);
  const [filter, setFilter] = useState<string>('');
  const [modelCheckLoading, setModelCheckLoading] = useState(false);
  const [modelResponse, setModelResponse] = useState<string | null>(null);
  const aiModel = useSettingsStore((state) => state.aiModel);

  const checkModel = useCallback(async () => {
    setModelCheckLoading(true);
    setModelResponse(null);
    try {
      const response = await invoke<string>('check_ai_model', { model: aiModel });
      setModelResponse(response);
      addDebugLog('AI_MODEL', `Checked ${aiModel}`, response);
    } catch (err) {
      const errorMsg = String(err);
      setModelResponse(`Error: ${errorMsg}`);
      addDebugLog('AI_MODEL', 'Check failed', errorMsg);
    } finally {
      setModelCheckLoading(false);
    }
  }, [aiModel]);

  // Subscribe to log updates
  useEffect(() => {
    return subscribe(() => {
      setLogs([...logStore]);
    });
  }, []);

  // Keyboard shortcut: Ctrl+Shift+D to toggle
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && e.key === 'D') {
        e.preventDefault();
        setIsOpen(prev => !prev);
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  const clearLogs = useCallback(() => {
    logStore.length = 0;
    setLogs([]);
  }, []);

  const filteredLogs = filter
    ? logs.filter(l => l.tag.includes(filter.toUpperCase()) || l.message.toLowerCase().includes(filter.toLowerCase()))
    : logs;

  // Mini indicator when closed
  if (!isOpen) {
    return (
      <button
        onClick={() => setIsOpen(true)}
        className="fixed bottom-2 right-2 z-[9999] bg-gray-800 text-gray-400 text-xs px-2 py-1 rounded opacity-50 hover:opacity-100 transition-opacity"
        title="Ctrl+Shift+D to toggle debug panel"
      >
        🔧 {logs.length}
      </button>
    );
  }

  return (
    <div className="fixed bottom-2 right-2 z-[9999] w-[500px] max-h-[400px] bg-gray-900 border border-gray-700 rounded-lg shadow-xl flex flex-col text-xs font-mono">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-700 bg-gray-800 rounded-t-lg">
        <span className="text-gray-300 font-semibold">Debug Console</span>
        <div className="flex items-center gap-2">
          <input
            type="text"
            placeholder="Filter..."
            value={filter}
            onChange={e => setFilter(e.target.value)}
            className="bg-gray-700 text-gray-200 px-2 py-0.5 rounded text-xs w-24"
          />
          <button
            onClick={clearLogs}
            className="text-gray-400 hover:text-white px-2"
            title="Clear logs"
          >
            Clear
          </button>
          <button
            onClick={() => setIsOpen(false)}
            className="text-gray-400 hover:text-white px-1"
            title="Close (Ctrl+Shift+D)"
          >
            ✕
          </button>
        </div>
      </div>

      {/* Debug Tools */}
      <div className="px-3 py-2 border-b border-gray-700 bg-gray-850">
        <div className="flex items-center gap-2 flex-wrap">
          <button
            onClick={checkModel}
            disabled={modelCheckLoading}
            className={`px-2 py-1 rounded text-xs ${
              modelCheckLoading
                ? 'bg-gray-600 text-gray-400 cursor-wait'
                : 'bg-pink-600 hover:bg-pink-500 text-white'
            }`}
          >
            {modelCheckLoading ? 'Checking...' : `Check Model (${aiModel})`}
          </button>
          <button
            onClick={clearZeroCache}
            className="px-2 py-1 rounded text-xs bg-red-600 hover:bg-red-500 text-white"
          >
            Clear Zero Cache
          </button>
          {modelResponse && (
            <div className="flex-1 text-pink-300 truncate" title={modelResponse}>
              {modelResponse}
            </div>
          )}
        </div>
      </div>

      {/* Logs */}
      <div className="flex-1 overflow-y-auto p-2 space-y-1">
        {filteredLogs.length === 0 ? (
          <div className="text-gray-500 text-center py-4">No logs yet...</div>
        ) : (
          filteredLogs.map(log => (
            <div key={log.id} className="flex gap-2 text-gray-300 leading-tight">
              <span className="text-gray-500 shrink-0">{log.timestamp}</span>
              <span className={`shrink-0 ${tagColors[log.tag] || 'text-gray-400'}`}>
                [{log.tag}]
              </span>
              <span className="flex-1">
                {log.message}
                {log.data !== undefined && (
                  <span className="text-gray-500 ml-1">
                    {typeof log.data === 'object' ? JSON.stringify(log.data) : String(log.data as string | number | boolean)}
                  </span>
                )}
              </span>
            </div>
          ))
        )}
      </div>

      {/* Footer hint */}
      <div className="px-3 py-1 border-t border-gray-700 text-gray-500 text-center">
        Ctrl+Shift+D to toggle • Watching for auth issues
      </div>
    </div>
  );
};
