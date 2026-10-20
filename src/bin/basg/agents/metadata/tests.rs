use super::*;
use bas::management::{Entry, Status};
use gpui_kit::TestAppContext;

fn entry(id: &str, source: Source, icon: &str, spec: Option<AgentSpec>) -> Entry {
    Entry {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        version: "1.0.0".into(),
        repository: String::new(),
        license: String::new(),
        license_url: String::new(),
        icon: icon.into(),
        installed_version: None,
        source,
        status: Status::Installed,
        message: String::new(),
        spec,
    }
}

fn catalog(entries: Vec<Entry>) -> Catalog {
    Catalog {
        entries,
        warning: None,
        settings_path: None,
        can_install: false,
    }
}

fn address(port: u16) -> Target {
    bas::service::Target::from(std::net::SocketAddr::from(([127, 0, 0, 1], port))).into()
}

#[gpui_kit::test]
fn ipc_and_tcp_metadata_have_distinct_cache_entries(cx: &mut TestAppContext) {
    let secure = |identity: &str| bas::service::tls::ConnectionOptions {
        target: "mtls://same-server:7338".parse().expect("target"),
        tls: Some(bas::service::tls::ClientTls {
            ca: "/certs/ca.pem".into(),
            certificate: format!("/certs/{identity}.pem").into(),
            private_key: format!("/certs/{identity}.key").into(),
            server_name: None,
        }),
    };
    let targets = [
        bas::service::Target::Local.into(),
        bas::service::Target::Unix("/tmp/first.sock".into()).into(),
        bas::service::Target::Unix("/tmp/second.sock".into()).into(),
        bas::service::Target::Pipe("bas-test".into()).into(),
        address(bas::service::DEFAULT_PORT),
        secure("first"),
        secure("second"),
    ];
    cx.update(|cx| {
        for (index, target) in targets.iter().enumerate() {
            for_test(
                target.clone(),
                &catalog(vec![entry(
                    "agent",
                    Source::Registry,
                    &format!("{index}.svg"),
                    None,
                )]),
                cx,
            );
        }
        let shared = entity(cx).expect("shared metadata");
        for (index, target) in targets.iter().enumerate() {
            assert_eq!(
                shared.read(cx).icon(target, "agent"),
                Some(format!("{index}.svg")),
            );
        }
    });
}

#[gpui_kit::test]
fn metadata_is_shared_but_isolated_by_server_and_retains_removed_thread_artwork(
    cx: &mut TestAppContext,
) {
    let first = address(bas::service::DEFAULT_PORT);
    let second = address(bas::service::DEFAULT_PORT.wrapping_add(1));
    cx.update(|cx| {
        for_test(
            first.clone(),
            &catalog(vec![entry(
                "same-agent",
                Source::Registry,
                "first.svg",
                None,
            )]),
            cx,
        )
    });
    let shared = cx.update(|cx| entity(cx).expect("shared metadata"));
    cx.update(|cx| {
        for_test(
            second.clone(),
            &catalog(vec![entry(
                "same-agent",
                Source::Registry,
                "second.svg",
                None,
            )]),
            cx,
        );
        assert_eq!(
            entity(cx).expect("same global entity").entity_id(),
            shared.entity_id()
        );
        let metadata = shared.read(cx);
        assert_eq!(
            metadata.icon(&first, "same-agent").as_deref(),
            Some("first.svg")
        );
        assert_eq!(
            metadata.icon(&second, "same-agent").as_deref(),
            Some("second.svg")
        );
        assert_eq!(
            metadata.icon(&first, "embedded").as_deref(),
            Some(crate::assets::OUTLINE)
        );
        assert!(metadata.icon(&first, "old-unknown").is_none());
        publish(first.clone(), &catalog(Vec::new()), cx);
        assert_eq!(
            shared.read(cx).icon(&first, "same-agent").as_deref(),
            Some("first.svg")
        );
    });
}

#[gpui_kit::test]
fn pending_catalog_task_survives_initialization_and_failure_preserves_loaded_icons(
    cx: &mut TestAppContext,
) {
    let server = address(bas::service::DEFAULT_PORT);
    cx.update(|cx| {
        for_test(
            server.clone(),
            &catalog(vec![entry("pi-acp", Source::Registry, "old.svg", None)]),
            cx,
        )
    });
    let shared = cx.update(|cx| entity(cx).expect("shared metadata"));
    let (send, receive) = futures::channel::oneshot::channel();
    shared.update(cx, |metadata, cx| {
        metadata
            .servers
            .get_mut(&server)
            .expect("server state")
            .loaded = false;
        metadata.begin_request(
            server.clone(),
            async move {
                receive
                    .await
                    .map_err(|_| "fixture request cancelled".to_owned())?
            },
            cx,
        );
        metadata.ensure(server.clone(), cx);
        assert!(
            !metadata.servers[&server].queued,
            "another window must not queue a duplicate initial request"
        );
    });
    cx.run_until_parked();
    send.send(Ok(catalog(vec![entry(
        "pi-acp",
        Source::Registry,
        "new.svg",
        None,
    )])))
    .expect("catalog task must retain its pending request");
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shared.read(cx).icon(&server, "pi-acp").as_deref(),
            Some("new.svg")
        );
        assert!(!shared.read(cx).servers[&server].requested);
        assert!(shared.read(cx).servers[&server].task.is_none());
        shared.update(cx, |metadata, cx| {
            metadata.begin_request(
                server.clone(),
                std::future::ready(Err("fixture offline".into())),
                cx,
            )
        });
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shared.read(cx).icon(&server, "pi-acp").as_deref(),
            Some("new.svg")
        )
    });
}
