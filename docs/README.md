# wickd Documentation

## Core Documentation

| Document | Description |
|----------|-------------|
| [architecture.md](architecture.md) | System design, data flow, and component overview (HISTORICAL — predates the local-first conversion) |
| [local-store.md](local-store.md) | The local SQLite store: schema, migrations, layers |
| [strategy-store.md](strategy-store.md) | Strategy store layout |

## Integration Guides

| Document | Description |
|----------|-------------|
| [tauri-guide.md](tauri-guide.md) | Tauri 2 patterns, localhost plugin, events |
| [oanda-api-reference.md](oanda-api-reference.md) | OANDA API integration reference |

## Operations

| Document | Description |
|----------|-------------|
| [staging-setup.md](staging-setup.md) | Staging environment configuration |
| [auto-update-verification.md](auto-update-verification.md) | Post-release auto-update verification harness (`scripts/verify-auto-update.sh`) |

## Audits

Technical debt and refactoring documentation.

| Document | Description |
|----------|-------------|
| [codebase-refactoring-plan.md](audits/codebase-refactoring-plan.md) | Multi-phase refactoring roadmap |
| [docs-cleanup-plan.md](audits/docs-cleanup-plan.md) | Documentation consolidation plan |
| [engineering-principles-audit.md](audits/engineering-principles-audit.md) | Codebase compliance audit |
| [rust-unwrap-audit.md](audits/rust-unwrap-audit.md) | Unwrap/expect usage audit |

## Plans

Active feature and implementation plans.

| Document | Description |
|----------|-------------|
| [candlesight_backtesting_methodologies.md](plans/candlesight_backtesting_methodologies.md) | Backtesting engine design |
| [candlesight_price_action_detection_plan.md](plans/candlesight_price_action_detection_plan.md) | Price action pattern detection |
| [candlesight_strategy_builder_redesign.md](plans/candlesight_strategy_builder_redesign.md) | Strategy builder V3 design |
| [chatgpt-app-store-guide.md](plans/chatgpt-app-store-guide.md) | App Store submission guidance |
| [regulatory-compliance-implementation.md](plans/regulatory-compliance-implementation.md) | Compliance implementation details |
| [streaming-optimizations.md](plans/streaming-optimizations.md) | Price streaming optimization |
| [telemetry-implementation.md](plans/telemetry-implementation.md) | Telemetry system design |
| [walk_forward_window_drilldown.md](plans/walk_forward_window_drilldown.md) | Walk-forward analysis UI |
