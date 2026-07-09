# PR Review Command

Review pull request with optional additional instructions.

**Arguments:** `<PR number> [additional instructions]`

Examples:
- `/review-pr 123` - Standard review
- `/review-pr 123 focus on the SQL changes` - Review with specific focus
- `/review-pr 123 this fixes a critical prod bug, check for regressions` - Review with context

**Parsed from `$ARGUMENTS`:**
- PR number: Extract the first number from the arguments
- Additional instructions: Everything after the PR number (if any)

---

**Input:** $ARGUMENTS

## Instructions

1. **Fetch the PR** using GitHub MCP tools:
   - Use `mcp__github__get_pull_request` to get PR details
   - Use `mcp__github__get_pull_request_files` to see changed files
   - Use `mcp__github__get_file_contents` to read the actual changes

2. **Read the review checklist** from `docs/pr-review-guidelines.md` (use the Read tool)
   - This file contains the authoritative, up-to-date checklist
   - Guidelines may have been updated since this command was written

3. **CRITICAL: Schema Sync Check**
   If ANY schema changes are made, verify ALL locations are updated:
   - `shared/schema.ts` - Source of truth
   - `queries-service/schema.ts` - Must be manually synced
   - `src-tauri/src/db.rs` - PostgreSQL migrations if adding DB columns

   **Flag as blocking if schema locations are out of sync!**

4. **CRITICAL: Backward Compatibility Check**
   The desktop app connects to Railway backends. Old app versions may still be running.
   - New API endpoints: OK (old apps won't call them)
   - Changed API endpoints: Do old apps still work?
   - New Zero columns: Are they nullable or have defaults?
   - Removed features: Will old apps crash or degrade gracefully?

   **Flag as blocking if changes break existing app versions!**

5. **CRITICAL: AI/MCP Security Check** (if PR touches AI code)
   Files to watch: `**/ai/**`, `**/mcp-server-rs/**`, `**/classifier.ts`, `**/anthropic-proxy.ts`, `**/chat.rs`

   Check for (see `docs/engineering-principles.md` Sections 7-13):
   - System prompts accepted from client requests (BLOCK - must be server-side only)
   - User input passed to LLM without sanitization (BLOCK)
   - MCP tool outputs containing user content without sanitization (BLOCK)
   - Internal error details exposed to clients (BLOCK)
   - Sensitive data logged (strategy logic, credentials, full content) (BLOCK)
   - Missing resource limits on user input (flag)

   **Flag as blocking if AI security patterns are violated!**

6. **Review each changed file** for:
   - Engineering principles violations
   - Type safety issues (Decimal vs f64, atomic state, etc.)
   - Database/schema changes done correctly
   - Feature gating if new features
   - Security concerns (general + AI-specific if applicable)
   - UI/UX consistency

7. **Identify deployment impact**:
   - Which services are affected (Tauri app, queries-service, zero-cache, web)
   - Required deployment ORDER (usually: backend first, then app)
   - Suggest appropriate labels

8. **Output format** (from the guidelines - keep it short, focus on problems):
   - **✅ Verified**: What you actually checked (compiled, ran tests). If not verified, say so.
   - **🛑 Blocking Issues**: Problems that MUST be fixed. File:line + what's wrong. "None" if none.
   - **Labels**: Only mention if wrong or missing. Use colored circles (🟠`build:staging` 🟣`deploy:*` 🩶`bump:patch`)
   - **⚠️ Non-blocking**: Optional, only if you have specific actionable suggestions
   - Do NOT include: summaries, praise, elaborate checklists, deployment order unless there's a concern

9. **Post the review** using `mcp__github__add_issue_comment`:
   - Use owner=MicroMediaSites, repo=fx-tracker, issue_number=<PR number>
   - Format the review as markdown per the guidelines
   - Do NOT ask for confirmation - submit the comment immediately
   - Note: Use `add_issue_comment` instead of `create_pull_request_review` because the latter fails when reviewing your own PRs

## Notes

- **Parse arguments**: Extract the PR number (first number in args). Everything after is additional instructions.
- If given a full URL, parse out the PR number
- Read the full diff, not just file names
- Be thorough but concise
- Schema sync, backward compatibility, and AI security are the most common sources of production issues
- **If additional instructions provided**: Apply them as extra focus areas or context. Mention in the review if they affected your assessment.
