# ACP Limitation Catalog

Use this catalog to find known limitations quickly.

## How To Read
- `When to open`: human-friendly trigger sentence.
- `Keywords`: machine-friendly retrieval hints.
- `File`: the exact limitation document.

## Limitation List

| ID | Title | When to open | Keywords | File |
|---|---|---|---|---|
| LIM-0001 | macOS preview color shift at dissolve/opacity boundaries | User reports a brief color flash only at fade/dissolve edges in preview. | `nv12`, `bgra`, `dissolve`, `fade`, `opacity`, `color shift`, `flash frame`, `preview mismatch`, `macos` | `anica/docs/acp/limitation/LIM-0001.md` |
| LIM-0002 | 8K export may not use VideoToolbox GPU path | User asks why 8K export does not stay on VideoToolbox and switches to CPU fallback. | `8k`, `7680x4320`, `videotoolbox`, `gpu export`, `compression session`, `fallback`, `cpu`, `macos` | `anica/docs/acp/limitation/LIM-0002.md` |

## Authoring Rules
- Keep each limitation in its own file: `LIM-xxxx.md`.
- Keep `Title` and `When to open` human-readable.
- Keep `Keywords` compact and lowercase for retrieval.
- Always include an `ACP Reply (Short)` block in each limitation file.
- Do not create another `index.md` in this folder; keep the router filename as `catalog.md`.
