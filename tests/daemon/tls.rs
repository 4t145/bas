use super::*;
use bas::service::tls::{ClientTls, ConnectionOptions};
use rcgen::{BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair};
use std::path::{Path, PathBuf};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint};

struct Certificates {
    ca: PathBuf,
    server: PathBuf,
    server_key: PathBuf,
    client: PathBuf,
    client_key: PathBuf,
}

fn certificates(directory: &Path) -> Certificates {
    std::fs::create_dir_all(directory).expect("certificate directory");
    let ca_key = KeyPair::generate().expect("CA key");
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = params.self_signed(&ca_key).expect("CA certificate");
    let issue = |name: &str, usage: ExtendedKeyUsagePurpose| {
        let key = KeyPair::generate().expect("leaf key");
        let mut params =
            CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).expect("SANs");
        params.extended_key_usages = vec![usage];
        let certificate = params
            .signed_by(&key, &ca, &ca_key)
            .expect("signed certificate");
        let cert_path = directory.join(format!("{name}.pem"));
        let key_path = directory.join(format!("{name}.key"));
        std::fs::write(&cert_path, certificate.pem()).expect("certificate file");
        std::fs::write(&key_path, key.serialize_pem()).expect("temporary private key");
        (cert_path, key_path)
    };
    let (server, server_key) = issue("server", ExtendedKeyUsagePurpose::ServerAuth);
    let (client, client_key) = issue("client", ExtendedKeyUsagePurpose::ClientAuth);
    let ca_path = directory.join("ca.pem");
    std::fs::write(&ca_path, ca.pem()).expect("CA file");
    Certificates {
        ca: ca_path,
        server,
        server_key,
        client,
        client_key,
    }
}

impl Certificates {
    fn options(&self, target: Target) -> ConnectionOptions {
        ConnectionOptions {
            target,
            tls: Some(ClientTls {
                ca: self.ca.clone(),
                certificate: self.client.clone(),
                private_key: self.client_key.clone(),
                server_name: None,
            }),
        }
    }

    fn configure(&self, command: &mut Command) {
        command
            .arg("--tls-certificate")
            .arg(&self.server)
            .arg("--tls-key")
            .arg(&self.server_key)
            .arg("--tls-client-ca")
            .arg(&self.ca);
    }
}

async fn agents(options: &ConnectionOptions) -> anyhow::Result<()> {
    let mut client = Client::connect_options(options).await?;
    client.list_agents(pb::ListAgentsRequest {}).await?;
    Ok(())
}

async fn rejected(options: &ConnectionOptions) {
    assert!(
        tokio::time::timeout(TIMEOUT, agents(options))
            .await
            .expect("authentication failure is bounded")
            .is_err()
    );
}

#[tokio::test]
async fn mtls_handshakes_rpc_stream_and_mixed_listeners() {
    let directory = tempfile::tempdir().expect("temporary secrets");
    let certs = certificates(&directory.path().join("trusted"));
    let other = certificates(&directory.path().join("untrusted"));
    let ipc = Target::Unix(directory.path().join("ipc/bas.sock"));
    let mut command = command();
    let settings = directory.path().join("settings.json");
    std::fs::write(
        &settings,
        serde_json::to_vec(&bas::config::Settings::default()).expect("settings"),
    )
    .expect("settings file");
    command
        .env_remove("BAS_AGENTS")
        .env("BAS_DAEMON_SETTINGS_FILE", &settings)
        .env("BAS_ACP_DATA_DIR", directory.path().join("acp"));
    command.args([
        "--notify",
        "never",
        "--log-backend",
        "stdout",
        "--log-filter",
        "info",
        "--listen",
        "127.0.0.1:0",
        "--listen",
        "mtls://127.0.0.1:0",
        "--listen",
        &ipc.to_string(),
    ]);
    certs.configure(&mut command);
    // The first readiness record is the plain TCP listener.
    let mut daemon = Daemon::start(command).await;
    let target: Target = tokio::time::timeout(TIMEOUT, async {
        loop {
            let line = daemon
                ._logs
                .next_line()
                .await
                .expect("read readiness")
                .expect("daemon remains alive");
            if line.contains("serving the bas.v1.Bas service") && line.contains("address=mtls://") {
                break line
                    .split("address=")
                    .nth(1)
                    .expect("address")
                    .split_whitespace()
                    .next()
                    .expect("target")
                    .parse()
                    .expect("mTLS target");
            }
        }
    })
    .await
    .expect("TLS listener ready");
    let options = certs.options(target.clone());
    agents(&options).await.expect("verified mutual TLS RPC");
    let mut client = Client::connect_options(&options)
        .await
        .expect("TLS channel");
    super::management::edit_configuration(&mut client, &settings).await;
    let session = client
        .create_session(pb::CreateSessionRequest::default())
        .await
        .expect("create over TLS")
        .into_inner()
        .session
        .expect("session");
    let mut stream = client
        .attach(pb::AttachRequest {
            session: session.id.clone(),
            cursor: None,
            replay: 0,
        })
        .await
        .expect("TLS streaming RPC")
        .into_inner();
    // Both other listeners see the same registry, not parallel runtimes.
    for retained in [daemon.address.clone(), ipc] {
        let mut plain = Client::connect_target(&retained)
            .await
            .expect("retained transport");
        let sessions = plain
            .list_sessions(pb::ListSessionsRequest {
                include_archived: true,
            })
            .await
            .expect("same registry")
            .into_inner()
            .sessions;
        assert!(sessions.iter().any(|candidate| candidate.id == session.id));
    }
    let error = client
        .remove_agent(pb::RemoveAgentRequest {
            id: "not-installed".into(),
        })
        .await
        .expect_err("unknown managed agent is rejected, not the TLS transport");
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);

    let Target::Mtls(address) = &target else {
        unreachable!()
    };
    let port = address
        .as_str()
        .parse::<std::net::SocketAddr>()
        .expect("bound IP")
        .port();
    let dns = certs.options(
        format!("mtls://localhost:{port}")
            .parse()
            .expect("DNS target"),
    );
    agents(&dns)
        .await
        .expect("DNS plus certificate hostname verification");
    let mut wrong_name = options.clone();
    wrong_name.tls.as_mut().expect("TLS").server_name = Some("wrong.invalid".into());
    rejected(&wrong_name).await;
    let mut wrong_trust = options.clone();
    wrong_trust.tls.as_mut().expect("TLS").ca = other.ca.clone();
    rejected(&wrong_trust).await;
    let mut wrong_client = options.clone();
    let tls = wrong_client.tls.as_mut().expect("TLS");
    tls.certificate = other.client.clone();
    tls.private_key = other.client_key.clone();
    rejected(&wrong_client).await;

    let anonymous = Endpoint::from_shared(format!("https://{address}"))
        .expect("endpoint")
        .tls_config(
            ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(std::fs::read(&certs.ca).expect("CA"))),
        )
        .expect("server trust only");
    let anonymous = async {
        let channel = anonymous.connect().await?;
        Client::new(channel)
            .list_agents(pb::ListAgentsRequest {})
            .await?;
        anyhow::Ok(())
    };
    assert!(
        tokio::time::timeout(TIMEOUT, anonymous)
            .await
            .expect("bounded")
            .is_err()
    );
    rejected(&ConnectionOptions::from(Target::Tcp(address.clone()))).await;
    assert!(
        Client::connect_target(&target).await.is_err(),
        "legacy API never downgrades"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_bast"))
        .arg("--server")
        .arg(target.to_string())
        .arg("--tls-ca")
        .arg(&certs.ca)
        .arg("--tls-certificate")
        .arg(&certs.client)
        .arg("--tls-key")
        .arg(&certs.client_key)
        .arg("--tls-server-name")
        .arg("localhost")
        .arg("agents")
        .kill_on_drop(true)
        .output()
        .await
        .expect("mTLS CLI");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("embedded"));
    daemon.stop().await;
    let mut stopped = false;
    tokio::time::timeout(TIMEOUT, async {
        while let Some(event) = stream.message().await.expect("stream drains") {
            if matches!(event.kind, Some(pb::event::Kind::StatusChanged(pb::StatusChanged { status }))
                if status == pb::SessionStatus::Stopped as i32) {
                stopped = true;
            }
        }
    }).await.expect("TLS stream closes");
    assert!(stopped, "live TLS stream delivers shutdown event");
}

#[tokio::test]
async fn invalid_or_incomplete_tls_configuration_fails_closed() {
    let directory = tempfile::tempdir().expect("temporary secrets");
    let certs = certificates(directory.path());
    let tls_target: Target = "mtls://127.0.0.1:1".parse().expect("target");
    assert!(
        ConnectionOptions::from(tls_target.clone())
            .validate()
            .is_err()
    );
    assert!(certs.options(Target::Local).validate().is_err());
    let mut options = certs.options(tls_target);
    options.tls.as_mut().expect("TLS").private_key = directory.path().join("missing");
    assert!(options.channel_lazy(TIMEOUT).is_err());
    std::fs::write(&certs.client_key, "not a private key").expect("invalid PEM");
    options.tls.as_mut().expect("TLS").private_key = certs.client_key.clone();
    assert!(options.channel_lazy(TIMEOUT).is_err());

    for args in [
        vec!["--listen", "mtls://127.0.0.1:0"],
        vec!["--tls-key", "/missing"],
    ] {
        let output = command()
            .args(args)
            .output()
            .await
            .expect("invalid daemon invocation");
        assert!(!output.status.success());
    }
    std::fs::write(&certs.server_key, "not a private key").expect("invalid server PEM");
    let mut invalid = command();
    invalid.args(["--listen", "mtls://127.0.0.1:0", "--notify", "never"]);
    certs.configure(&mut invalid);
    let output = tokio::time::timeout(TIMEOUT, invalid.output())
        .await
        .expect("fails before ready")
        .expect("invalid server");
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("serving the bas.v1.Bas service"));
}
