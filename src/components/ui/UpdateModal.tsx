import { useEffect } from 'react';
import type { Update } from '@tauri-apps/plugin-updater';
import { useAppUpdater, UpdateStatus, BuildMode } from '../../hooks/useAppUpdater';

interface UpdateModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** If true, show "up to date" message. If false (automatic check), close silently when up to date. */
  triggeredManually?: boolean;
  /** Auto-check on open */
  autoCheck?: boolean;
  /** Pre-resolved update from startup check — skips redundant check() call */
  preResolvedUpdate?: Update | null;
}

export const UpdateModal = ({
  isOpen,
  onClose,
  triggeredManually = false,
  autoCheck = true,
  preResolvedUpdate,
}: UpdateModalProps) => {
  const {
    status,
    progress,
    error,
    currentVersion,
    newVersion,
    buildMode,
    checkForUpdates,
    downloadAndInstall,
    restartApp,
    reset,
    setAvailable,
  } = useAppUpdater();

  // Seed hook state from pre-resolved update (startup check already called check())
  useEffect(() => {
    if (isOpen && preResolvedUpdate && status === 'idle') {
      setAvailable(preResolvedUpdate);
    }
  }, [isOpen, preResolvedUpdate, status, setAvailable]);

  // Auto-check when modal opens
  useEffect(() => {
    if (isOpen && autoCheck && status === 'idle') {
      checkForUpdates();
    }
  }, [isOpen, autoCheck, status, checkForUpdates]);

  // Auto-close if up-to-date, a non-updatable local build, or errored and
  // not manually triggered
  useEffect(() => {
    if (
      isOpen &&
      !triggeredManually &&
      (status === 'up-to-date' || status === 'error' || status === 'local-build')
    ) {
      onClose();
    }
  }, [isOpen, status, triggeredManually, onClose]);

  // Reset state when modal closes
  useEffect(() => {
    if (!isOpen) {
      reset();
    }
  }, [isOpen, reset]);

  // Escape key handling
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      // Don't allow closing during download or ready state
      if (e.key === 'Escape' && isOpen && status !== 'downloading' && status !== 'ready') {
        onClose();
      }
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [isOpen, status, onClose]);

  if (!isOpen) return null;

  // For automatic startup checks, don't render anything until we have a result.
  // This prevents a spinner flash on every app launch.
  if (!triggeredManually && (status === 'idle' || status === 'checking')) {
    return null;
  }

  const canClose = status !== 'downloading' && status !== 'ready';

  return (
    <div
      className="fixed inset-0 z-[150] flex items-center justify-center"
      onClick={canClose ? onClose : undefined}
    >
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-gray-800 rounded-lg shadow-xl max-w-md w-full mx-4 border border-gray-700"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-gray-700">
          <h3 className="text-lg font-semibold">Software Update</h3>
          {canClose && (
            <button
              onClick={onClose}
              className="text-gray-400 hover:text-white transition-colors"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          )}
        </div>

        {/* Content */}
        <div className="px-6 py-6">
          {renderContent(status, {
            progress,
            error,
            currentVersion,
            newVersion,
            buildMode,
            onRetry: checkForUpdates,
            onDownload: downloadAndInstall,
            onRestart: restartApp,
            onLater: onClose,
          })}
        </div>
      </div>
    </div>
  );
};

interface ContentProps {
  progress: number;
  error: string | null;
  currentVersion: string;
  newVersion: string | null;
  buildMode: BuildMode;
  onRetry: () => void;
  onDownload: () => void;
  onRestart: () => void;
  onLater: () => void;
}

function renderContent(status: UpdateStatus, props: ContentProps) {
  switch (status) {
    case 'idle':
    case 'checking':
      return (
        <div className="flex flex-col items-center py-4">
          <div className="animate-spin rounded-full h-10 w-10 border-b-2 border-blue-500 mb-4" />
          <p className="text-gray-300">Checking for updates...</p>
        </div>
      );

    case 'up-to-date':
      return (
        <div className="flex flex-col items-center py-4">
          <div className="w-12 h-12 rounded-full bg-green-600/20 flex items-center justify-center mb-4">
            <svg className="w-6 h-6 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
            </svg>
          </div>
          <p className="text-gray-300 text-center">
            You're running the latest version
          </p>
          {props.buildMode === 'staging' ? (
            <p className="text-amber-400 text-sm mt-1">
              Staging Pre-release {props.currentVersion}
            </p>
          ) : props.buildMode === 'development' ? (
            <p className="text-blue-400 text-sm mt-1">
              Development {props.currentVersion}
            </p>
          ) : (
            <p className="text-gray-500 text-sm mt-1">
              Version {props.currentVersion}
            </p>
          )}
        </div>
      );

    case 'available':
      return (
        <div className="space-y-4">
          <div className="flex items-start gap-4">
            <div className="w-12 h-12 rounded-full bg-blue-600/20 flex items-center justify-center flex-shrink-0">
              <svg className="w-6 h-6 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
              </svg>
            </div>
            <div>
              <p className="text-gray-300">
                A new {props.buildMode === 'staging' ? 'pre-release ' : ''}version of wickd is available!
              </p>
              <p className={`text-sm mt-1 ${props.buildMode === 'staging' ? 'text-amber-400' : 'text-gray-500'}`}>
                {props.currentVersion} → {props.newVersion}
              </p>
            </div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <button
              onClick={props.onLater}
              className="px-4 py-2 bg-gray-600 rounded hover:bg-gray-500 transition-colors"
            >
              Later
            </button>
            <button
              onClick={props.onDownload}
              className="px-4 py-2 bg-blue-600 rounded hover:bg-blue-500 transition-colors"
            >
              Download & Install
            </button>
          </div>
        </div>
      );

    case 'downloading':
      return (
        <div className="space-y-4">
          <p className="text-gray-300 text-center">
            Downloading update...
          </p>
          <div className="w-full bg-gray-700 rounded-full h-2">
            <div
              className="bg-blue-600 h-2 rounded-full transition-all duration-300"
              style={{ width: `${props.progress}%` }}
            />
          </div>
          <p className="text-gray-500 text-sm text-center">
            {props.progress}%
          </p>
        </div>
      );

    case 'ready':
      return (
        <div className="space-y-4">
          <div className="flex flex-col items-center">
            <div className="w-12 h-12 rounded-full bg-green-600/20 flex items-center justify-center mb-4">
              <svg className="w-6 h-6 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
              </svg>
            </div>
            <p className="text-gray-300 text-center">
              Update downloaded! Restart to apply changes.
            </p>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <button
              onClick={props.onLater}
              className="px-4 py-2 bg-gray-600 rounded hover:bg-gray-500 transition-colors"
            >
              Later
            </button>
            <button
              onClick={props.onRestart}
              className="px-4 py-2 bg-green-600 rounded hover:bg-green-500 transition-colors"
            >
              Restart Now
            </button>
          </div>
        </div>
      );

    case 'local-build':
      return (
        <div className="space-y-4" data-testid="update-local-build">
          <div className="flex items-start gap-4">
            <div className="w-12 h-12 rounded-full bg-gray-600/20 flex items-center justify-center flex-shrink-0">
              <svg className="w-6 h-6 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
              </svg>
            </div>
            <div>
              <p className="text-gray-300">
                This is a local build — self-update is disabled
              </p>
              <p className="text-gray-500 text-sm mt-1">
                Version {props.currentVersion}, built from source. Updates are
                installed by rebuilding and reinstalling; release builds check
                GitHub Releases automatically.
              </p>
            </div>
          </div>
          <div className="flex justify-end pt-2">
            <button
              onClick={props.onLater}
              className="px-4 py-2 bg-gray-600 rounded hover:bg-gray-500 transition-colors"
            >
              Close
            </button>
          </div>
        </div>
      );

    case 'error':
      return (
        <div className="space-y-4">
          <div className="flex items-start gap-4">
            <div className="w-12 h-12 rounded-full bg-red-600/20 flex items-center justify-center flex-shrink-0">
              <svg className="w-6 h-6 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
            </div>
            <div>
              <p className="text-gray-300">
                Unable to check for updates
              </p>
              <p className="text-red-400 text-sm mt-1">
                {props.error}
              </p>
            </div>
          </div>
          <div className="flex justify-end gap-3 pt-2">
            <button
              onClick={props.onLater}
              className="px-4 py-2 bg-gray-600 rounded hover:bg-gray-500 transition-colors"
            >
              Close
            </button>
            <button
              onClick={props.onRetry}
              className="px-4 py-2 bg-blue-600 rounded hover:bg-blue-500 transition-colors"
            >
              Try Again
            </button>
          </div>
        </div>
      );

    default:
      return null;
  }
}
