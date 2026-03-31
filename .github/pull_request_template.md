## Summary

- What changed:
- Why this belongs in this module:

## Validation

- [ ] `cargo fmt --check`
- [ ] `cargo check`
- [ ] `cargo clippy --workspace --all-targets -- -D clippy::correctness -D clippy::suspicious -W clippy::perf`
- [ ] `cargo test`
- [ ] Not run intentionally, with reason explained below

## Contributor Standard Check

- [ ] New business logic was placed in `src/core/` or `src/api/`, not hidden in UI rendering code
- [ ] Repeated or risky primitive units were reviewed for stronger domain typing
- [ ] Reusable modules avoided adding new `Result<T, String>` APIs without a clear reason
- [ ] Added or significantly modified code blocks include concise English purpose comments where needed
- [ ] Tests were added or an explicit reason was provided

## Notes

- Risk:
- Follow-up work:
