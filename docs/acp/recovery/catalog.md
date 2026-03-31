# ACP Recovery Catalog

Use this catalog when users ask about autosave, recovery drafts, crash restore, or unsaved-project policy.

## Routing Rules
1. Open this file first.
2. Match the user question with `When to open` + `Keywords`.
3. Open only the best matching `REC-xxxx.md`.
4. If exact behavior is unknown, reply: `Not documented yet`.

## Document List

| ID | Title | When to open | Keywords | File |
|---|---|---|---|---|
| REC-0001 | Recovery draft policy, autosave snapshots, and startup prompt behavior | User asks how Anica handles unsaved changes, `.autosave`, `.recovery`, crash recovery, or why a recovery prompt appeared (or did not appear). | `recovery`, `autosave`, `draft`, `crash`, `unsaved`, `startup prompt`, `.recovery`, `.autosave`, `recover`, `discard`, `open saved` | `anica/docs/acp/recovery/REC-0001.md` |

## Authoring Rules
- Keep one recovery topic per file: `REC-xxxx.md`.
- Prefer policy wording over speculative implementation guesses.
- Include an `ACP Reply (Short)` block in each recovery file.
- Do not create `index.md` in this folder; keep the router filename as `catalog.md`.
