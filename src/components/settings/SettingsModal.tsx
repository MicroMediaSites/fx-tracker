import { useState, useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import {
  isPermissionGranted,
  requestPermission,
} from '@tauri-apps/plugin-notification';
import { useSettingsStore, StartupWindow, DataSource } from '../../stores/settingsStore';
import { useDesktopCredentials } from '../../hooks/useDesktopCredentials';
import { AddLiveCredentialsModal } from './AddLiveCredentialsModal';
import { UpdateCredentialsModal } from './UpdateCredentialsModal';

interface OandaCredentials {
  apiKeyPreview: string;
  accountId: string;
  accountAlias: string | null;
  environment: string;
  isConfigured: boolean;
}

interface OandaInstrument {
  name: string;
  displayName: string;
  instrumentType: string;
}

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const SettingsModal = ({ isOpen, onClose }: SettingsModalProps) => {
  const {
    dataSource,
    setDataSource,
    devUsePracticeUrlForLive,
    startupWindows,
    toggleStartupWindow,
    mySymbols,
    addSymbol,
    removeSymbol,
    desktopNotifications,
    setDesktopNotifications,
  } = useSettingsStore();

  const {
    hasLiveCredentials,
    practiceAccountId,
    liveAccountId,
    practiceBlob,
    liveBlob,
    updateCredentials,
    deleteCredentials,
    refreshCredentialStatus,
  } = useDesktopCredentials();


  // Ensure dataSource has a valid value
  // Fall back to 'demo' if undefined/null OR if 'live' but no live credentials
  const effectiveDataSource =
    (!dataSource || (dataSource === 'live' && !hasLiveCredentials))
      ? 'demo'
      : dataSource;

  const [newSymbol, setNewSymbol] = useState('');
  const [isChangingEnvironment, setIsChangingEnvironment] = useState(false);
  const [showAddLiveModal, setShowAddLiveModal] = useState(false);
  const [showUpdateModal, setShowUpdateModal] = useState(false);
  const [updateEnvironment, setUpdateEnvironment] = useState<'practice' | 'live'>('practice');
  const [oandaCredentials, setOandaCredentials] = useState<OandaCredentials | null>(null);
  const [isRequestingNotificationPermission, setIsRequestingNotificationPermission] = useState(false);
  const [availableInstruments, setAvailableInstruments] = useState<OandaInstrument[]>([]);
  const [isLoadingInstruments, setIsLoadingInstruments] = useState(false);
  const [isSymbolPickerOpen, setIsSymbolPickerOpen] = useState(false);
  const symbolPickerRef = useRef<HTMLDivElement>(null);
  const symbolInputRef = useRef<HTMLInputElement>(null);

  // Fetch OANDA credentials and instruments when modal opens
  useEffect(() => {
    if (isOpen) {
      invoke<OandaCredentials>('get_oanda_credentials')
        .then(setOandaCredentials)
        .catch((err) => console.error('Failed to fetch OANDA credentials:', err));

      // Fetch available instruments
      setIsLoadingInstruments(true);
      invoke<OandaInstrument[]>('fetch_instruments')
        .then(setAvailableInstruments)
        .catch((err) => console.error('Failed to fetch instruments:', err))
        .finally(() => setIsLoadingInstruments(false));
    }
  }, [isOpen, effectiveDataSource]);

  const handleAddLiveComplete = async (newLiveAccountId: string, newLiveBlob: string) => {
    // Update Zero with live account ID and encrypted live blob
    await updateCredentials(undefined, newLiveBlob, undefined, newLiveAccountId);
    // Refresh credential status to update hasLiveCredentials in the UI
    await refreshCredentialStatus();
    setShowAddLiveModal(false);
  };

  const handleUpdateComplete = async (
    updatedAccountId: string,
    environment: 'practice' | 'live'
  ) => {
    // Update Zero with new account ID for the updated environment
    const practiceAccountIdUpdate = environment === 'practice' ? updatedAccountId : undefined;
    const liveAccountIdUpdate = environment === 'live' ? updatedAccountId : undefined;

    try {
      await updateCredentials(
        undefined, // No blob changes
        undefined, // No blob changes
        practiceAccountIdUpdate,
        liveAccountIdUpdate
      );
    } catch (err) {
      console.error('[SettingsModal] Zero update failed:', err);
      throw err; // Re-throw so UpdateCredentialsModal shows the error
    }

    setShowUpdateModal(false);
  };

  const handleDeleteCredentials = async (environment: 'practice' | 'live') => {
    if (environment === 'practice') {
      // Deleting practice credentials resets everything - user must set up vault again
      await deleteCredentials();
      // lockVault is called inside deleteCredentials, which will trigger redirect to onboarding
      onClose();
    } else {
      // Deleting live credentials:
      // 1. Clear live credentials from in-memory vault
      // 2. Update Zero to null out live_blob and live_account_id
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('clear_live_credentials');
      await updateCredentials(undefined, null, undefined, null);
      await refreshCredentialStatus();
      // If currently on live data source, switch to demo
      if (dataSource === 'live') {
        await handleDataSourceChange('demo');
      }
      setShowUpdateModal(false);
    }
  };

  const openUpdateModal = (env: 'practice' | 'live') => {
    setUpdateEnvironment(env);
    setShowUpdateModal(true);
  };

  const handleApiKeyUpdate = async (
    newBlob: string,
    environment: 'practice' | 'live',
    newAccountId?: string
  ) => {
    // Update Zero with new API key blob and optionally new account ID
    if (environment === 'practice') {
      await updateCredentials(newBlob, undefined, newAccountId, undefined);
    } else {
      await updateCredentials(undefined, newBlob, undefined, newAccountId);
    }
    await refreshCredentialStatus();
    setShowUpdateModal(false);
  };

  const handleDataSourceChange = async (newSource: DataSource) => {
    // For demo/live, we need to switch the OANDA environment
    const environment = newSource === 'live' ? 'live' : 'practice';
    const accountId = newSource === 'live' ? liveAccountId : practiceAccountId;

    if (!accountId) {
      console.error(`No account ID found for ${environment}`);
      setSettingsError(`No account configured for ${environment} environment`);
      return;
    }

    // Dev mode: when switching to "live" with dev toggle enabled, use practice URL
    const usePracticeUrl = newSource === 'live' && devUsePracticeUrlForLive;

    setIsChangingEnvironment(true);
    try {
      await invoke('switch_oanda_environment', { environment, accountId, usePracticeUrl });
      setDataSource(newSource);
      // Notify all windows of environment change
      emit('environment-changed', { source: newSource, environment });
    } catch (error) {
      console.error('[SettingsModal] Failed to switch environment:', error);
      const message = error instanceof Error ? error.message : String(error);
      setSettingsError(`Failed to switch environment: ${message}`);
    } finally {
      setIsChangingEnvironment(false);
    }
  };

  const [notificationError, setNotificationError] = useState<string | null>(null);
  const [notificationSuccess, setNotificationSuccess] = useState(false);
  const [isSendingTestNotification, setIsSendingTestNotification] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);

  // Auto-dismiss settings error after 5 seconds
  useEffect(() => {
    if (settingsError) {
      const timer = setTimeout(() => setSettingsError(null), 5000);
      return () => clearTimeout(timer);
    }
  }, [settingsError]);

  // Click-outside handler for symbol picker
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (symbolPickerRef.current && !symbolPickerRef.current.contains(event.target as Node)) {
        setIsSymbolPickerOpen(false);
      }
    };
    if (isSymbolPickerOpen) {
      document.addEventListener('mousedown', handleClickOutside);
      return () => document.removeEventListener('mousedown', handleClickOutside);
    }
  }, [isSymbolPickerOpen]);

  // Reset symbol picker state when modal closes
  useEffect(() => {
    if (!isOpen) {
      setIsSymbolPickerOpen(false);
      setNewSymbol('');
    }
  }, [isOpen]);

  const handleNotificationToggle = async () => {
    setNotificationError(null);

    if (desktopNotifications) {
      // Turning off - update frontend setting and notify backend
      setDesktopNotifications(false);
      await invoke('set_desktop_notifications_enabled', { enabled: false });
      return;
    }

    // Turning on - check/request permission
    setIsRequestingNotificationPermission(true);
    try {
      let permissionGranted = await isPermissionGranted();
      if (!permissionGranted) {
        const permission = await requestPermission();
        permissionGranted = permission === 'granted';
      }

      if (permissionGranted) {
        setDesktopNotifications(true);
        // Notify backend that notifications are enabled
        await invoke('set_desktop_notifications_enabled', { enabled: true });
        // Reset the notification banner so it can show again if later disabled
        localStorage.removeItem('candlesight_notification_banner_hidden');
      } else {
        setNotificationError('Permission denied. Enable notifications in System Settings > Notifications.');
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setNotificationError(`Failed to request permission: ${message}`);
      console.error('[Settings] Failed to request notification permission:', err);
    } finally {
      setIsRequestingNotificationPermission(false);
    }
  };

  if (!isOpen) return null;

  // Normalize symbol for searching (strip slashes, underscores, spaces)
  const normalizeSymbol = (s: string) => s.toUpperCase().replace(/[_/\s-]/g, '');

  const handleAddAllSymbols = () => {
    // Add all forex pairs (CURRENCY type) that aren't already added
    const forexInstruments = availableInstruments
      .filter((i) => i.instrumentType === 'CURRENCY' && !mySymbols.includes(i.name))
      .map((i) => i.name);

    forexInstruments.forEach((symbol) => addSymbol(symbol));
  };

  // Filter to instruments not already in mySymbols
  // Also filter by search input if user is typing
  const normalizedSearch = normalizeSymbol(newSymbol);
  const availableToAdd = availableInstruments
    .filter((i) => !mySymbols.includes(i.name))
    .filter((i) => {
      if (!normalizedSearch) return true;
      // Match against normalized name or display name
      return normalizeSymbol(i.name).includes(normalizedSearch) ||
             normalizeSymbol(i.displayName).includes(normalizedSearch);
    })
    .sort((a, b) => {
      // Sort CURRENCY first, then alphabetically
      if (a.instrumentType === 'CURRENCY' && b.instrumentType !== 'CURRENCY') return -1;
      if (a.instrumentType !== 'CURRENCY' && b.instrumentType === 'CURRENCY') return 1;
      return a.name.localeCompare(b.name);
    });

  return createPortal(
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-[10000]">
      <div className="bg-[var(--color-bg-card)] rounded-lg border border-[var(--color-border)] w-full max-w-lg max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex justify-between items-center p-4 border-b border-[var(--color-border)]">
          <h2 className="text-lg font-semibold text-[var(--color-text-primary)]">Settings</h2>
          <button
            onClick={onClose}
            className="text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
          >
            <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Error Toast */}
        {settingsError && (
          <div className="mx-4 mt-4 px-4 py-3 bg-[var(--color-sell-bg)] border border-[var(--color-sell-border)] text-[var(--color-sell-text)] text-sm rounded-lg flex items-center justify-between">
            <span>{settingsError}</span>
            <button
              onClick={() => setSettingsError(null)}
              className="text-[var(--color-sell-text)] hover:opacity-80 ml-3"
            >
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        )}

        <div className="p-4 space-y-6">
          {/* Data Source */}
          <section>
            <h3 className="text-sm font-medium text-[var(--color-text-secondary)] mb-3">Data Source</h3>
            <p className="text-xs text-[var(--color-text-muted)] mb-3">Choose where trading data comes from</p>
            <div className="space-y-2">
              <label className="flex items-center gap-3 cursor-pointer">
                <div className={`w-4 h-4 rounded-full border-2 flex items-center justify-center ${
                  effectiveDataSource === 'demo' ? 'border-[var(--color-info)] bg-[var(--color-info)]' : 'border-[var(--color-text-muted)]'
                }`}>
                  {effectiveDataSource === 'demo' && <div className="w-2 h-2 rounded-full bg-[var(--color-bg-card)]" />}
                </div>
                <input
                  type="radio"
                  name="dataSource"
                  checked={effectiveDataSource === 'demo'}
                  onChange={() => handleDataSourceChange('demo')}
                  disabled={isChangingEnvironment}
                  className="sr-only"
                />
                <div className="flex-1">
                  <span className="text-sm text-[var(--color-text-primary)]">Demo Account</span>
                  <span className="text-xs text-[var(--color-text-muted)] ml-2">(OANDA Practice - paper trading)</span>
                </div>
              </label>
              {hasLiveCredentials ? (
                <label className="flex items-center gap-3 cursor-pointer">
                  <div className={`w-4 h-4 rounded-full border-2 flex items-center justify-center ${
                    effectiveDataSource === 'live' ? 'border-[var(--color-buy)] bg-[var(--color-buy)]' : 'border-[var(--color-text-muted)]'
                  }`}>
                    {effectiveDataSource === 'live' && <div className="w-2 h-2 rounded-full bg-[var(--color-bg-card)]" />}
                  </div>
                  <input
                    type="radio"
                    name="dataSource"
                    checked={effectiveDataSource === 'live'}
                    onChange={() => handleDataSourceChange('live')}
                    disabled={isChangingEnvironment}
                    className="sr-only"
                  />
                  <div className="flex-1">
                    <span className="text-sm text-[var(--color-buy-text)]">Live Account</span>
                    <span className="text-xs text-[var(--color-buy-text)]/70 ml-2">(Real money - use with caution)</span>
                  </div>
                </label>
              ) : (
                <div className="flex items-center gap-3 pl-7 py-2">
                  <button
                    onClick={() => setShowAddLiveModal(true)}
                    className="text-sm text-[var(--color-info-text)] hover:opacity-80 transition-colors"
                  >
                    + Enable Live Trading
                  </button>
                  <span className="text-xs text-[var(--color-text-muted)]">(Add your OANDA live account credentials)</span>
                </div>
              )}
            </div>

            {isChangingEnvironment && (
              <p className="text-xs text-[var(--color-info-text)] mt-2">Switching environment...</p>
            )}

            {/* OANDA Accounts */}
            <div className="mt-6">
              <h4 className="text-xs font-medium text-[var(--color-text-muted)] uppercase tracking-wide mb-2">OANDA Accounts</h4>
              <div className="space-y-1">
                <div
                  className="flex items-center justify-between py-2 px-2 hover:bg-[var(--color-bg-hover)] transition-colors"
                  style={{ borderLeft: '3px solid var(--color-warning)' }}
                >
                  <div className="flex flex-col">
                    <span className="text-xs font-medium text-[var(--color-warning-text)]">Demo</span>
                    {practiceAccountId && (
                      <span className="text-[10px] text-[var(--color-text-muted)] font-mono truncate max-w-[180px]" title={practiceAccountId}>
                        {oandaCredentials?.environment === 'practice' && oandaCredentials.accountAlias
                          ? oandaCredentials.accountAlias
                          : practiceAccountId}
                      </span>
                    )}
                  </div>
                  <button
                    onClick={() => openUpdateModal('practice')}
                    className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
                  >
                    Update
                  </button>
                </div>
                {hasLiveCredentials && (
                  <div
                    className="flex items-center justify-between py-2 px-2 hover:bg-[var(--color-bg-hover)] transition-colors"
                    style={{ borderLeft: '3px solid var(--color-buy)' }}
                  >
                    <div className="flex flex-col">
                      <span className="text-xs font-medium text-[var(--color-buy-text)]">Live</span>
                      {liveAccountId && (
                        <span className="text-[10px] text-[var(--color-text-muted)] font-mono truncate max-w-[180px]" title={liveAccountId}>
                          {oandaCredentials?.environment === 'live' && oandaCredentials.accountAlias
                            ? oandaCredentials.accountAlias
                            : liveAccountId}
                        </span>
                      )}
                    </div>
                    <button
                      onClick={() => openUpdateModal('live')}
                      className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
                    >
                      Update
                    </button>
                  </div>
                )}
              </div>
            </div>
          </section>

          {/* Startup Windows */}
          <section>
            <h3 className="text-sm font-medium text-[var(--color-text-secondary)] mb-3">Startup Windows</h3>
            <p className="text-xs text-[var(--color-text-muted)] mb-3">Choose which windows open when you launch the app</p>
            <div className="space-y-2">
              {([
                  { id: 'charting', label: 'Charting' },
                  { id: 'backtesting', label: 'Strategy' },
                  { id: 'watcher', label: 'Live Monitor' },
                ] as { id: StartupWindow; label: string }[]).map(({ id, label }) => (
                <label key={id} className="flex items-center gap-3 cursor-pointer">
                  <div className={`w-4 h-4 rounded border-2 flex items-center justify-center ${
                    startupWindows.includes(id) ? 'border-[var(--color-info)] bg-[var(--color-info)]' : 'border-[var(--color-text-muted)]'
                  }`}>
                    {startupWindows.includes(id) && (
                      <svg className="w-3 h-3 text-[var(--color-bg-card)]" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                      </svg>
                    )}
                  </div>
                  <input
                    type="checkbox"
                    checked={startupWindows.includes(id)}
                    onChange={() => toggleStartupWindow(id)}
                    className="sr-only"
                  />
                  <span className="text-sm text-[var(--color-text-primary)]">{label}</span>
                </label>
              ))}
            </div>
          </section>

          {/* Desktop Notifications */}
          <section>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-sm font-medium text-[var(--color-text-secondary)]">Desktop Notifications</h3>
              <button
                onClick={async () => {
                  setNotificationError(null);
                  setNotificationSuccess(false);
                  setIsSendingTestNotification(true);
                  try {
                    const success = await invoke<boolean>('test_notification');
                    if (success) {
                      setNotificationSuccess(true);
                      setTimeout(() => setNotificationSuccess(false), 3000);
                    } else {
                      setNotificationError('Notification failed. Check System Settings > Notifications.');
                    }
                  } catch (err) {
                    console.error('[Settings] Test notification failed:', err);
                    setNotificationError(`Test notification failed: ${err instanceof Error ? err.message : String(err)}`);
                  } finally {
                    setIsSendingTestNotification(false);
                  }
                }}
                disabled={isSendingTestNotification}
                className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] disabled:opacity-50 transition-colors"
              >
                {isSendingTestNotification ? 'Sending...' : 'Send Test'}
              </button>
            </div>
            <p className="text-xs text-[var(--color-text-muted)] mb-3">Get notified when strategy watchers detect pattern matches</p>
            <div className="space-y-2">
              <label className="flex items-center gap-3 cursor-pointer">
                <div className={`w-4 h-4 rounded-full border-2 flex items-center justify-center ${
                  !desktopNotifications ? 'border-[var(--color-info)] bg-[var(--color-info)]' : 'border-[var(--color-text-muted)]'
                }`}>
                  {!desktopNotifications && <div className="w-2 h-2 rounded-full bg-[var(--color-bg-card)]" />}
                </div>
                <input
                  type="radio"
                  name="notifications"
                  checked={!desktopNotifications}
                  onChange={() => desktopNotifications && handleNotificationToggle()}
                  disabled={isRequestingNotificationPermission}
                  className="sr-only"
                />
                <span className="text-sm text-[var(--color-text-primary)]">Disabled</span>
              </label>
              <label className="flex items-center gap-3 cursor-pointer">
                <div className={`w-4 h-4 rounded-full border-2 flex items-center justify-center ${
                  desktopNotifications ? 'border-[var(--color-info)] bg-[var(--color-info)]' : 'border-[var(--color-text-muted)]'
                }`}>
                  {desktopNotifications && <div className="w-2 h-2 rounded-full bg-[var(--color-bg-card)]" />}
                </div>
                <input
                  type="radio"
                  name="notifications"
                  checked={desktopNotifications}
                  onChange={() => !desktopNotifications && handleNotificationToggle()}
                  disabled={isRequestingNotificationPermission}
                  className="sr-only"
                />
                <span className="text-sm text-[var(--color-text-primary)]">Enabled</span>
              </label>
            </div>
            {notificationError && (
              <p className="text-xs text-[var(--color-sell-text)] mt-2">{notificationError}</p>
            )}
            {notificationSuccess && (
              <p className="text-xs text-[var(--color-buy-text)] mt-2">Test notification sent. Check your notification center.</p>
            )}
          </section>

          {/* My Symbols */}
          <section>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-sm font-medium text-[var(--color-text-secondary)]">My Symbols</h3>
              <button
                onClick={handleAddAllSymbols}
                disabled={isLoadingInstruments || availableToAdd.filter((i) => i.instrumentType === 'CURRENCY').length === 0}
                className="text-xs text-[var(--color-info-text)] hover:opacity-80 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                + Add All Forex
              </button>
            </div>
            <p className="text-xs text-[var(--color-text-muted)] mb-3">Customize which currency pairs appear in dropdowns</p>

            {/* Current symbols */}
            <div className="flex flex-wrap gap-x-3 gap-y-1 mb-3">
              {mySymbols.map((symbol) => (
                <span
                  key={symbol}
                  className="inline-flex items-center gap-1 text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
                >
                  {normalizeSymbol(symbol)}
                  <button
                    onClick={() => removeSymbol(symbol)}
                    className="text-[var(--color-text-muted)] hover:text-[var(--color-sell-text)] transition-colors"
                  >
                    <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                    </svg>
                  </button>
                </span>
              ))}
            </div>

            {/* Symbol picker - styled like SymbolPicker component */}
            <div className="relative" ref={symbolPickerRef}>
              <div
                className={`flex items-center bg-transparent border rounded transition-colors ${
                  isSymbolPickerOpen
                    ? 'border-[var(--color-border-focus)]'
                    : 'border-[var(--color-border)] hover:border-[var(--color-border-focus)]'
                } ${isLoadingInstruments ? 'opacity-50' : ''}`}
              >
                <input
                  ref={symbolInputRef}
                  type="text"
                  value={newSymbol}
                  onChange={(e) => {
                    setNewSymbol(e.target.value.toUpperCase());
                    setIsSymbolPickerOpen(true);
                  }}
                  onFocus={() => !isLoadingInstruments && setIsSymbolPickerOpen(true)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && availableToAdd.length > 0) {
                      addSymbol(availableToAdd[0].name);
                      setNewSymbol('');
                      setIsSymbolPickerOpen(false);
                    } else if (e.key === 'Escape') {
                      setIsSymbolPickerOpen(false);
                      setNewSymbol('');
                    }
                  }}
                  placeholder={isLoadingInstruments ? 'Loading...' : 'Type to search (e.g., EURUSD)'}
                  disabled={isLoadingInstruments}
                  className="flex-1 min-w-0 bg-transparent px-3 py-2 text-sm outline-none text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)] disabled:cursor-not-allowed"
                />
                <button
                  type="button"
                  onClick={() => {
                    if (isLoadingInstruments) return;
                    setIsSymbolPickerOpen(!isSymbolPickerOpen);
                    if (!isSymbolPickerOpen) symbolInputRef.current?.focus();
                  }}
                  disabled={isLoadingInstruments}
                  className="flex-shrink-0 px-3 py-2 text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors outline-none disabled:cursor-not-allowed"
                >
                  <svg
                    className={`w-4 h-4 transition-transform duration-200 ${isSymbolPickerOpen ? 'rotate-180' : ''}`}
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                    strokeWidth={2}
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
                  </svg>
                </button>
              </div>

              {/* Dropdown */}
              {isSymbolPickerOpen && !isLoadingInstruments && (
                <div className="absolute top-full left-0 right-0 mt-1 bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded shadow-xl z-50 max-h-48 overflow-auto">
                  {availableToAdd.length > 0 ? (
                    availableToAdd.slice(0, 50).map((instrument) => (
                      <button
                        key={instrument.name}
                        type="button"
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={() => {
                          addSymbol(instrument.name);
                          setNewSymbol('');
                          setIsSymbolPickerOpen(false);
                        }}
                        className="w-full text-left px-3 py-2 text-sm transition-colors text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-hover)] hover:text-[var(--color-text-primary)] flex justify-between items-center"
                      >
                        <span>{normalizeSymbol(instrument.name)}</span>
                        <span className="text-xs text-[var(--color-text-muted)]">{instrument.instrumentType}</span>
                      </button>
                    ))
                  ) : (
                    <div className="px-3 py-2 text-sm text-[var(--color-text-muted)]">
                      {newSymbol ? 'No matches found' : 'All instruments added'}
                    </div>
                  )}
                  {availableToAdd.length > 50 && (
                    <div className="px-3 py-2 text-xs text-[var(--color-text-muted)] border-t border-[var(--color-border)]">
                      Type to filter ({availableToAdd.length - 50} more...)
                    </div>
                  )}
                </div>
              )}
            </div>
          </section>

        </div>

        {/* Footer */}
        <div className="flex justify-end p-4 border-t border-[var(--color-border)]">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-[var(--color-bg-hover)] text-[var(--color-text-primary)] rounded hover:bg-[var(--color-bg-active)] transition-colors"
          >
            Done
          </button>
        </div>
      </div>

      {/* Add Live Credentials Modal */}
      {practiceBlob && (
        <AddLiveCredentialsModal
          isOpen={showAddLiveModal}
          practiceBlob={practiceBlob}
          onComplete={handleAddLiveComplete}
          onCancel={() => setShowAddLiveModal(false)}
        />
      )}

      {/* Update Credentials Modal */}
      <UpdateCredentialsModal
        isOpen={showUpdateModal}
        environment={updateEnvironment}
        currentBlob={(updateEnvironment === 'practice' ? practiceBlob : liveBlob) || undefined}
        onComplete={handleUpdateComplete}
        onApiKeyUpdate={handleApiKeyUpdate}
        onDelete={handleDeleteCredentials}
        onCancel={() => setShowUpdateModal(false)}
      />

    </div>,
    document.body
  );
}
