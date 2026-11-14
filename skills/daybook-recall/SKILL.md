---
name: daybook-recall
description: Recall prior Daybook sessions for the current project when investigating a recurring problem, past decision, or earlier implementation. Use instead of guessing archive dates.
---

# Daybook Recall

1. Identify the current project name from its repository root.
2. Run `daybook query --project <name> --search "<important words>" --limit 5`.
3. Read the returned summary and keep its `ledger://<date>/<machine>/<session-id>` source reference with any claim you reuse.
4. If the search is too narrow, retry with fewer words. Use `daybook query YYYY-MM-DD` only when you already know the date.

Do not describe a summary as verified current behavior. Check the source code or rerun the relevant test before relying on it.
