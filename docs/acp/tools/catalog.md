# ACP Tools Catalog

Use this catalog when users ask what ACP can call, how to call it, or why a tool call failed.

## Routing Rules
1. Read this file first.
2. Pick the best matching tool group.
3. Open only that `TOOL-xxxx.md`.
4. If still unclear, ask one short clarifying question.

## Tool Groups

| ID | Group | When to open | Keywords | File |
|---|---|---|---|---|
| TOOL-0001 | Docs tools | User asks where docs are, list/read docs, or docs path errors. | `docs`, `catalog`, `list_files`, `read_file`, `file not found`, `subdir` | `anica/docs/acp/tools/TOOL-0001.md` |
| TOOL-0002 | Media pool tools | User asks clip metadata, codec/fps/resolution, counts, media listing, or remove/clear media pool entries. | `media pool`, `metadata`, `codec`, `fps`, `resolution`, `duration`, `remove`, `clear all` | `anica/docs/acp/tools/TOOL-0002.md` |
| TOOL-0003 | Timeline analysis/edit-plan tools | User asks timeline snapshot, silence/subtitle gaps, or any validate/apply edit operation. | `timeline`, `silence`, `subtitle`, `audio_silence_cut_plan`, `subtitle_gap_cut_plan`, `validate`, `apply`, `track_ops`, `clip_ops`, `subtitle_ops`, `effect_ops`, `transition_ops` | `anica/docs/acp/tools/TOOL-0003.md` |
| TOOL-0004 | Export run tool | User asks to actually run export/render via ACP. | `export`, `run`, `render`, `smart_universal`, `keep_source_copy`, `preset_reencode` | `anica/docs/acp/tools/TOOL-0004.md` |

## Authoring Rules
- Keep one tool group per file.
- Add new group rows here before referencing new docs.
- Do not create `index.md` in this folder; use `catalog.md`.
