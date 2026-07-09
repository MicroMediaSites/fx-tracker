/**
 * Security messaging for wickd.
 * Shown in onboarding (moved in-app from the deleted @candlesight/content
 * package when the marketing site was removed, AGT-653).
 */

export interface TrustPoint {
  id: string;
  title: string;
  description: string;
  icon: 'lock' | 'eye-off' | 'shield' | 'key' | 'server';
}

export interface TechnicalSpec {
  title: string;
  content: string;
}

export interface SecurityFAQ {
  question: string;
  answer: string;
}

export interface SecurityMessaging {
  headline: string;
  tagline: string;
  trustPoints: TrustPoint[];
  technicalSpecs: TechnicalSpec[];
  faq: SecurityFAQ[];
  whatSyncs: {
    synced: string[];
    neverSynced: string[];
  };
}

export const securityMessaging: SecurityMessaging = {
  headline: 'Your Keys, Your Control',
  tagline: 'Your API keys are encrypted on your device and never leave your computer. We can\'t see them, and neither can anyone else.',

  trustPoints: [
    {
      id: 'local-encryption',
      title: 'Local-First Encryption',
      description: 'Encrypted on your device before anything syncs',
      icon: 'lock',
    },
    {
      id: 'zero-knowledge',
      title: 'Zero-Knowledge Design',
      description: 'We can\'t see your credentials - ever',
      icon: 'eye-off',
    },
    {
      id: 'rust-safety',
      title: 'Built with Rust',
      description: 'Memory-safe code protects sensitive data',
      icon: 'shield',
    },
  ],

  technicalSpecs: [
    {
      title: 'Encryption',
      content: 'AES-256-GCM with HMAC-SHA256 tamper detection',
    },
    {
      title: 'Key Derivation',
      content: 'Argon2id with 128MB memory cost',
    },
    {
      title: 'Memory Safety',
      content: 'Keys zeroized from memory after use (Rust secrecy crate)',
    },
    {
      title: 'Rate Limiting',
      content: '5 failed attempts = 5 minute lockout',
    },
    {
      title: 'No Server Storage',
      content: 'Master password never transmitted. Only encrypted blobs sync.',
    },
  ],

  whatSyncs: {
    synced: [
      'Encrypted API key blob (unreadable without your password)',
      'Account IDs (not sensitive)',
      'Trade history and notes',
      'Strategies and backtest results',
      'App settings',
    ],
    neverSynced: [
      'Master password (never transmitted)',
      'Decrypted API key (never transmitted)',
    ],
  },

  faq: [
    {
      question: 'What if I forget my master password?',
      answer: 'There is no recovery option. You\'ll need to re-enter your OANDA credentials. This is by design - it means even we can\'t access your keys.',
    },
    {
      question: 'Is wickd open source?',
      answer: 'The core application is not currently open source, but our security architecture is documented and we welcome security researchers to review our approach.',
    },
    {
      question: 'Has wickd been audited?',
      answer: 'We have completed an internal security assessment. Third-party audits are planned for future versions.',
    },
    {
      question: 'Where is my data stored?',
      answer: 'Your encrypted credentials are stored locally on your device. Trade history and strategies sync to our secure cloud infrastructure hosted in the US.',
    },
    {
      question: 'Can wickd access my OANDA account?',
      answer: 'wickd requires full API access to sync your trades and execute orders through the trading ticket. Your credentials are encrypted locally and never transmitted to our servers.',
    },
  ],
};

/** Short security message for compact displays */
export const securityShort = securityMessaging.tagline;

/** Get trust points for display */
export const getTrustPoints = (): TrustPoint[] => securityMessaging.trustPoints;

/** Get technical specs for expandable sections */
export const getTechnicalSpecs = (): TechnicalSpec[] => securityMessaging.technicalSpecs;

/** Get FAQ items */
export const getSecurityFAQ = (): SecurityFAQ[] => securityMessaging.faq;
