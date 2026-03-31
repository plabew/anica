use media_gen_protocol::error::{ErrorCode, ProtocolError};

#[test]
fn protocol_error_keeps_provider_metadata() {
    let err = ProtocolError::new(ErrorCode::ContentPolicyViolation, "blocked")
        .with_provider("openai")
        .with_provider_code("content_policy_violation")
        .with_http_status(400)
        .retriable(false);

    assert_eq!(err.code, ErrorCode::ContentPolicyViolation);
    assert_eq!(err.provider.as_deref(), Some("openai"));
    assert_eq!(
        err.provider_code.as_deref(),
        Some("content_policy_violation")
    );
    assert_eq!(err.provider_http_status, Some(400));
}
