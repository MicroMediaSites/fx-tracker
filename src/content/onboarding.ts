/**
 * Onboarding welcome content.
 *
 * Moved in-app from the deleted @candlesight/content package (AGT-653):
 * the tier/entitlement machinery is gone, so this is just the static
 * feature highlights shown on the onboarding welcome step.
 */

export interface HeroFeature {
  /** Stable identifier (used as a React key) */
  id: string;
  /** Display title */
  title: string;
  /** Short description for the welcome card */
  description: string;
  /** Icon identifier (see the iconMap in WelcomeStep) */
  icon: string;
}

const HERO_FEATURES: HeroFeature[] = [
  {
    id: 'account-sync',
    title: 'Account Sync',
    description: 'Your trades sync automatically from OANDA with real-time updates',
    icon: 'sync',
  },
  {
    id: 'trade-history',
    title: 'Trade History',
    description: 'Complete record of all your trades with P&L, dates, and prices',
    icon: 'layers',
  },
  {
    id: 'open-positions',
    title: 'Open Positions',
    description: 'Monitor your current positions with live P&L updates',
    icon: 'trending',
  },
];

/** Top N feature highlights for the onboarding welcome step. */
export const heroFeatures = (count = 3): HeroFeature[] =>
  HERO_FEATURES.slice(0, count);
