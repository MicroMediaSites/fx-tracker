import { create } from 'zustand';

interface UIState {
  // Welcome rays animation - plays once after vault unlock
  showWelcomeRays: boolean;
  triggerWelcomeRays: () => void;
  clearWelcomeRays: () => void;
}

export const useUIStore = create<UIState>()((set) => ({
  showWelcomeRays: false,
  triggerWelcomeRays: () => set({ showWelcomeRays: true }),
  clearWelcomeRays: () => set({ showWelcomeRays: false }),
}));
