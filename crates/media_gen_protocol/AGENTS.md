## media_gen_protocol Rules (No Fallback)

Scope:
- Applies to all files under `crates/media_gen_protocol/`.
- Applies to protocol, gateway, and provider behavior inside this crate.

Hard rules:
- Never perform implicit fallback when the caller specifies `provider/model`.
- Never silently rewrite model names (for example, `model-a` to `model-b`).
- Never silently switch provider-specific behavior to a different provider.
- Never silently downgrade capability (for example, image-edit request to a non-edit generation path).

Validation and errors:
- If requested model/capability is unsupported, return typed `ProtocolError` with `ErrorCode::InvalidArgument`.
- Error messages must include:
  - the requested value
  - why it is unsupported
  - at least one valid alternative (when known)
- Preserve provider diagnostics when available:
  - `provider`
  - `provider_code`
  - `provider_http_status`

Auth/key handling:
- Do not use implicit cross-provider key fallback.
- If required key slot is missing, return `ErrorCode::AuthFailed` with explicit slot/provider context.

Review checklist:
- Any new model routing code must prove no implicit fallback path exists.
- Any compatibility handling must be explicit and opt-in, never silent by default.
