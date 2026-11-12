use super::harness::*;

#[tokio::test]
async fn admin_web_token_helper_authenticates_admin_api() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let token = admin_web_token(&client, &base).await;

    let response = client
        .get(format!("{base}/api/admin/system"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let relogin = client
        .post(format!("{base}/api/admin/auth/login"))
        .json(&json!({ "password": HARNESS_ADMIN_PASSWORD }))
        .send()
        .await
        .unwrap();
    assert_eq!(relogin.status(), StatusCode::OK);
    task.abort();
}
