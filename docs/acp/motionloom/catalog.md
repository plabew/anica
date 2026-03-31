# ACP MotionLoom Catalog

Use this catalog when users ask MotionLoom-specific questions (DSL syntax, kernels, params, scope mapping, or effect writing guidance).

## Routing Rules
1. Open this file first.
2. Match by `When to open` + `Keywords`.
3. Open only one `ML-xxxx.md` unless user asks for broad comparison.
4. If the question is runtime behavior, prefer implementation truth from `crates/motionloom/src/*`.

## Document List

| ID | Title | When to open | Keywords | File |
|---|---|---|---|---|
| ML-0001 | MotionLoom DSL syntax, scope mapping, parameters, and authoring process | User asks how to write MotionLoom code, how `apply/duration` works, what fields are required, or how to choose effect category/kernel. | `motionloom`, `dsl`, `graph`, `pass`, `kernel`, `apply`, `duration`, `explicit`, `implicit`, `scope`, `adjustment`, `clip fusion`, `fusion comp`, `10 categories`, `7 tags` | `anica/docs/acp/motionloom/ML-0001.md` |

## Authoring Rules
- Keep one topic per `ML-xxxx.md`.
- Keep examples copy-pasteable.
- Mark unsupported/removed kernels explicitly.
- Do not create another `index.md` in this folder; use `catalog.md`.
