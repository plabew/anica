# Anica Rust Contributor Standard

This document defines the Rust engineering standard for Anica contributors and AI agents.

The goal is not only formatting consistency. The goal is to keep logic easy to locate, safe to modify, and hard to accidentally break.

## Scope

This standard applies to:

- `src/core/`
- `src/api/`
- `src/ui/`
- `crates/ai-subtitle-engine/`
- `crates/video-engine/`
- `crates/gpui-video-renderer/`
- `crates/motionloom/`

This document complements:

- `CONTRIBUTING.md` for setup and validation commands

## Core Principles

1. Business logic must have a single clear home.
2. UI code must not become the source of truth for editor behavior.
3. Domain units should be modeled with types when confusion risk is real.
4. Errors should carry structured meaning, not only strings.
5. Contributors should be able to answer "where should this change go?" quickly.

## Layering Rules

Treat the current repository layout as if it already had strict crate boundaries.

- `src/core/` is the home for editor behavior, timeline rules, project state transforms, and reusable logic.
- `src/api/` is the home for request parsing, response shaping, tool contracts, and API-oriented orchestration.
- `src/ui/` is the home for rendering, layout, interaction wiring, and view-specific derived presentation.
- `crates/video-engine/` is the home for playback and decoding details.
- `crates/gpui-video-renderer/` and `crates/motionloom/` are the home for rendering implementation details.

Use these dependency rules:

- `core` must not depend on `ui`.
- `core` should avoid direct `gpui` dependencies unless a module is inherently UI-coupled.
- `ui` may call into `core`, but should not re-implement business rules already available there.
- `api` may orchestrate `core`, but should not become the long-term home for editor logic that belongs in `core`.

## Logic Placement Rules

Before adding code, decide which category the change belongs to:

- Timeline semantics, clip rules, edit-plan validation, subtitle rules, and export decisions go in `src/core/` or `src/api/`.
- Painting, layout, event listeners, and view-only derived values stay in `src/ui/`.
- Low-level frame upload, shader dispatch, and GPU resource management stay in renderer crates.

Do not:

- add new business rules directly inside GPUI render closures
- duplicate the same rule in `ui`, `api`, and `core`
- hide state mutations inside rendering code

Prefer:

- thin UI handlers that call clearly named methods
- pure helper functions for derived calculations
- state transitions that are centralized in one place

## Domain Types

Use the type system to prevent unit mixups where it meaningfully reduces risk.

Good candidates:

- frame counts
- sample counts
- timeline milliseconds
- clip IDs
- track indices
- decibel values
- normalized audio values

Examples:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Frames(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipId(pub u64);
```

Rules:

- Introduce newtypes at domain boundaries and error-prone unit boundaries.
- Do not wrap every primitive mechanically.
- If a primitive is easy to confuse with another primitive in the same code path, prefer a newtype.
- Prefer explicit conversion helpers instead of scattered `.0` access in high-level code.

## Typestate

Typestate is allowed, but it is a targeted tool, not a default style.

Use it only when:

- resource lifecycle order is strict
- invalid call ordering is a known bug source
- the state machine is small and obvious

Good candidates:

- encoder/export job stages
- explicit rendering command wrappers
- resource initialization flows with mandatory ordering

Do not use typestate when it makes common code paths harder to understand than the runtime checks it replaces.

## Renderer and UI Boundary

Keep low-level rendering details behind a stable boundary.

- `src/ui/` should not know backend-specific rendering details unless the API surface truly requires it.
- Prefer renderer-facing helper types or adapters over scattering GPU details across multiple views.
- Project-owned render/shader code should live in renderer-oriented crates, not inside general UI panels.

For this repository, the practical rule is:

- high-level UI talks to rendering abstractions
- low-level rendering stays inside `crates/gpui-video-renderer/` and `crates/motionloom/`

Do not edit vendored GPUI copies unless explicitly approved by the maintainer.

## State Management in GPUI

This repository uses `GlobalState` and GPUI entities as the source of truth.

Rules:

- render methods should read state and build UI
- mutation should happen through named methods, not ad hoc field edits spread across views
- view code should prefer dispatching intent to state methods instead of embedding business logic inline
- GPU handles and low-level renderer resources should not become general-purpose application state

## Error Handling

Prefer structured errors in reusable modules and crates.

Rules:

- library-style modules should prefer typed errors with `thiserror`
- application entrypoints and top-level command handlers may use `anyhow`
- avoid new `Result<T, String>` APIs in reusable code
- error messages should include action context
- only convert typed errors to strings at UI or transport boundaries when necessary

Example:

```rust
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("GPU device lost: {0}")]
    DeviceLost(String),
}
```

If a module already uses string errors, treat typed errors as the preferred direction for future cleanup rather than forcing a giant rewrite in one PR.

## Comments and File Shape

Repository-specific rules already require:

- concise English comments for added or significantly modified code blocks
- the 3-line file header in Rust source files

Follow these additional rules:

- comments must be written in English
- comments should explain purpose, constraints, or why a block exists
- do not add comments for obvious assignments
- if a function needs a paragraph to explain basic flow, the function is probably too large
- each Rust source file should keep this 3-line header at the top:

```rust
// =========================================
// =========================================
// <repo-relative-file-path>
```

- the third line should match the file location, for example `// src/ui/video_preview.rs`

## Function and Module Design

Rules:

- one function should do one coherent job
- if a function needs many unrelated booleans or more than a small set of positional parameters, consider a struct
- long functions should be split around meaningful state transitions or phases
- keep helper functions close to the module that owns the behavior

Prefer:

- pure helpers for calculations
- explicit names over abbreviated local cleverness
- small orchestration functions calling focused helpers

## Shader and Asset Management

For project-owned shader code:

- keep shader entry points centralized instead of scattering string paths everywhere
- use `include_str!` or another compile-time loading pattern when practical
- add build-time validation if shader count or complexity grows enough to justify it

Do not mix project-owned shader conventions with vendored GPUI shader copies.

## Testing Expectations

Validation depth should match change risk.

Rules:

- core logic changes should usually include tests
- API validation and edit-plan logic should get regression coverage when bugs are fixed
- UI-only changes with no reasonable test seam must still pass `cargo check`

When changing behavior, prefer tests around:

- edit-plan validation
- timeline snapshot behavior
- subtitle and silence analysis
- project serialization and restore flows

## Required Validation Commands

Before opening a PR, run the strongest applicable checks:

```bash
cargo fmt --check
cargo check
cargo clippy --workspace --all-targets -- -D clippy::correctness -D clippy::suspicious -W clippy::perf
cargo test
```

If a change is truly UI-only and there is no existing test seam for that area, document that and still run:

```bash
cargo fmt --check
cargo check
```

## PR Standard

Every contributor should be able to answer these questions in the PR:

- Why does this logic live in this file/module?
- What invariant is now enforced more clearly?
- What tests or checks cover the change?
- Did this add new business logic to UI code? If yes, why was that unavoidable?

## Preferred Direction for Future Refactors

Long term, the project should continue moving toward:

- stronger domain types around time, units, and IDs
- cleaner separation between `core`, renderer code, and UI
- typed errors in reusable modules
- more deterministic tests around timeline editing flows

Do this incrementally. Do not force large architectural churn into unrelated feature PRs.
