# Staging Environment Setup

> **PARTIALLY HISTORICAL:** the Clerk/Zero/queries-service staging pieces
> described below were removed in the local-first conversion (AGT-650..653).
> Staging desktop builds still exist (`.github/workflows/staging.yml`), but
> there are no staging cloud services anymore.

This guide covers setting up a staging environment for wickd that mirrors production behavior while using isolated Railway services.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      Shared Services                             │
│  ├── Clerk (adapted-raptor-2.clerk.accounts.dev)                │
│  ├── OANDA Practice Account (101-001-00000000-001)              │
│  └── Anthropic API                                               │
└─────────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┴─────────────────────┐
        ▼                                           ▼
┌───────────────────────┐               ┌───────────────────────┐
│   Staging (Railway)   │               │  Production (Railway) │
├───────────────────────┤               ├───────────────────────┤
│  PostgreSQL           │               │  PostgreSQL           │
│  zero-cache           │               │  zero-cache           │
│  queries-service      │               │  queries-service      │
└───────────────────────┘               └───────────────────────┘
```

## Railway Staging Setup

### Step 1: Create Staging Railway Project

1. Go to [Railway Dashboard](https://railway.app/dashboard)
2. Click "New Project" → "Empty Project"
3. Name it `candlesight-staging` (or similar)

### Step 2: Add PostgreSQL Database

1. In the staging project, click "New" → "Database" → "PostgreSQL"
2. Wait for provisioning to complete
3. Go to the database settings → Variables tab
4. Copy the `DATABASE_URL` connection string (format: `postgresql://postgres:PASSWORD@HOST:PORT/railway`)
5. **Important**: Enable logical replication for Zero:
   ```sql
   -- Connect to the database and run:
   ALTER SYSTEM SET wal_level = 'logical';
   -- Then restart the database from Railway dashboard
   ```

### Step 3: Deploy zero-cache Service

1. Click "New" → "GitHub Repo" → Select `fx-tracker` repository
2. Configure the service:
   - **Name**: `staging-zero-cache`
   - **Root Directory**: `/` (root of repo)
   - **Dockerfile Path**: `Dockerfile` (uses the existing Dockerfile)
3. Add environment variables:
   ```
   ZERO_UPSTREAM_DB=<staging PostgreSQL connection string>
   ```
4. Note the generated URL (e.g., `https://staging-zero-cache-XXXX.up.railway.app`)

### Step 4: Deploy queries-service

1. Click "New" → "GitHub Repo" → Select `fx-tracker` repository
2. Configure the service:
   - **Name**: `staging-queries-service`
   - **Root Directory**: `queries-service`
   - **Build Command**: `npm install && npm run build`
   - **Start Command**: `npm start`
3. Add environment variables:
   ```
   ZERO_UPSTREAM_DB=<staging PostgreSQL connection string>
   NODE_ENV=production
   ZERO_CACHE_URL=<staging zero-cache URL from Step 3>
   ```
4. Note the generated URL (e.g., `https://staging-queries-service-XXXX.up.railway.app`)

### Step 5: Update Local Environment Files

Update the placeholder values in your staging environment files:

**`.env.staging`**:
```env
ZERO_UPSTREAM_DB=postgresql://postgres:PASSWORD@HOST:PORT/railway
ZERO_MUTATE_URL=https://staging-queries-service-XXXX.up.railway.app/push
ZERO_GET_QUERIES_URL=https://staging-queries-service-XXXX.up.railway.app/get-queries
ZERO_AUTH_JWKS_URL=https://adapted-raptor-2.clerk.accounts.dev/.well-known/jwks.json
```

**`.env.staging.local`**:
```env
VITE_CLERK_PUBLISHABLE_KEY=pk_test_YWRhcHRlZC1yYXB0b3ItMi5jbGVyay5hY2NvdW50cy5kZXYk
VITE_ZERO_SERVER=https://staging-zero-cache-XXXX.up.railway.app
```

**`src-tauri/.env.staging`**:
```env
DATABASE_URL=postgresql://postgres:PASSWORD@HOST:PORT/railway
# OANDA and Anthropic remain unchanged (shared with production)
```

## Building for Staging

Once environment files are configured:

```bash
./build-staging.sh
```

This will:
1. Validate staging environment files exist
2. Check for placeholder values
3. Build the Tauri app with staging configuration
4. Restore dev environment after build

## Environment Variable Summary

| Variable | Staging | Production | Shared? |
|----------|---------|------------|---------|
| `ZERO_UPSTREAM_DB` | Staging DB | Prod DB | No |
| `DATABASE_URL` | Staging DB | Prod DB | No |
| `VITE_ZERO_SERVER` | Staging zero-cache | Prod zero-cache | No |
| `ZERO_MUTATE_URL` | Staging queries | Prod queries | No |
| `ZERO_GET_QUERIES_URL` | Staging queries | Prod queries | No |
| `VITE_CLERK_PUBLISHABLE_KEY` | Same | Same | Yes |
| `ZERO_AUTH_JWKS_URL` | Same | Same | Yes |
| `OANDA_API_KEY` | Same | Same | Yes |
| `OANDA_ACCOUNT_ID` | Same | Same | Yes |
| `ANTHROPIC_API_KEY` | Same | Same | Yes |
| `NODE_ENV` | `production` | `production` | Yes |

## Railway Service Environment Variables

### zero-cache (staging)
```
ZERO_UPSTREAM_DB=<staging PostgreSQL>
```

### queries-service (staging)
```
ZERO_UPSTREAM_DB=<staging PostgreSQL>
NODE_ENV=production
ZERO_CACHE_URL=<staging zero-cache URL>
```

## Verifying Staging Deployment

1. **Health Checks**:
   - zero-cache: `curl https://staging-zero-cache-XXXX.up.railway.app/health`
   - queries-service: `curl https://staging-queries-service-XXXX.up.railway.app/health`

2. **Test Sync**:
   - Build staging app
   - Sign in with Clerk
   - Verify trades sync from OANDA
   - Check Zero sync is working (trades appear in UI)

3. **Test Mutations**:
   - Create a note on a trade
   - Verify it persists after app restart

## Rollback Procedure

If staging reveals issues before production deployment:

1. Staging changes do not affect production
2. Fix issues in development
3. Re-deploy to staging
4. Verify fix
5. Then deploy to production

## Notes

- Staging and production share the same Clerk instance, so users can sign in with the same credentials
- OANDA practice account is shared, so trade history is the same across environments
- Only the database and sync services are isolated between staging and production
- `NODE_ENV=production` ensures staging behaves exactly like production at the app layer
