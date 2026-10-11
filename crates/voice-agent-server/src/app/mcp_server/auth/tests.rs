use super::*;
use serde_json::json;
fn token(key: &hmac::Key, header: serde_json::Value, body: serde_json::Value) -> String {
    let head = URL_SAFE_NO_PAD.encode(header.to_string());
    let body = URL_SAFE_NO_PAD.encode(body.to_string());
    let input = format!("{head}.{body}");
    format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(hmac::sign(key, input.as_bytes()))
    )
}
#[test]
fn tokens_bind_signature_algorithm_resource_identity_scope_and_time() {
    let verifier = Verifier {
        key: hmac::Key::new(
            hmac::HMAC_SHA256,
            b"a dedicated test key that is sufficiently long",
        ),
        issuer: "https://issuer.example".into(),
        audience: "https://agent.example/mcp/system".into(),
    };
    let now = crate::database::unix_seconds().unwrap();
    let header = json!({"alg":"HS256","typ":"JWT"});
    let claims = json!({"iss":verifier.issuer,"aud":verifier.audience,"sub":"client","jti":"one","scope":"system:read","iat":now,"exp":now+300});
    let context = verifier
        .verify(&token(&verifier.key, header.clone(), claims.clone()))
        .unwrap();
    assert!(context.grants.system_read);
    assert!(!context.grants.mcp_manage_request);
    for (field, value) in [
        ("iss", json!("https://wrong.example")),
        ("aud", json!("admin")),
        ("scope", json!("mcp:manage:execute")),
        ("exp", json!(now)),
        ("exp", json!(now + 3601)),
        ("iat", json!(now + 1)),
        ("nbf", json!(now + 1)),
        ("sub", json!("")),
        ("jti", json!("")),
    ] {
        let mut invalid = claims.clone();
        invalid[field] = value;
        assert!(
            verifier
                .verify(&token(&verifier.key, header.clone(), invalid))
                .is_err(),
            "{field}"
        );
    }
    assert!(
        verifier
            .verify(&token(
                &verifier.key,
                json!({"alg":"none","typ":"JWT"}),
                claims.clone()
            ))
            .is_err()
    );
    let wrong = hmac::Key::new(hmac::HMAC_SHA256, b"wrong key");
    assert!(verifier.verify(&token(&wrong, header, claims)).is_err());
    assert!(verifier.verify("admin-token").is_err());
}
