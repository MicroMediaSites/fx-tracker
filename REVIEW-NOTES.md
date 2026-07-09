# REVIEW NOTES — AGT-653: de-SaaS cleanup — delete billing, entitlements, marketing, Clerk shell

Branch: `de-saas` (from main at 591975f, after AGT-650/651/652).

This is the FINAL code ticket of the CandleSight→wickd conversion. Everything
here is deletion of dead SaaS surface; no behavior is added.

## What changed

### AC1 — Stripe/billing, entitlements, marketing site: deleted

- **`web/` deleted wholesale** (Next.js marketing + billing site: Stripe
  checkout/portal/subscription API routes, buy/download/beta funnel, pricing
  pages, dashboard, sign-in/up, desktop SSO callback) along with
  `netlify.toml` and the `web` npm workspace. Nothing in the app referenced
  it at runtime — only `VITE_WEB_URL` links (removed below).
- **`packages/content` deleted** (the `@candlesight/content`
  tier/feature/pricing definitions). The only non-entitlement content the app
  still used — onboarding security messaging and the three welcome-step
  feature highlights — moved in-app to `src/content/security.ts` (verbatim)
  and `src/content/onboarding.ts` (minimal `heroFeatures`, tier machinery
  stripped). Workspaces removed from `package.json`; `build:content` gone
  from build/check scripts and `pr.yml`; lockfile regenerated (no
  clerk/stripe/next entries remain).
- **Entitlement UI deleted**: `FeatureGate`, `PricingModal`,
  `TokenTopUpModal`, `UpgradePrompt`, `PricingModalContext`,
  `useEntitlements` (the AGT-650 static shim). Consumers un-gated:
  - `IndicatorMenu`: premium banner/lock icons gone, all indicators addable.
  - `MethodologySelector` + `StrategyListPanel` + `BacktestApp`: lock icons,
    tier badges, `onUpgradeNeeded` plumbing and the `FeatureGate` wrappers
    removed; `METHODOLOGY_FEATURE_IDS` deleted from `types/strategy.ts`.
  - `BacktestResultsPanel`: analytics section unwrapped (was blur-if-locked).
  - `TerminalOverlay`: `/buy` command, trial-upgrade notice, token-top-up
    CTA and quota error special-casing removed.
  - `Toggle`: unused `locked`/`lockedTooltip`/`onLockedClick` mechanism
    removed (no remaining callers).
- **`WEB_URL`/`VITE_WEB_URL` removed everywhere** (`src/lib/config.ts`
  deleted; `AddLiveCredentialsModal` security-page link replaced with inline
  text; `vite-env.d.ts`, `playwright.config.ts`, `release.yml`,
  `staging.yml` cleaned). `ConfigError.tsx` deleted (zero consumers).
- **CI**: `.github/workflows/deploy.yml` deleted (only web deploy jobs
  remained after AGT-649/650). `.virgil.yml` web-marketing +
  membership-payments domains and the `deploy:web` label rule removed.

### AC2 — Clerk shell: gone; no auth surface beyond the local vault

- The `?window=login` shell and desktop OAuth were already deleted
  (AGT-650/652); this ticket removes the last Clerk remnants: the web Clerk
  surface (with `web/`), Clerk/Railway domains in the **tauri.conf.json CSP**
  (now localhost + OANDA only), `DESKTOP_AUTH_WEB_URL`/`CLERK_*` env plumbing
  in `release.yml`/`staging.yml` (no Rust consumer existed), and
  `.agent/auth-security/` + `.agent/membership-payments/` +
  `.agent/web-marketing/` (all describe deleted code; the credential vault
  now lives in `crates/wickd-core/src/crypto/` and is unchanged).
- Stale-doc cleanup tied to the above: CLAUDE.md replaced (it described the
  dead Zero/Clerk orchestrator architecture); `docs/architecture.md` and
  `docs/staging-setup.md` marked HISTORICAL; `docs/local-store.md` /
  `docs/tauri-guide.md` / `docs/README.md` corrected;
  `docs/user-guide.yaml` deleted (unconsumed CandleSight support content,
  tier-gated FAQs, describes deleted windows).

### AC3 — Railway project deletion: NOT in this diff (human-gated)

Per the ticket guard: Matt's git-origin remap + final archive freshness check
+ live confirmation are hard pre-conditions. HUMAN-ITEMS.md item 7 tracks it.
This ticket adds one new Inbox item: delete the now-dead GitHub Actions
variables (`VITE_WEB_URL`, `DESKTOP_AUTH_WEB_URL`, `CLERK_*`) and the
`deploy:web` label.

### AC4 — offline regression: green, evidence attached

- Full suite: `npm run build`, `npm test` (Rust + UI), and
  `CI=1 npm run test:e2e` — **all 66 e2e pass**, including the offline-boot
  specs (zero non-localhost requests).
- New spec `e2e/tests/agt-653-de-saas.spec.ts` proves the post-teardown
  surface and captures the evidence:
  - `review-evidence/AGT-653-offline-boot-de-saas.png` — default window
    cold-boots offline, no sign-in/upgrade/subscription surface, zero
    external requests.
  - `review-evidence/AGT-653-methodology-ungated.png` — methodology dropdown
    with no lock icons/tier badges/upgrade prompts (only "coming soon" for
    unimplemented methodologies).

## Reviewer notes

- Historical comments referencing Clerk/Zero/queries-service in spec headers
  and Rust module docs (`chat.rs`, `main.rs`, `strategy_convert/convert.rs`,
  AGT-650 hook comments) are kept deliberately — they explain why cloud
  paths are absent.
- `tauri-plugin-localhost` stays: originally Clerk-motivated, but window
  URLs/CSP/capabilities are built around the localhost origin (noted in
  `docs/tauri-guide.md`).
- Wholesale-deleted paths are marked `-diff` in `.gitattributes` for this
  review only (per the established mega-teardown review protocol).
