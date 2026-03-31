# ACP Intent Phrases Catalog

Use this catalog for phrase dictionaries that route user intent in ACP.

## Routing Rules
1. Read this file first.
2. Pick the best matching intent dictionary.
3. Open only that `INTENT-xxxx.md`.
4. Keep dictionaries language-rich and concise.

## Intent Dictionaries

| ID | Intent | When to open | Keywords | File |
|---|---|---|---|---|
| INTENT-0001 | LLM similar-sentence cut | User asks to cut/remove similar or repeated subtitle/sentence content (multi-language). | `cut similar`, `duplicate subtitles`, `刪相似字句`, `刪重複字幕`, `重複字幕を削除`, `중복 자막 삭제` | `anica/docs/acp/intent-phrases/INTENT-0001.md` |

## Authoring Rules
- Keep one intent dictionary per file.
- Add new rows here before referencing new intent docs.
- Do not create `index.md` in this folder; use `catalog.md`.
