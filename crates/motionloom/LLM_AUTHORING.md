# MotionLoom LLM Authoring Guide

Use this guide when generating or editing MotionLoom DSL. Prefer valid,
predictable, editable, and renderable output over the shortest possible script.

## Choose One Graph Family

- **Scene graph**: vector graphics, text, animation, characters, cameras, masks,
  rigs, and composition.
- **Process graph**: media input, textures, compute effects, and multi-pass image
  processing.
- **World graph**: 3D/world content where the documented world components are
  required.
- Do not mix graph families unless the composition genuinely requires it and a
  documented example demonstrates the connection.

## Canonical Scene Structure

Use the complete hierarchy. Do not invent shorthand that places visual nodes
directly below `<Scene>`.

```xml
<Graph fps={30} duration="3s" size={[1920,1080]}>
  <Background color="#000000" />

  <Scene id="example_scene">
    <Timeline>
      <Track id="main" space="world" z="0">
        <Sequence from="0s" duration="3s" out="hold">
          <Layer>
            <Text id="title" value="HELLO" x="center" y="center"
                  fontSize="120" color="#ffffff" />
          </Layer>
        </Sequence>
      </Track>
    </Timeline>
  </Scene>

  <Present from="example_scene" />
</Graph>
```

`Graph -> Scene -> Timeline -> Track -> Sequence -> Layer` is the canonical
authoring grammar. Keeping one structure makes scripts easier for parsers, UI
editors, humans, and other LLMs to modify safely.

## Canonical Process Structure

```xml
<Graph fps={30} size={[800,450]} renderSize={[800,450]}>
  <Process id="brightness_process">
    <Input id="clip0" type="video" from="input:clip0" />
    <Tex id="src" fmt="rgba16f" from="clip0" />
    <Tex id="out" fmt="rgba16f" size={[800,450]} />
    <Pass id="fx_brightness" kind="compute" effect="brightness"
          in={["src"]} out={["out"]}
          params={{ brightness: "0.3" }} />
  </Process>

  <Present from="brightness_process" />
</Graph>
```

Use explicit textures for multi-pass processing:

- `rgba8`: lightweight/final color where HDR precision is unnecessary.
- `rgba16f`: HDR and intermediate color processing.
- `r16f`: one-channel masks, depth, or scalar data.

Do not silently change texture formats between connected passes.

## Background Rule

If the full-frame background is static, use `<Background color="..."/>` only.
Do not add a full-canvas `<Rect>` that duplicates the same background color.

Only use a full-canvas `<Rect>` when the background needs timeline animation,
blend mode, opacity animation, masking, clipping, or scene-local layering.

## IDs and References

- Give every animated, referenced, interactive, or UI-editable node a stable,
  unique `snake_case` ID.
- Use semantic IDs such as `right_forearm`, `send_button`, and `title_reveal`.
- Every `from`, `in`, `out`, `target`, `attachTo`, `rig`, `skeleton`, and mask
  reference must resolve.
- Never depend on generated names such as `Group#01`; they are unstable across
  edits and render backends.
- Group related artwork semantically so one transform controls the intended
  object rather than many unrelated paths.

## Animation Rules

- Use `curve(...)` for concise deterministic numeric animation.
- Curve points must contain numeric values only:
  `curve("0:0:linear, 1:100:ease_out")`.
- Keep procedural expressions such as `sin(...)` or `random(...)` outside curve
  keyframe values.
- Use `<AnimationTarget>` and `<Key>` when animation must be editable as explicit
  timeline keyframes by the UI.
- Do not drive the same node property with both `curve(...)` and
  `<AnimationTarget>`.
- Prefer `time="1.5s"` keys when timing should survive FPS changes; use
  `frame="45"` when exact frame identity is intentional.
- Do not animate string attributes such as `Text.value` or `Path.d` with numeric
  curves. Use supported path morphing, transforms, opacity, trim, or masks.
- For typing and reveal effects, prefer one complete text node revealed by a
  real mask. Avoid stacking many text snapshots with one-frame opacity swaps.

## Rigging and Deformation

- Use nested `Group` transforms for simple parent-child motion.
- Use `Skeleton`, `Bone`, and `Action` for reusable FK animation.
- Use IK for target-driven limb or joint-chain solving.
- Use `Puppet` with `Pin` and auto mesh for ordinary image deformation.
- Add `MeshTopology` only when advanced users need manual vertices, triangles,
  edges, or regions. Do not require topology in normal examples.

## Effects and Resources

- Use only documented effect names and parameters.
- Keep pass dependencies explicit through texture IDs.
- Keep `<Present ... />` as the final direct child of `<Graph>`.
- Define reusable fonts, gradients, brushes, masks, and textures in their
  documented scopes instead of duplicating them across nodes.
- Prefer existing primitives and features over approximating them with many
  unrelated nodes.

## Reliable Generation Workflow

1. Classify the request as Scene, Process, or World.
2. Find the nearest working example in `motionloom-example/core`.
3. Copy its structural skeleton, not its decorative content.
4. Add stable semantic IDs before animation or references.
5. Build the static composition first.
6. Add animation, masks, rigs, or effects one system at a time.
7. Verify all references, durations, texture formats, and presentation output.
8. Render a representative frame and test the GPU path when relevant.

## Final Checklist

- The graph uses a documented canonical hierarchy.
- All IDs are unique and all references resolve.
- No duplicate attributes exist on a node.
- Curves contain numeric keyframe values only.
- No property has two competing animation sources.
- Static backgrounds do not include duplicate full-frame rectangles.
- Texture formats and pass inputs/outputs match.
- The graph duration covers every sequence and animation.
- `<Present>` is the last direct child of `<Graph>`.
- The script has been parsed or rendered with a current MotionLoom tool.

## Sources of Truth

When guidance differs, use this order:

1. Current parser, schema, and tests.
2. This guide and `README.md`.
3. `PUBLIC_API.md` and ACP documentation.
4. Current `motionloom-example/core` examples.
5. Showcase examples for composition ideas, not minimal grammar.
