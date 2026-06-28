# ACP MotionLoom Catalog

Use this catalog when users ask MotionLoom-specific questions (DSL syntax, kernels, params, unified graph syntax, or effect writing guidance).
For current parser-safe authoring rules, open `ML-0005.md` first.
For failed MotionLoom generation, validation errors, or safe copy-paste starting points, open `ML-0006.md`.

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
| ML-0005 | Current MotionLoom authoring rules | User asks for latest/current MotionLoom rules, parser-safe DSL constraints, current effect IDs, kernel mapping, `<Process>` behavior, or hard do-not-generate rules. | `latest`, `current`, `rules`, `parser-safe`, `present`, `process`, `effect map`, `kernel map`, `do not generate`, `camera track` | `anica/docs/acp/motionloom/ML-0005.md` |
| ML-0006 | Error repair guide and golden valid templates | User reports MotionLoom parse/compile/render errors, broken generated DSL, missing gradients, invalid paint, invalid scene hierarchy, or asks for a safe template. | `error`, `repair`, `fix`, `parse error`, `compile error`, `render error`, `gradient reference not found`, `invalid scene paint`, `golden template`, `safe template`, `Scene root only accepts`, `url(#` | `anica/docs/acp/motionloom/ML-0006.md` |
| ML-0007 | Scene control, rigging, deformation, and editor keyframes | User asks about AE/DaVinci-style keyframes, `AnimationTarget`, `Key`, puppet pins, mesh topology, IK/FK, bones, character images, `Path d`, `Layer3D`, mask follow, or `FaceJaw`. | `AnimationTarget`, `Key`, `Puppet`, `Pin`, `MeshTopology`, `Vertex`, `Triangle`, `IK`, `FK`, `Skeleton`, `Bone`, `Character src`, `base64`, `Path d`, `Layer3D`, `FaceJaw`, `mask follow`, `topology`, `pin x/y` | `anica/docs/acp/motionloom/ML-0007.md` |
| Scene UI Schema | Machine-readable Scene Camera and Layer3D property metadata | UI/editor work needs property groups, labels, property types, animatability flags, or Camera/Layer3D naming rules. | `ui schema`, `property panel`, `Scene Camera`, `Layer3D`, `2.5D Layer`, `animatable`, `editor group`, `camera labels` | `anica/docs/acp/motionloom/scene-ui-schema.json` |

## Authoring Rules
- Keep one topic per `ML-xxxx.md`.
- Keep examples copy-pasteable.
- Mark unsupported/removed kernels explicitly.
- Do not create another `index.md` in this folder; use `catalog.md`.
