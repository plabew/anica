# media_gen_protocol

Provider-agnostic protocol and runtime skeleton for image/video generation.

## Goals

- Use `provider/model-name` route key for provider routing.
- Keep caller-facing schema stable across providers.
- Normalize provider errors into one typed error model.
- Keep `negative_prompt` always available (adapter may ignore silently).
- Expose async submit/poll/cancel flow for all providers (including sync providers).
- Provide `provider_options` as an escape hatch for provider-specific fields.
- Normalize final outputs to URL form.

## Status

This crate currently provides:

- Stable protocol types (`request`, `job`, `response`, `asset`)
- Typed errors via `thiserror`
- Model route key parsing and basic in-memory registry
- Gateway + adapter trait skeleton
- In-memory job store skeleton
- Output normalizer/uploader trait skeleton

Provider-specific API calls are intentionally left as stubs in this phase.
