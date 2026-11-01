use base64::Engine as _;

use super::super::config::{has_required_cookies, parse_accounts, parse_cookie_header};
use super::super::credentials::{
    ParsedMuseCredential, build_wss_url, get_wss_expiry_unix_secs, merge_set_cookies,
};
use super::super::noise::{LOW_ORDER_POINTS, NoiseXXInitiator, NoiseXXResponder, x25519_dh};
use super::super::proto::{
    NoiseFrameDecoder, SERVICE_DAEMON, decode_service_request_frame, encode_application_request,
    encode_service_frame, encode_service_request, split_into_transport_frames,
};
use super::super::workflow::{
    decode_uncompressed_or_simple_png, flatten_messages, format_image_prompt, format_video_prompt,
    remove_background_to_rgba_png,
};
use super::harness::make_test_rgba_png;
use crate::providers::llm::{ChatMessage, LlmRequest};

// ============================================================================
// 1. Noise_XX_25519_AESGCM_SHA256 Handshake & Transport Cryptography Tests
// ============================================================================

#[test]
fn noise_xx_handshake_and_transport_roundtrip() {
    let mut initiator = NoiseXXInitiator::new(None).expect("initiator");
    let mut responder = NoiseXXResponder::new(None).expect("responder");

    let msg1 = initiator.write_message_1().expect("write message 1");
    assert_eq!(msg1.len(), 66);

    let _client_nonce = responder
        .read_message_1(&msg1)
        .expect("responder reads message 1");
    let msg2 = responder
        .write_message_2(b"server-cert-payload")
        .expect("responder writes message 2");
    let cert_payload = initiator
        .read_message_2(&msg2)
        .expect("initiator reads message 2");
    assert_eq!(cert_payload, b"server-cert-payload");
    let (msg3, mut client_send, mut client_recv) = initiator
        .write_message_3()
        .expect("initiator writes message 3");
    let (_msg3_payload, mut server_send, mut server_recv) = responder
        .read_message_3(&msg3)
        .expect("responder reads message 3");

    for i in 0..5 {
        let client_plain = format!("hello from client frame #{i}").into_bytes();
        let ct = client_send
            .encrypt_with_ad(&[], &client_plain)
            .expect("client encrypt");
        let dec = server_recv
            .decrypt_with_ad(&[], &ct)
            .expect("server decrypt");
        assert_eq!(dec, client_plain);

        let server_plain = format!("reply from server frame #{i}").into_bytes();
        let s_ct = server_send
            .encrypt_with_ad(&[], &server_plain)
            .expect("server encrypt");
        let s_dec = client_recv
            .decrypt_with_ad(&[], &s_ct)
            .expect("client decrypt");
        assert_eq!(s_dec, server_plain);
    }
}

#[test]
fn noise_xx_rejects_low_order_public_keys() {
    let scalar = [7u8; 32];
    for point in &LOW_ORDER_POINTS {
        let res = x25519_dh(&scalar, point);
        assert!(res.is_err(), "must reject low-order X25519 point");
    }
}

// ============================================================================
// 2. Protobuf Wire Framing & 32 KB Multi-Chunk Reassembly Tests
// ============================================================================

#[test]
fn protobuf_request_and_multi_chunk_reassembly_roundtrip() {
    let large_body: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
    let headers: &[(&str, &str)] = &[
        ("accept", "application/json"),
        ("content-type", "application/json"),
    ];
    let app_req = encode_application_request("POST", "/fs/write", headers, Some(&large_body), true);
    let svc_frame = encode_service_frame(42, &app_req);
    let svc_req = encode_service_request(SERVICE_DAEMON, &svc_frame);

    let mut chunks = split_into_transport_frames(42, &svc_req);
    assert!(
        chunks.len() >= 4,
        "100 KB payload must split into >= 4 chunks"
    );
    // Reverse chunk arrival order to verify out-of-order reassembly
    chunks.reverse();

    let mut decoder = NoiseFrameDecoder::default();
    let mut assembled = None;
    for (idx, chunk) in chunks.iter().enumerate() {
        let res = decoder.push(chunk).expect("valid chunk");
        if idx + 1 < chunks.len() {
            assert!(res.is_none(), "must wait until all chunks arrive");
        } else {
            assembled = res;
        }
    }

    let assembled = assembled.expect("reassembled payload");
    assert_eq!(assembled, svc_req);

    let decoded_req = decode_service_request_frame(&assembled).expect("decode service request");
    assert_eq!(decoded_req.service_type, SERVICE_DAEMON);
    assert_eq!(decoded_req.stream_id, 42);
    assert_eq!(decoded_req.verb, "POST");
    assert_eq!(decoded_req.path, "/fs/write");
    assert_eq!(decoded_req.body, large_body);
}

// ============================================================================
// 3. Credential Normalization, Redaction & Token Expiry Tests
// ============================================================================

#[test]
fn credential_parser_accepts_single_hatch_sess_wss_hybrid_and_init_json() {
    // 1. Single hatch_sess cookie
    let c1 = ParsedMuseCredential::parse("hatch_sess=secret_cookie_val").expect("single cookie");
    assert_eq!(
        c1.cookie_header.as_deref(),
        Some("hatch_sess=secret_cookie_val")
    );
    assert!(c1.wss_url.is_none());
    assert!(!format!("{c1:?}").contains("secret_cookie_val"));
    assert!(format!("{c1:?}").contains("[REDACTED]"));

    // 2. Direct wss:// URL
    let wss = build_wss_url("vm-123", "s0:eyJ.eyJ.sig", "notary-xyz");
    let c2 = ParsedMuseCredential::parse(&wss).expect("direct wss");
    assert_eq!(c2.wss_url.as_deref(), Some(wss.as_str()));
    assert!(c2.cookie_header.is_none());
    assert!(!format!("{c2:?}").contains("notary-xyz"));

    // 3. Hybrid ("wss://...\nhatch_sess=...")
    let hybrid = format!("{wss}\nhatch_sess=hybrid_secret; theme=dark");
    let c3 = ParsedMuseCredential::parse(&hybrid).expect("hybrid");
    assert_eq!(c3.wss_url.as_deref(), Some(wss.as_str()));
    assert_eq!(
        c3.cookie_header.as_deref(),
        Some("hatch_sess=hybrid_secret")
    );
    assert_eq!(
        c3.to_hybrid_string(),
        format!("{wss}\nhatch_sess=hybrid_secret")
    );

    // 4. __HATCH_GATEWAY_INIT__ JSON
    let init_json = r#"{"vmName":"vm-init-9","authToken":"tok-abc","noiseNotaryToken":"notary-9"}"#;
    let c4 = ParsedMuseCredential::parse(init_json).expect("init json");
    let built_wss = c4.wss_url.as_deref().expect("wss built from init json");
    assert!(built_wss.starts_with("wss://hatch.metaaivm.com/v1/noise?"));
    assert!(built_wss.contains("vm_id=vm-init-9"));
    assert!(built_wss.contains("auth_token=tok-abc"));

    // Verify MuseAccountCredential Debug redaction as well
    let accs = parse_accounts(&hybrid);
    assert_eq!(accs.len(), 1);
    assert_eq!(accs[0].vm_id.as_deref(), Some("vm-123"));
    let acc_dbg = format!("{:?}", accs[0]);
    assert!(!acc_dbg.contains("hybrid_secret"));
    assert!(!acc_dbg.contains("notary-xyz"));
}

#[test]
fn wss_expiry_extracts_notary_timeout_and_s0_prefixed_jwt_exp() {
    let jwt_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(r#"{"exp":1893457000,"iat":1893400000}"#);
    let auth_token = format!("s0:eyJhbGciOiJIUzI1NiJ9.{jwt_payload}.sig");

    // Raw timeout:<unix> in notary_token
    let wss_raw = build_wss_url("vm-1", &auth_token, "v1.timeout:1893456000.sig");
    assert_eq!(get_wss_expiry_unix_secs(&wss_raw), Some(1_893_456_000));

    // Base64-encoded timeout:<unix> in notary_token where JWT exp is smaller
    let b64_notary =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("meta:timeout:1893499000");
    let wss_b64 = build_wss_url("vm-1", &auth_token, &format!("hdr.{b64_notary}.sig"));
    assert_eq!(get_wss_expiry_unix_secs(&wss_b64), Some(1_893_457_000));

    // Set-Cookie merge ignores Max-Age=0 deletion headers
    let mut headers = reqwest::header::HeaderMap::new();
    headers.append(
        reqwest::header::SET_COOKIE,
        "hatch_sess=deleted; Max-Age=0; Path=/".parse().unwrap(),
    );
    headers.append(
        reqwest::header::SET_COOKIE,
        "hatch_gw=new_gw; Path=/; HttpOnly".parse().unwrap(),
    );
    let merged = merge_set_cookies("hatch_sess=keep_me", &headers);
    assert_eq!(merged, "hatch_gw=new_gw; hatch_sess=keep_me");
    assert!(has_required_cookies(&parse_cookie_header(&merged)));
}

#[test]
fn flatten_messages_and_media_prompts_preserve_structure() {
    let req = LlmRequest {
        messages: vec![
            ChatMessage::System {
                content: "Be concise.".to_string(),
            },
            ChatMessage::User {
                content: "Hello!".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let flat = flatten_messages(&req);
    assert!(flat.contains("[System]\nBe concise."));
    assert!(flat.contains("[User]\nHello!"));

    let orch_req = LlmRequest {
        messages: vec![
            ChatMessage::System {
                content: "<!-- base-instructions-version: 1 -->\n# Orchestrator base instructions\n\nTrả lời cuối cùng là văn bản rõ ràng. `completed` chỉ nghĩa là lượt chạy đã kết thúc,\n  không chứng nhận yêu cầu nghiệp vụ đã đạt; hãy nêu rõ phần chưa làm được.\n\nBạn tìm kiếm tổng hợp thông tin sau đó trả lời bằng một tin nhắn\n\n<project_instructions>\n# Wiki Schema — Research Deep-Dive\n</project_instructions>\n\n<available_skills>\n  <skill><id>wiki-taxonomy</id></skill>\n</available_skills>".to_string(),
            },
            ChatMessage::User {
                content: "Tìm thông tin về Chiến tranh Triều Tiên.".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let orch_flat = flatten_messages(&orch_req);
    assert!(
        !orch_flat.contains("Orchestrator base instructions"),
        "Orchestrator base boilerplate must be stripped for Muse prompt"
    );
    assert!(
        !orch_flat.contains("<project_instructions>"),
        "Wiki project_instructions must be stripped for Muse prompt"
    );
    assert!(
        !orch_flat.contains("<available_skills>"),
        "available_skills XML must be stripped for Muse prompt"
    );
    assert!(
        orch_flat
            .contains("[System]\nBạn tìm kiếm tổng hợp thông tin sau đó trả lời bằng một tin nhắn")
    );
    assert!(orch_flat.contains("[User]\nTìm thông tin về Chiến tranh Triều Tiên."));

    let img_prompt = format_image_prompt("A neon city", 1920, 1080, true);
    assert!(img_prompt.contains("aspect ratio 16:9"));
    assert!(img_prompt.contains("solid, uniform, flat pure-white (#FFFFFF) studio background"));

    let vid_prompt = format_video_prompt("A rocket launch", 1280, 720, 28);
    assert!(vid_prompt.contains("3 sequential ~10-second beats"));
    assert!(vid_prompt.contains("[ALL_MEDIA_ATTACHED]"));
}

#[test]
fn remove_background_to_rgba_png_clears_border_background() {
    let encoded = make_test_rgba_png(16, 16, 4..12);
    let out_bytes = remove_background_to_rgba_png(&encoded).expect("remove background");
    let (w, _h, rgba) = decode_uncompressed_or_simple_png(&out_bytes).expect("decode output png");
    assert_eq!(rgba[3], 0, "corner background must be transparent");
    let center_idx = ((8 * w + 8) * 4 + 3) as usize;
    assert_eq!(
        rgba[center_idx], 255,
        "center foreground must remain opaque"
    );
}
