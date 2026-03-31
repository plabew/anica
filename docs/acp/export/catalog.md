# ACP Export Catalog

Use this catalog for all export-related user questions.

## Agent Routing Rules (Export)
1. Open this file first.
2. Match the user question with `When to open` + `Keywords`.
3. Open only the matched target file.
4. If user asks for source-like export, always mention tradeoff: compatibility vs size vs quality.
5. If exact behavior is unknown, reply: `Not documented yet`.

## Document List

| ID | Title | When to open | Keywords | File |
|---|---|---|---|---|
| EXP-0001 | Export modes, presets, and recommendation logic | User asks which export preset/mode to choose, or asks for ffmpeg export options inside Anica. | `export`, `preset`, `mode`, `smart_universal`, `keep_source_copy`, `preset_reencode`, `h264`, `hevc`, `prores`, `dnxhr`, `audio-only`, `crf`, `fps`, `bitrate` | `anica/docs/acp/export/EXP-0001.md` |

## Answering Rules
- Prefer exact preset IDs from docs (for example: `h264_mp4`, `prores_422_hq_mov`).
- Keep output in Markdown.
- Give a short recommendation first, then optional details.
- If user asks "same as source", state that exact bit-identical output may require `keep_source_copy` constraints.
- Do not invent unsupported presets or ffmpeg flags.

## Authoring Rules
- Keep one topic per file under this folder.
- Add each new file into this catalog table with `ID`, `When to open`, and `Keywords`.
- Keep keywords lowercase and compact for retrieval.
- Do not create `index.md` in this folder; keep router filename as `catalog.md`.
