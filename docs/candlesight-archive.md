# CandleSight data archive

During the CandleSight -> wickd conversion (vault ticket AGT-636, 2026-07-06),
every CandleSight-era data store was snapshotted into a single local archive
directory so nothing collides with fresh wickd data and everything stays
recoverable.

**Archive location:** `~/Documents/candlesight-archive/` (Matt's machine).
A `MANIFEST.md` inside the archive lists every artifact with origin, size, and
SHA-256 checksum — verify checksums before trusting any restore.

## Archived artifacts

| Artifact | What it is |
|---|---|
| `candlesight-prod-2026-07-06.dump` | Full `pg_dump -Fc` of the Railway prod Postgres (db `railway`, PostgreSQL 17.10). 51 tables, 275 TOC entries, ~15 MB. Integrity-verified with `pg_restore --list` on 2026-07-06. |
| `candlesight-dev-postgres-pgdata-2026-07-06.tar.gz` | Raw PostgreSQL 16 data directory from the local dev Docker volume `fx-tracker_fx_tracker_data` (the `fx-tracker-postgres` container defined in `docker-compose.yml`). ~21 MB compressed. |
| `app-local/com.candlesight.app/` | The Tauri app's local data dir from `~/Library/Application Support/com.candlesight.app/`: `device_id`, `mock_price_cache.json`, `rate_limit.json`, `.window-state.json`. |

## Not archived (and why)

- **Zero replicas** — the `zero-cache` SQLite replica is ephemeral derived
  state; zero-cache rebuilds it from upstream Postgres on next startup. No
  replica files existed on disk at archive time.
- **`com.fx-tracker.app` app dir** — contains only a `.env` secrets file
  (credentials, not data); excluded by policy.
- **Railway staging** — staging environment and services were deleted on
  2026-07-06 after prod was dumped and verified; nothing remained to archive.

## Restore procedures

### Prod dump (logical, portable)

Requires PostgreSQL **16+ client tools** — pg 15's `pg_restore` fails with
`unsupported version (1.16) in file header`.

```sh
# Inspect without restoring
/opt/homebrew/opt/postgresql@17/bin/pg_restore --list \
  ~/Documents/candlesight-archive/candlesight-prod-2026-07-06.dump

# Restore into a fresh database
createdb candlesight_restored
pg_restore --no-owner --no-privileges -d candlesight_restored \
  ~/Documents/candlesight-archive/candlesight-prod-2026-07-06.dump
```

### Dev pgdata (raw cluster, version-locked to PostgreSQL 16)

The tarball is a physical data directory — it must be started with PostgreSQL
**16** binaries (not 15, not 17). Easiest via Docker:

```sh
cd ~/Documents/candlesight-archive
mkdir -p /tmp/cs-dev-restore
tar xzf candlesight-dev-postgres-pgdata-2026-07-06.tar.gz -C /tmp/cs-dev-restore
docker run -d --name cs-dev-restore -p 5433:5432 \
  -v /tmp/cs-dev-restore/dev-postgres-pgdata:/var/lib/postgresql/data \
  postgres:16
psql -h localhost -p 5433 -U postgres   # password: postgres (dev compose default)
```

Once started, take a logical `pg_dump` if you need something portable across
Postgres versions.

### App-local files

Copy the files back into `~/Library/Application Support/com.candlesight.app/`
(or the wickd equivalent dir) as needed. They are plain JSON/text caches; the
app also regenerates them from scratch if absent.

## Importing into the wickd local store (AGT-648)

The archive's app datasets can be restored into the wickd local store
(`~/.wickd/app.db`) with the repo's import CLI — this is the sanctioned way to
"restore into a new, wickd-namespaced store" (see the namespacing rule below):

```sh
cd src-tauri
cargo run --bin import_candlesight -- --dry-run   # preview (writes nothing)
cargo run --bin import_candlesight                # import
cargo run --bin import_candlesight -- --status    # per-dataset imported counts
cargo run --bin import_candlesight -- --list      # imported strategies
```

Semantics (implementation: `src-tauri/src/local_store/import.rs`):

- **Provenance-tagged** — every imported row is written with
  `source = 'candlesight'` (local-store schema v5 adds a `source` column to
  all dataset tables; native wickd rows carry `''`). The local window badges
  imported strategies and can filter by source; the CLI's `--status`/`--list`
  read the same tag.
- **Idempotent, never clobbers** — rows are written with `INSERT OR IGNORE`
  in one transaction: re-running the import inserts nothing new, and a row
  that already exists locally (e.g. a trade wickd has since re-synced from
  OANDA) always wins over the archive copy.
- **Single-user** — the local store has no `user_id`, so only one CandleSight
  user's rows are imported per run. Pass the required `--user <clerk-user-id>`
  to select whose rows are imported (typically the primary account that owns
  the 33 recovered non-archived strategies). The dump's demo/test accounts
  should stay out of the store.
- **Trade ids normalized** — the cloud path's `userID:oandaId` composite ids
  become raw OANDA ids (`trade.id`, `trade_score.trade_id`, `note.trade_id`),
  matching the AGT-647 local-store convention.
- **In-flight jobs closed out** — `backtest_job` rows that were
  running/pending at archive time are imported as `cancelled` (with an
  explanatory `error_message`) so the UI shows no zombie jobs.
- **Datasets covered** — strategy, sr_zone, note, trade, trade_score,
  backtest, backtest_job, promotion_audit. `chart_config` is not in the dump
  (it lived in localStorage and was imported once by AGT-646); non-app tables
  in the dump (chat history, quotas, Zero bookkeeping, …) are ignored.
- Requires a PostgreSQL **16+** `pg_restore` (see the version note above);
  the CLI auto-detects the Homebrew `postgresql@17`/`@16` binaries and falls
  back to PATH, or takes `--pg-restore <path>`.

Reference run against the 2026-07-06 prod dump (primary user): 41 strategies
(33 active + 8 archived), 7 sr_zones, 1 note, 28 trades, 6 trade_scores,
0 backtests, 1148 backtest_jobs (1 in-flight job imported as cancelled),
20 promotion_audits — 1,251 rows; an immediate re-run inserts 0.

## Namespacing rule

Nothing under `~/Documents/candlesight-archive/` is written to by any live
system. All new wickd data stores live elsewhere (see the wickd architecture
docs); if a wickd component ever needs CandleSight history, restore from this
archive into a **new, wickd-namespaced** store — never point wickd at the
archive or at the old `fx-tracker_fx_tracker_data` volume directly.
