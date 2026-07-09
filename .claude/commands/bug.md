---
description: Log or resolve bugs in the bug tracker
argument-hint: [description] OR [--flag description] OR [--resolve bugId]
---

Manage the bug tracker at `docs/bugs.md`.

**Input:** $ARGUMENTS

Parse the input and perform one of these actions:

## If input starts with `--resolve`:
- Extract the bug ID (e.g., `--resolve BUG-003`)
- Find that bug in `docs/bugs.md`
- Change its status from `Open` to `Resolved`
- Add resolution date

## Otherwise, it's a new bug:
- Check for an optional category flag (e.g., `--staging`, `--prod`, `--dev`, `--backend`, `--frontend`)
- If no flag is provided, categorize under **General**
- The rest is the bug description
- Generate a sequential bug ID (BUG-001, BUG-002, etc.) by checking existing IDs
- Add to the appropriate category section in the doc

## Bug file format:

```md
# Bug Tracker

## General
- **BUG-001** | Open | 2025-12-16 | Affects all environments

## Staging
- **BUG-002** | Open | 2025-12-16 | Description here

## Production
- **BUG-003** | Open | 2025-12-16 | Prod issue

## Dev
- **BUG-004** | Open | 2025-12-16 | Dev issue

## Backend
- **BUG-005** | Resolved (2025-12-17) | 2025-12-16 | Fixed backend bug

## Frontend
- **BUG-006** | Open | 2025-12-16 | UI issue
```

Create the file and any missing category sections as needed. After adding a bug, show the bug ID. After resolving, confirm the resolution.
