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

fn address(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

#[test]
fn configured_pi_uses_exact_executable_identity_without_guessing_user_ids() {
    let mut icons = BTreeMap::new();
    let mut commands = BTreeMap::new();
    let entries = catalog(vec![
        entry(
            "pi-acp",
            Source::Registry,
            "https://example.com/pi.svg",
            None,
        ),
        entry(
            "pi",
            Source::Custom,
            "",
            Some(AgentSpec::acp(
                "pi",
                Command::new("/home/user/.npm-global/bin/pi-acp", []),
            )),
        ),
        entry(
            "other",
            Source::Custom,
            "",
            Some(AgentSpec::acp(
                "other",
                Command::new("/home/user/bin/my-pi-acp-wrapper", []),
            )),
        ),
    ]);
    assert!(merge_icons(&mut icons, &mut commands, &entries));
    assert_eq!(icons.get("pi"), icons.get("pi-acp"));
    assert!(!icons.contains_key("other"));
    let unrelated = catalog(vec![entry(
        "pi",
        Source::Custom,
        "",
        Some(AgentSpec::acp("pi", Command::new("unrelated-backend", []))),
    )]);
    let mut fresh = BTreeMap::new();
    assert!(!merge_icons(&mut fresh, &mut BTreeMap::new(), &unrelated));
    assert!(!fresh.contains_key("pi"));
}

#[test]
fn changing_a_custom_backend_clears_its_old_alias_but_partial_catalogs_and_removed_ids_keep_it() {
    let official = entry("pi-acp", Source::Registry, "pi.svg", None);
    let configured = entry(
        "pi",
        Source::Custom,
        "",
        Some(AgentSpec::acp("pi", Command::new("/private/pi-acp", []))),
    );
    let mut icons = BTreeMap::new();
    let mut commands = BTreeMap::new();
    merge_icons(
        &mut icons,
        &mut commands,
        &catalog(vec![official.clone(), configured.clone()]),
    );
    assert_eq!(icons.get("pi").map(String::as_str), Some("pi.svg"));
    merge_icons(&mut icons, &mut commands, &catalog(vec![official.clone()]));
    assert!(
        icons.contains_key("pi"),
        "removed configurations can still own threads"
    );
    let changed = entry(
        "pi",
        Source::Custom,
        "",
        Some(AgentSpec::acp(
            "pi",
            Command::new("/private/unrelated-backend", []),
        )),
    );
    merge_icons(&mut icons, &mut commands, &catalog(vec![changed.clone()]));
    assert!(
        icons.contains_key("pi"),
        "a registry outage cannot prove the new command is unknown"
    );
    merge_icons(&mut icons, &mut commands, &catalog(vec![official, changed]));
    assert!(
        !icons.contains_key("pi"),
        "the current unrelated backend must use its fallback icon"
    );
    assert!(icons.contains_key("pi-acp"));
}

#[test]
fn managed_node_entrypoints_and_windows_executable_suffixes_match_exact_packages() {
    let official = BTreeMap::from([("codex-acp", "codex.svg"), ("pi-acp", "pi.svg")]);
    let node = AgentSpec::acp(
        "custom-codex",
        Command::new(
            "/private/bin/node",
            ["/private/agent/node_modules/@agentclientprotocol/codex-acp/dist/index.js".into()],
        ),
    );
    assert_eq!(command_icon(&node, &official), Some("codex.svg"));
    let wrong = AgentSpec::acp(
        "custom-codex",
        Command::new(
            "/private/bin/node",
            ["/private/not-node_modules/codex-acp/dist/index.js".into()],
        ),
    );
    assert!(command_icon(&wrong, &official).is_none());
    let windows = AgentSpec::acp("custom-pi", Command::new("pi-acp.exe", []));
    assert_eq!(command_icon(&windows, &official), Some("pi.svg"));
}

#[gpui_kit::test]
fn metadata_is_shared_but_isolated_by_server_and_retains_removed_thread_artwork(
    cx: &mut TestAppContext,
) {
    let first = address(7337);
    let second = address(7338);
    cx.update(|cx| {
        for_test(
            first,
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
            second,
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
            metadata.icon(first, "same-agent").as_deref(),
            Some("first.svg")
        );
        assert_eq!(
            metadata.icon(second, "same-agent").as_deref(),
            Some("second.svg")
        );
        assert_eq!(
            metadata.icon(first, "embedded").as_deref(),
            Some(crate::assets::OUTLINE)
        );
        assert!(metadata.icon(first, "old-unknown").is_none());
        publish(first, &catalog(Vec::new()), cx);
        assert_eq!(
            shared.read(cx).icon(first, "same-agent").as_deref(),
            Some("first.svg")
        );
    });
}

#[gpui_kit::test]
fn pending_catalog_task_survives_initialization_and_failure_preserves_loaded_icons(
    cx: &mut TestAppContext,
) {
    let server = address(7337);
    cx.update(|cx| {
        for_test(
            server,
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
            server,
            async move {
                receive
                    .await
                    .map_err(|_| "fixture request cancelled".to_owned())?
            },
            cx,
        );
        metadata.ensure(server, cx);
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
            shared.read(cx).icon(server, "pi-acp").as_deref(),
            Some("new.svg")
        );
        assert!(!shared.read(cx).servers[&server].requested);
        assert!(shared.read(cx).servers[&server].task.is_none());
        shared.update(cx, |metadata, cx| {
            metadata.begin_request(
                server,
                std::future::ready(Err("fixture offline".into())),
                cx,
            )
        });
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            shared.read(cx).icon(server, "pi-acp").as_deref(),
            Some("new.svg")
        )
    });
}
