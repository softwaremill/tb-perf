---
description: Address all //FIX comments in modified files
---

# Fix Comments Task

Find and address all `//FIX:` comments in the codebase.

## Process

1. Search for all `//FIX:` comments in modified or staged files (use `git status` to find them)
2. For each FIX comment found:
   - Read the surrounding code context
   - Either:
     - **Fix it**: Modify the code to address the issue, then remove the `//FIX:` comment
     - **Ask for clarification**: If the fix requires user input or a design decision, add a `//REPLY:` comment on the next line with your question, and keep the `//FIX:` comment
3. After processing all comments, provide a summary

## Output Format

At the end, provide a summary like:

```
## FIX Summary

**Fixed (N):**
- file.rs:42 - [brief description of what was fixed]
- file.rs:87 - [brief description]

**Needs Clarification (M):**
- file.rs:123 - [question asked in //REPLY comment]

**Total: N fixed, M need clarification**
```

## Notes

- Be conservative: if unsure, ask rather than make assumptions
- Keep fixes minimal and focused on what the FIX comment asks
- When adding //REPLY:, phrase it as a clear question
