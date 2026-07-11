#![cfg(feature = "git-fetch")]

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use akua_core::host_auth::{BasicAuth, HostAuthMap};
use base64::Engine;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::PrivateKeyDer;
use rustls::{ServerConfig, ServerConnection, StreamOwned};

struct HttpsGitServer {
    address: std::net::SocketAddr,
    ca_path: PathBuf,
    auth_seen: Arc<AtomicBool>,
    ambient_header_seen: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HttpsGitServer {
    fn start(repo: PathBuf, fixture_root: &Path, username: &str, password: &str) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let certified = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let ca_path = fixture_root.join("ca.pem");
        fs::write(&ca_path, certified.cert.pem()).unwrap();

        let key = PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into());
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert.der().clone()], key)
            .unwrap();
        let config = Arc::new(config);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let auth_seen = Arc::new(AtomicBool::new(false));
        let ambient_header_seen = Arc::new(AtomicBool::new(false));
        let expected_auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
        );

        let thread_stop = Arc::clone(&stop);
        let thread_auth_seen = Arc::clone(&auth_seen);
        let thread_ambient_header_seen = Arc::clone(&ambient_header_seen);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        if let Err(error) = handle_request(
                            stream,
                            Arc::clone(&config),
                            &repo,
                            &expected_auth,
                            &thread_auth_seen,
                            &thread_ambient_header_seen,
                        ) {
                            eprintln!("HTTPS git fixture request failed: {error}");
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            address,
            ca_path,
            auth_seen,
            ambient_header_seen,
            stop,
            thread: Some(thread),
        }
    }

    fn url(&self) -> String {
        format!("https://localhost:{}/repo.git", self.address.port())
    }
}

impl Drop for HttpsGitServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn handle_request(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    repo: &Path,
    expected_auth: &str,
    auth_seen: &AtomicBool,
    ambient_header_seen: &AtomicBool,
) -> std::io::Result<()> {
    let connection = ServerConnection::new(config).unwrap();
    let mut stream = StreamOwned::new(connection, stream);
    let request = read_request(&mut stream)?;
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .ok_or_else(|| std::io::Error::other("missing HTTP header terminator"))?;
    let headers = String::from_utf8_lossy(&request[..header_end]);
    if headers
        .lines()
        .any(|line| line == "X-Akua-Ambient-Secret: must-not-leak")
    {
        ambient_header_seen.store(true, Ordering::Relaxed);
    }
    let mut request_line = headers
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = request_line.next().unwrap_or_default();
    let target = request_line.next().unwrap_or_default();

    let authenticated = headers.lines().any(|line| {
        line.strip_prefix("Authorization: ")
            .is_some_and(|value| value.trim() == expected_auth)
    });
    if !authenticated {
        return write_response(
            &mut stream,
            "401 Unauthorized",
            "text/plain",
            b"",
            Some("WWW-Authenticate: Basic realm=\"akua-test\"\r\n"),
        );
    }
    auth_seen.store(true, Ordering::Relaxed);

    match method {
        "GET" if target.starts_with("/repo.git/info/refs?service=git-upload-pack") => {
            let advertised = git_upload_pack(repo, true, &[])?;
            let service = b"# service=git-upload-pack\n";
            let mut body = format!("{:04x}", service.len() + 4).into_bytes();
            body.extend_from_slice(service);
            body.extend_from_slice(b"0000");
            body.extend_from_slice(&advertised);
            write_response(
                &mut stream,
                "200 OK",
                "application/x-git-upload-pack-advertisement",
                &body,
                None,
            )
        }
        "POST" if target == "/repo.git/git-upload-pack" => {
            let result = git_upload_pack(repo, false, &request[header_end..])?;
            write_response(
                &mut stream,
                "200 OK",
                "application/x-git-upload-pack-result",
                &result,
                None,
            )
        }
        _ => write_response(&mut stream, "404 Not Found", "text/plain", b"", None),
    }
}

fn read_request(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut content_length = None;
    loop {
        let mut chunk = [0_u8; 8192];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(header_start) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let header_end = header_start + 4;
            let length = *content_length.get_or_insert_with(|| {
                String::from_utf8_lossy(&request[..header_end])
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .and_then(|value| value.trim().parse().ok())
                    })
                    .unwrap_or(0)
            });
            if request.len() >= header_end + length {
                break;
            }
        }
    }
    Ok(request)
}

fn write_response(
    stream: &mut impl Write,
    status: &str,
    content_type: &str,
    body: &[u8],
    extra_header: Option<&str>,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        body.len(),
        extra_header.unwrap_or_default()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn git_upload_pack(repo: &Path, advertise: bool, input: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut command = Command::new("git");
    command.arg("upload-pack").arg("--stateless-rpc");
    if advertise {
        command.arg("--advertise-refs");
    }
    let mut child = command
        .arg(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    if !input.is_empty() {
        child.stdin.take().unwrap().write_all(input)?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("git upload-pack failed"));
    }
    Ok(output.stdout)
}

fn git(args: &[&str], cwd: &Path) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap();
    assert!(status.success(), "git command failed: {args:?}");
}

fn make_tagged_repo(root: &Path) -> PathBuf {
    let work = root.join("work");
    let bare = root.join("origin.git");
    fs::create_dir_all(work.join("templates")).unwrap();
    git(&["init", "--bare", bare.to_str().unwrap()], root);
    git(&["init"], &work);
    git(&["config", "user.name", "Akua Test"], &work);
    git(&["config", "user.email", "test@akua.dev"], &work);
    fs::write(
        work.join("Chart.yaml"),
        "apiVersion: v2\nname: protected\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(work.join("templates/configmap.yaml"), "kind: ConfigMap\n").unwrap();
    git(&["add", "."], &work);
    git(&["commit", "-m", "fixture"], &work);
    git(&["tag", "v1.0.0"], &work);
    git(&["remote", "add", "origin", bare.to_str().unwrap()], &work);
    git(
        &["push", "origin", "HEAD:refs/heads/main", "refs/tags/v1.0.0"],
        &work,
    );
    bare
}

#[test]
fn vendor_add_forces_verification_and_preserves_environment_ca_bundle() {
    let fixture = tempfile::tempdir().unwrap();
    let bare = make_tagged_repo(fixture.path());
    let server = HttpsGitServer::start(bare, fixture.path(), "alice", "secret-token");

    let workspace = fixture.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(
        workspace.join("akua.toml"),
        format!(
            r#"[package]
name = "vendor-https-test"
version = "0.1.0"
edition = "akua.dev/v1alpha1"

[dependencies]
upstream = {{ git = "{}", tag = "v1.0.0" }}
"#,
            server.url()
        ),
    )
    .unwrap();

    std::env::set_var("XDG_CACHE_HOME", fixture.path().join("untrusted-cache"));

    let auth: HostAuthMap = HashMap::from([(
        format!("localhost:{}", server.address.port()),
        BasicAuth {
            username: "alice".to_string(),
            password: "secret-token".to_string(),
        },
    )]);

    let unrelated_ca = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let unrelated_ca_path = fixture.path().join("unrelated-ca.pem");
    fs::write(&unrelated_ca_path, unrelated_ca.cert.pem()).unwrap();
    std::env::set_var("GIT_SSL_NO_VERIFY", "true");
    std::env::set_var("GIT_SSL_CAINFO", &unrelated_ca_path);
    std::env::set_var("GIT_CONFIG_COUNT", "1");
    std::env::set_var("GIT_CONFIG_KEY_0", "http.extraHeader");
    std::env::set_var("GIT_CONFIG_VALUE_0", "X-Akua-Ambient-Secret: must-not-leak");
    let untrusted = akua_core::vendor::add(&workspace, "upstream", Some(&auth));
    assert!(
        untrusted.is_err(),
        "vendorAdd must force verification despite GIT_SSL_NO_VERIFY"
    );

    std::env::set_var("GIT_SSL_CAINFO", &server.ca_path);
    // A failed clone can leave transport-specific partial cache state. The
    // trusted clone is a separate assertion and must start from a clean cache.
    std::env::set_var("XDG_CACHE_HOME", fixture.path().join("trusted-cache"));
    let result = akua_core::vendor::add(&workspace, "upstream", Some(&auth));

    std::env::remove_var("GIT_SSL_NO_VERIFY");
    std::env::remove_var("GIT_SSL_CAINFO");
    std::env::remove_var("GIT_CONFIG_COUNT");
    std::env::remove_var("GIT_CONFIG_KEY_0");
    std::env::remove_var("GIT_CONFIG_VALUE_0");
    std::env::remove_var("XDG_CACHE_HOME");

    let output = result.expect("vendorAdd must trust GIT_SSL_CAINFO while verifying HTTPS");
    assert!(server.auth_seen.load(Ordering::Relaxed));
    assert!(!server.ambient_header_seen.load(Ordering::Relaxed));
    assert!(output.path.join("Chart.yaml").is_file());
    assert!(output.path.join("templates/configmap.yaml").is_file());
}
