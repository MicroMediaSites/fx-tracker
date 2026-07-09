import React, { Component, ReactNode, lazy, Suspense } from 'react';
import ReactDOM from 'react-dom/client';
import { Router, Route, Switch } from 'wouter';
import { CredentialGate } from './components/CredentialGate';
import './index.css';

// Detect OS and add class to html element for platform-specific styling
const isWindows = navigator.userAgent.includes('Windows');
if (isWindows) {
  document.documentElement.classList.add('os-windows');
}

declare const __BUILD_MODE__: 'development' | 'staging' | 'production';

// Block DevTools access in production (keep available in staging/dev)
if (__BUILD_MODE__ === 'production') {
  // Disable right-click context menu (Inspect Element)
  document.addEventListener('contextmenu', (e) => e.preventDefault());

  // Block DevTools keyboard shortcuts
  document.addEventListener('keydown', (e) => {
    // Cmd+Opt+I (macOS) / Ctrl+Shift+I (Windows/Linux)
    if ((e.metaKey || e.ctrlKey) && e.altKey && e.key === 'i') {
      e.preventDefault();
    }
    // Cmd+Opt+J (macOS) / Ctrl+Shift+J (Windows/Linux) — Console
    if ((e.metaKey || e.ctrlKey) && e.altKey && e.key === 'j') {
      e.preventDefault();
    }
    // F12
    if (e.key === 'F12') {
      e.preventDefault();
    }
    // Cmd+Shift+C (macOS) / Ctrl+Shift+C (Windows/Linux) — Element picker
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === 'c') {
      e.preventDefault();
    }
  });
}

// BUG-040: Prevent browser/webview zoom - reserve pinch/scroll for chart zoom
// Wheel events with Ctrl/Cmd (scroll-to-zoom)
document.addEventListener('wheel', (e) => {
  if (e.ctrlKey || e.metaKey) {
    e.preventDefault();
  }
}, { passive: false });
// WebKit gesture events (trackpad pinch-to-zoom on macOS)
document.addEventListener('gesturestart', (e) => e.preventDefault());
document.addEventListener('gesturechange', (e) => e.preventDefault());

// Lazy load app components - each window only loads what it needs.
// AGT-652: surface reduced to local (strategy viewer, boot), chart, backtest,
// and watcher (the wickd daemon's front window). Account/ticket/tradeanalysis/
// login windows were deleted with the in-app engine and auth shell.
const BacktestApp = lazy(() => import('./BacktestApp').then((m) => ({ default: m.BacktestApp })));
const ChartApp = lazy(() => import('./ChartApp').then((m) => ({ default: m.ChartApp })));
const StrategyWatcherApp = lazy(() =>
  import('./StrategyWatcherApp').then((m) => ({ default: m.StrategyWatcherApp }))
);
const LocalApp = lazy(() => import('./LocalApp').then((m) => ({ default: m.LocalApp })));

// Global error handler to catch unhandled errors
window.onerror = (message, source, lineno, colno, error) => {
  console.error('[Global Error]', { message, source, lineno, colno, error });
  return true;
};

window.onunhandledrejection = (event) => {
  console.error('[Unhandled Promise Rejection]', event.reason);
  event.preventDefault();
};

// Error boundary to catch React errors
interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

class ErrorBoundary extends Component<{ children: ReactNode }, ErrorBoundaryState> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error('[ErrorBoundary] Caught error:', error, errorInfo);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="min-h-screen bg-[var(--color-bg-page)] flex items-center justify-center p-4">
          <div className="bg-[var(--color-sell)]/10 border border-[var(--color-sell)]/30 rounded-lg p-6 max-w-lg">
            <h2 className="text-[var(--color-sell)] text-xl font-bold mb-2">Something went wrong</h2>
            <p className="text-[var(--color-text-secondary)] mb-4">{this.state.error?.message || 'Unknown error'}</p>
            <button
              onClick={() => this.setState({ hasError: false, error: null })}
              className="px-4 py-2 bg-[var(--color-info)] hover:bg-[var(--color-info)]/80 rounded transition-colors"
            >
              Try Again
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

// Detect which window we're in based on URL param
const urlParams = new URLSearchParams(window.location.search);
const windowType = urlParams.get('window');

// Local-first window (AGT-642): the default cold-boot window. Served entirely
// from the local SQLite store (~/.wickd/app.db) — no sign-in, works offline.
// AGT-652: also the default when no ?window= param is present (the login
// window is gone).
const isLocalWindow = windowType === 'local' || !windowType;

// Loading fallback for lazy-loaded components
const LoadingFallback = () => (
  <div className="min-h-screen bg-[var(--color-bg-page)] flex items-center justify-center">
    <div className="text-[var(--color-text-muted)]">Loading...</div>
  </div>
);

// App window wrapper (AGT-650): no auth requirement and no Zero context —
// every domain reads the local SQLite store or invokes Tauri commands, so app
// windows work fully offline. The credential gate (local crypto vault) still
// guards OANDA access.
const AppWindowWrapper = () => {
  const getMainApp = () => {
    switch (windowType) {
      case 'backtest':
        return BacktestApp;
      case 'chart':
        return ChartApp;
      case 'watcher':
      default:
        return StrategyWatcherApp;
    }
  };
  const MainApp = getMainApp();

  return (
    <Suspense fallback={<LoadingFallback />}>
      <Router>
        <Switch>
          <Route>
            {() => (
              <CredentialGate>
                <MainApp />
              </CredentialGate>
            )}
          </Route>
        </Switch>
      </Router>
    </Suspense>
  );
};

const root = ReactDOM.createRoot(document.getElementById('root')!);
root.render(
  <React.StrictMode>
    <ErrorBoundary>
      {isLocalWindow ? (
        // Local-first boot path (AGT-642): the default window.
        <Suspense fallback={<LoadingFallback />}>
          <LocalApp />
        </Suspense>
      ) : (
        // App windows: local store + Tauri commands only — no auth, no Zero.
        <AppWindowWrapper />
      )}
    </ErrorBoundary>
  </React.StrictMode>
);
