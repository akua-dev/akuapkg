use std::path::Path;

#[test]
fn git_fetch_uses_curl_transport_for_tls_configuration() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(manifest_path).expect("read akua-core Cargo.toml");
    let manifest: toml::Value = toml::from_str(&manifest).expect("parse akua-core Cargo.toml");

    let features = manifest
        .get("target")
        .expect("target table")
        .get("cfg(not(target_arch = \"wasm32\"))")
        .expect("host target table")
        .get("dependencies")
        .expect("host dependencies table")
        .get("gix")
        .expect("gix dependency")
        .get("features")
        .expect("gix features")
        .as_array()
        .expect("gix features");

    assert!(
        features
            .iter()
            .any(|feature| feature.as_str() == Some("blocking-http-transport-curl")),
        "gix HTTP transport must use curl so gitoxide applies GIT_SSL_CAINFO/http.sslCAInfo"
    );
    assert!(
        features
            .iter()
            .all(|feature| feature.as_str() != Some("blocking-http-transport-reqwest-rust-tls")),
        "reqwest-rust-tls transport does not honor the E2E git TLS trust path"
    );
}
