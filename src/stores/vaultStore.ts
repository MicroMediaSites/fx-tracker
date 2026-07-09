import { create } from 'zustand';

export interface PasswordStrength {
  score: number; // 0-4
  feedback: string[];
  isCompromised: boolean;
  breachCount: number;
  meetsRequirements: boolean;
}

export type VaultStatus =
  | 'loading'      // Checking initial state
  | 'no_credentials' // No credentials stored (needs onboarding)
  | 'locked'       // Credentials exist but vault is locked
  | 'unlocked';    // Vault is unlocked

interface VaultState {
  // Status
  status: VaultStatus;
  deviceId: string | null;
  hasPracticeCredentials: boolean;
  hasLiveCredentials: boolean;
  unlockedAt: number | null; // Timestamp when vault was unlocked (for animation timing)

  // Rate limiting
  rateLimitMessage: string | null;
  rateLimitSeconds: number | null;

  // Error state
  error: string | null;

  // Actions
  setStatus: (status: VaultStatus) => void;
  setDeviceId: (deviceId: string) => void;
  setCredentialStatus: (practice: boolean, live: boolean) => void;
  setUnlockedAt: (timestamp: number | null) => void;
  setRateLimitMessage: (message: string | null) => void;
  setRateLimitSeconds: (seconds: number | null) => void;
  setError: (error: string | null) => void;
  reset: () => void;
}

const initialState = {
  status: 'loading' as VaultStatus,
  deviceId: null,
  hasPracticeCredentials: false,
  hasLiveCredentials: false,
  unlockedAt: null,
  rateLimitMessage: null,
  rateLimitSeconds: null,
  error: null,
};

export const useVaultStore = create<VaultState>((set) => ({
  ...initialState,

  setStatus: (status) => set({ status }),
  setDeviceId: (deviceId) => set({ deviceId }),
  setCredentialStatus: (practice, live) => set({
    hasPracticeCredentials: practice,
    hasLiveCredentials: live
  }),
  setUnlockedAt: (timestamp) => set({ unlockedAt: timestamp }),
  setRateLimitMessage: (message) => set({ rateLimitMessage: message }),
  setRateLimitSeconds: (seconds) => set({ rateLimitSeconds: seconds }),
  setError: (error) => set({ error }),
  reset: () => set(initialState),
}));
