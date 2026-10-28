# Whitespace cell report

## Changed path

- `studio/src/components/shell/AppShell.tsx`

## Change

Removed the single extra blank line at EOF (file ended with `}\n\n`, now ends with `}\n`). Exactly one final newline is retained; no content or behavior edits. The pre-existing dirty tree (ongoing redesign) was left untouched.

## Verification

`git diff --check` — clean, no output, exit code 0.

Before:

```
studio/src/components/shell/AppShell.tsx:166: new blank line at EOF.
```

After: no whitespace errors reported for any path.
