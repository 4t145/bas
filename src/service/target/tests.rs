use super::*;

#[test]
fn default_is_local_and_tcp_is_explicit() {
    assert_eq!(Target::default(), Target::Local);
    assert_eq!(
        super::super::DEFAULT_TARGET.parse::<Target>().unwrap(),
        Target::Local
    );
    for input in [
        format!("127.0.0.1:{DEFAULT_PORT}"),
        format!("tcp://127.0.0.1:{DEFAULT_PORT}"),
    ] {
        let target: Target = input.parse().expect("TCP target");
        assert!(!target.is_local());
        assert!(target.is_loopback_tcp());
        assert_eq!(target.to_string(), format!("127.0.0.1:{DEFAULT_PORT}"));
        assert_eq!(
            target.state_key().unwrap(),
            format!("127.0.0.1_{DEFAULT_PORT}")
        );
    }
}

#[test]
fn endpoint_formats_round_trip() {
    for input in [
        "local",
        "unix:///run/user/1000/bas/service.sock",
        "pipe://bas-S-1-5-21",
        &format!("[::1]:{DEFAULT_PORT}"),
        &format!("192.0.2.10:{DEFAULT_PORT}"),
        "mtls://workstation:7338",
        "mtls://[::1]:7338",
    ] {
        let target: Target = input.parse().expect("valid target");
        assert_eq!(target.to_string().parse::<Target>().unwrap(), target);
    }
}

#[test]
fn hostname_targets_are_syntax_only_and_not_local_authority() {
    for host in [
        "localhost",
        "workstation",
        "workstation.tailnet.ts.net",
        "hosts_file",
        "unresolvable.invalid",
        "example.com.",
    ] {
        let plain = format!("{host}:{DEFAULT_PORT}");
        let target: Target = format!("tcp://{plain}").parse().expect("hostname");
        assert_eq!(target.to_string(), plain);
        assert_eq!(plain.parse::<Target>().expect("plain hostname"), target);
        assert_eq!(target.resolved().expect("no DNS lookup"), target);
        assert!(!target.is_local());
        assert!(!target.is_loopback_tcp());
        assert!(
            target
                .state_key()
                .expect("state key")
                .starts_with("tcp-host-")
        );
        assert_eq!(
            target.endpoint(Duration::from_secs(1)).uri().host(),
            Some(host)
        );
    }
}

#[test]
fn tcp_numeric_conversion_and_legacy_state_keys() {
    for (input, key) in [
        (
            format!("127.0.0.1:{DEFAULT_PORT}"),
            format!("127.0.0.1_{DEFAULT_PORT}"),
        ),
        (
            format!("[::1]:{DEFAULT_PORT}"),
            format!("___1__{DEFAULT_PORT}"),
        ),
    ] {
        let socket: SocketAddr = input.parse().expect("socket");
        let target = Target::from(socket);
        assert_eq!(target, Target::Tcp(TcpAddress::from(socket)));
        assert_eq!(target.to_string(), input);
        assert_eq!(target.state_key().expect("state key"), key);
        assert!(target.is_loopback_tcp());
    }
    assert!("host:0".parse::<Target>().is_ok());
    assert!("host:65535".parse::<Target>().is_ok());
}

#[test]
fn hostname_state_keys_are_distinct_from_legacy_numeric_keys() {
    let inputs = [
        format!("[::1]:{DEFAULT_PORT}"),
        format!("___1_:{DEFAULT_PORT}"),
        format!("localhost:{DEFAULT_PORT}"),
        format!("localhost:{}", DEFAULT_PORT + 1),
    ];
    let keys: std::collections::HashSet<_> = inputs
        .iter()
        .map(|input| {
            input
                .parse::<Target>()
                .expect("target")
                .state_key()
                .expect("key")
        })
        .collect();
    assert_eq!(keys.len(), inputs.len());
    assert!(keys.iter().all(|key| !key.contains(['/', '\\', ':'])));
}

#[test]
fn rejects_invalid_tcp_authorities() {
    for authority in [
        "user@host",
        "user:pass@host",
        "host/path",
        "host?query",
        "host#fragment",
        &format!("user@host:{DEFAULT_PORT}"),
        &format!("user:pass@host:{DEFAULT_PORT}"),
        &format!("host:{DEFAULT_PORT}/path"),
        &format!("host:{DEFAULT_PORT}?query"),
        &format!("host:{DEFAULT_PORT}#fragment"),
        "host:",
        &format!(":{DEFAULT_PORT}"),
        "host:http",
        "host:-1",
        "host:+1",
        "host:65536",
        "host:999999999999",
        &format!(" host:{DEFAULT_PORT}"),
        &format!("host:{DEFAULT_PORT} "),
        &format!("ho st:{DEFAULT_PORT}"),
        &format!("host:\t{DEFAULT_PORT}"),
        &format!("host:{DEFAULT_PORT}\n"),
        &format!("::1:{DEFAULT_PORT}"),
        &format!("[host]:{DEFAULT_PORT}"),
        "::1",
        "[::1]:",
        "[::1",
        "[::1]extra",
        "[::1]:65536",
        &format!("host..name:{DEFAULT_PORT}"),
        &format!(".:{DEFAULT_PORT}"),
        &format!("host..:{DEFAULT_PORT}"),
    ] {
        assert!(authority.parse::<TcpAddress>().is_err(), "{authority:?}");
        assert!(authority.parse::<Target>().is_err(), "{authority:?}");
        assert!(
            format!("tcp://{authority}").parse::<Target>().is_err(),
            "{authority:?}"
        );
        assert!(format!("mtls://{authority}").parse::<Target>().is_err());
    }
}

#[test]
fn rejects_ambiguous_and_remote_pipe_paths() {
    for input in [
        "",
        &format!("http://127.0.0.1:{DEFAULT_PORT}"),
        "unix://relative",
        "unix:///",
        "unix:///tmp/\0socket",
        "pipe://",
        "pipe://..",
        "pipe://server/name",
        "pipe://\\\\server\\pipe\\name",
        "https://example.com",
    ] {
        assert!(input.parse::<Target>().is_err(), "{input:?}");
    }
}

#[test]
fn oversized_tcp_authorities_are_rejected_before_creating_a_channel() {
    let authority = format!("{}:{DEFAULT_PORT}", "a".repeat(70_000));
    assert!(authority.parse::<TcpAddress>().is_err());
    assert!(format!("tcp://{authority}").parse::<Target>().is_err());
    assert!("a".repeat(70_000).parse::<TcpAddress>().is_err());
}

#[test]
fn omitted_default_ports_share_canonical_authorities_and_state() {
    assert_eq!(
        super::super::DEFAULT_ADDRESS.as_str(),
        format!("127.0.0.1:{DEFAULT_PORT}")
    );
    for host in [
        "localhost",
        "unresolvable.invalid",
        "192.0.2.10",
        "[::1]",
        "[2001:db8::1]",
    ] {
        for prefix in ["", "tcp://", "mtls://"] {
            let implicit: Target = format!("{prefix}{host}").parse().expect("portless target");
            let explicit: Target = format!("{prefix}{host}:{DEFAULT_PORT}")
                .parse()
                .expect("explicit target");
            assert_eq!(implicit, explicit);
            assert_eq!(
                implicit.state_key().expect("state"),
                explicit.state_key().expect("state")
            );
            let address: TcpAddress = host.parse().expect("address");
            assert_eq!(address.as_str(), format!("{host}:{DEFAULT_PORT}"));
            assert_eq!(address.display_compact(), host);
            assert_eq!(
                address
                    .display_compact()
                    .parse::<TcpAddress>()
                    .expect("round trip"),
                address
            );
        }
    }
    let expanded: TcpAddress = "[0:0:0:0:0:0:0:1]".parse().expect("IPv6");
    assert_eq!(expanded.as_str(), format!("[::1]:{DEFAULT_PORT}"));
    for host in ["host", "127.0.0.1", "[::1]"] {
        for port in [0, DEFAULT_PORT + 1] {
            let full = format!("{host}:{port}");
            let address: TcpAddress = full.parse().expect("explicit port");
            assert_eq!(address.display_compact(), full);
        }
    }
}

#[test]
fn ipc_state_keys_do_not_become_paths_or_collide_when_sanitized() {
    let a: Target = "unix:///tmp/a/b.sock".parse().unwrap();
    let b: Target = "unix:///tmp/a_b.sock".parse().unwrap();
    assert_ne!(a.state_key().unwrap(), b.state_key().unwrap());
    assert!(!a.state_key().unwrap().contains('/'));
}

#[cfg(any(unix, windows))]
#[test]
fn local_alias_and_concrete_endpoint_share_restore_state() {
    let concrete = Target::Local.resolved().expect("local endpoint");
    assert_ne!(concrete, Target::Local);
    assert_eq!(
        Target::Local.state_key().unwrap(),
        concrete.state_key().unwrap()
    );
}
