---
description: Track security and quality audit items
argument-hint: [description] OR [--flag description] OR [--resolve auditId]
---

Manage the audit tracker at `docs/audits.md`.

**Input:** $ARGUMENTS

Parse the input and perform one of these actions:

## If input starts with `--resolve`:
- Extract the audit ID (e.g., `--resolve AUDIT-003`)
- Find that item in `docs/audits.md`
- Change its status from `Open` to `Resolved`
- Add resolution date

## Otherwise, it's a new audit item:
- Check for an optional category flag (e.g., `--security`, `--performance`, `--accessibility`, `--code-quality`)
- If no flag is provided, categorize under **General**
- The rest is the audit item description
- Generate a sequential audit ID (AUDIT-001, AUDIT-002, etc.) by checking existing IDs
- Add to the appropriate category section in the doc

## Audit file format:

```md
# Audit Tracker

## General
- **AUDIT-001** | Open | 2025-12-16 | General quality item

## Security
- **AUDIT-002** | Open | 2025-12-16 | Security concern

## Performance
- **AUDIT-003** | Resolved (2025-12-17) | 2025-12-16 | Fixed perf issue

## Accessibility
- **AUDIT-004** | Open | 2025-12-16 | A11y improvement needed

## Code Quality
- **AUDIT-005** | Open | 2025-12-16 | Code smell or tech debt
```

Create the file and any missing category sections as needed. After adding an item, show the audit ID. After resolving, confirm the resolution.
