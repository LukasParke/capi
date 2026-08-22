//! update.rs download-failure arms via the mock release server.

#![cfg(feature = "mock-cec")]

use capi::settings::Settings;

#[tokio::test]
async fn binary_asset_404_surfaces_download_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("capi"), b"OLD").unwrap();
    let (settings, _) = Settings::load(&dir.path().join("config.json")).unwrap();

    // Serve latest-release JSON pointing at a MISSING binary asset but a
    // present SHA256SUMS: the binary download must 404 first.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let app = axum::Router::new()
        .route("/repos/{owner}/{repo}/releases/latest", axum::routing::get(move || {
            let base = base.clone();
            async move {
                axum::Json(serde_json::json!({
                    "tag_name": "v99.0.0",
                    "assets": [
                        {"name": "capi-linux-arm64-libcec6", "browser_download_url": format!("{base}/assets/MISSING")},
                        {"name": "SHA256SUMS", "browser_download_url": format!("{base}/assets/sums")},
                    ],
                }))
            }
        }))
        .route("/assets/sums", axum::routing::get(|| async {
            "deadbeef  capi-linux-arm64-libcec6\n".to_string()
        }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let err = capi::update::__test_check(
        &settings,
        &format!("http://{addr}"),
        Some(dir.path().to_path_buf()),
    )
    .await
    .unwrap_err();
    assert!(err.contains("404") || err.contains("download"), "{err}");
    assert_eq!(std::fs::read(dir.path().join("capi")).unwrap(), b"OLD");
}
