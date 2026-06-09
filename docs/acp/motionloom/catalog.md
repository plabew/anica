# ACP MotionLoom Catalog

Use this catalog when users ask MotionLoom-specific questions (DSL syntax, kernels, params, unified graph syntax, or effect writing guidance).

## Routing Rules
1. Open this file first.
2. Match by `When to open` + `Keywords`.
3. Open only one `ML-xxxx.md` unless user asks for broad comparison.
4. If the question is runtime behavior, prefer implementation truth from `crates/motionloom/src/*`.

## Document List

| ID | Title | When to open | Keywords | File |
|---|---|---|---|---|
| ML-0001 | MotionLoom DSL syntax, unified graph syntax, parameters, and authoring process | User asks how to write MotionLoom code, how `apply/duration` works, what fields are required, or how to choose effect category/kernel. | `motionloom`, `dsl`, `graph`, `pass`, `kernel`, `apply`, `duration`, `explicit`, `implicit`, `10 categories`, `7 tags`, `scene rendering`, `glb helpers` | `anica/docs/acp/motionloom/ML-0001.md` |
| ML-0002 | Natural-language scene prompt to valid MotionLoom scene DSL | User asks for "black background + text fade-in" or similar quick VFX scenes, and ACP must generate parser-safe scene DSL and trigger render actions. | `natural language`, `scene dsl`, `black background`, `fade in`, `hello world`, `raster`, `text_overlay`, `gpu render`, `timeline` | `anica/docs/acp/motionloom/ML-0002.md` |
| ML-0003 | Timeline, Track, Sequence, Layer, and Chain reference | User asks about scene structure, zDepth, layering, multi-track composition, or how to chain/sequence animations. | `timeline`, `track`, `sequence`, `layer`, `chain`, `zdepth`, `z depth`, `layering`, `animation timing`, `out=hold`, `out=hide` | `anica/docs/acp/motionloom/ML-0003.md` |
| ML-0004 | Scene + post-process composition patterns | User asks how to combine scene rendering with GPU effects, blur, opacity, color correction, or multi-pass effect chains. | `post-process`, `composition`, `blur`, `opacity`, `scene + world`, `multi-pass`, `precompose`, `tex`, `pass`, `from=scene:` | `anica/docs/acp/motionloom/ML-0004.md` |

## Authoring Rules
- Keep one topic per `ML-xxxx.md`.
- Keep examples copy-pasteable.
- Mark unsupported/removed kernels explicitly.
- Do not create another `index.md` in this folder; use `catalog.md`.
