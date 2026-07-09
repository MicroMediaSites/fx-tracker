import { create } from 'zustand';

/**
 * Store for credential modal form state
 *
 * This persists form values across component unmounts/remounts
 * which can happen during auth token refreshes when switching windows.
 */
interface CredentialFormState {
  // Form values
  masterPassword: string;
  apiKey: string;
  accountId: string;

  // Actions
  setMasterPassword: (value: string) => void;
  setApiKey: (value: string) => void;
  setAccountId: (value: string) => void;
  clearForm: () => void;
}

export const useCredentialFormStore = create<CredentialFormState>((set) => ({
  masterPassword: '',
  apiKey: '',
  accountId: '',

  setMasterPassword: (value) => set({ masterPassword: value }),
  setApiKey: (value) => set({ apiKey: value }),
  setAccountId: (value) => set({ accountId: value }),
  clearForm: () => set({ masterPassword: '', apiKey: '', accountId: '' }),
}));
