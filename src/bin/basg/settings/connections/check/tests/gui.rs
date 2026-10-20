use super::*;
use crate::settings::connections::tests::setup;
use gpui_kit::{TestAppContext, test::TestWindowExt};

fn pump(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    condition: impl Fn(&mut TestAppContext) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        runtime.block_on(async { tokio::task::yield_now().await });
        cx.run_until_parked();
        if condition(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "connection check finished"
        );
        std::thread::yield_now();
    }
}

#[gpui_kit::test]
fn card_checks_its_saved_target_without_selecting_or_writing(cx: &mut TestAppContext) {
    let (directory, window, page) = setup(cx);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    let server = runtime.block_on(Server::start(None));
    let profile = ConnectionProfile {
        id: "checked".into(),
        name: "Checked service".into(),
        note: None,
        target: server.target.clone(),
        tls: None,
    };
    let active: ConnectionOptions = "tcp://192.0.2.1".parse().expect("other service");
    cx.update(|cx| {
        cx.set_global(crate::agents::catalog::Service::new(
            active.clone(),
            runtime.handle().clone(),
        ));
        page.read(cx).profiles.clone().update(cx, |profiles, cx| {
            profiles.put(profile.clone(), cx).expect("saved profile");
        });
    });
    let path = directory.path().join("settings.json");
    let before = std::fs::read(&path).expect("settings");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("connection-check-checked", cx);
    })
    .expect("check button");
    pump(cx, &runtime, |cx| {
        cx.update(|cx| {
            matches!(
                page.read(cx).check_state(&profile),
                Some(State::Complete(_))
            )
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("connection-check-result-checked").label(),
            Some(crate::i18n::tr!("connections-check-passed"))
        );
        assert_eq!(connections::current(cx), Some(active.clone()));
        assert_eq!(
            page.read(cx)
                .profiles
                .read(cx)
                .selected()
                .expect("selected")
                .id,
            "local"
        );
        assert_eq!(std::fs::read(&path).expect("unchanged settings"), before);
        assert!(
            window.find("connection-actions-checked").bounds().right()
                <= window.bounds().size.width
        );
    })
    .expect("success result");
    server.task.abort();
    let _ = runtime.block_on(server.task);
    cx.update_window(window.into(), |_, window, cx| {
        window.click("connection-check-checked", cx);
    })
    .expect("retry check");
    pump(cx, &runtime, |cx| {
        cx.update(|cx| {
            matches!(
                page.read(cx).check_state(&profile),
                Some(State::Complete(Err(_)))
            )
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let label = window
            .find("connection-check-result-checked")
            .label()
            .expect("failed result")
            .to_owned();
        assert_eq!(label, crate::i18n::tr!("connections-check-refused"));
        assert_eq!(cx.windows().len(), 1, "check failures remain in the card");
    })
    .expect("failure result");
}

#[gpui_kit::test]
fn changed_profiles_cancel_checks_and_stale_results_cannot_replace_retries(
    cx: &mut TestAppContext,
) {
    let (_directory, window, page) = setup(cx);
    let profile = ConnectionProfile {
        id: "checked".into(),
        name: "Checked service".into(),
        note: None,
        target: "mtls://first".parse().expect("endpoint"),
        tls: Some(bas::service::tls::ClientTls {
            ca: "/ca.pem".into(),
            certificate: "/client.pem".into(),
            private_key: "/client.key".into(),
            server_name: None,
        }),
    };
    let (send, receive) = futures::channel::oneshot::channel::<()>();
    let old = cx
        .update_window(window.into(), |_, window, cx| {
            page.read(cx).profiles.clone().update(cx, |profiles, cx| {
                profiles.put(profile.clone(), cx).expect("profile");
            });
            let generation = page.update(cx, |page, cx| {
                page.begin_check(
                    profile.id.clone(),
                    profile.options(),
                    async move {
                        let _ = receive.await;
                        Ok(())
                    },
                    cx,
                );
                page.check_generation
            });
            window.render_frame(cx);
            assert_eq!(
                window.find("connection-check-checked").label(),
                Some(crate::i18n::tr!("connections-checking"))
            );
            assert!(window.try_find("connection-check-result-checked").is_none());
            generation
        })
        .expect("pending check");
    let mut changed = profile.clone();
    changed.tls.as_mut().expect("TLS").private_key = "/second-client.key".into();
    cx.update(|cx| {
        page.read(cx).profiles.clone().update(cx, |profiles, cx| {
            profiles.put(changed.clone(), cx).expect("replace target");
        });
    });
    cx.run_until_parked();
    assert!(
        send.send(()).is_err(),
        "replacing credentials/target cancels the old request"
    );
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            assert!(page.check_state(&changed).is_none());
            page.begin_check(
                changed.id.clone(),
                changed.options(),
                std::future::pending(),
                cx,
            );
            page.finish_check(&changed.id, old, Ok(()), cx);
            assert!(matches!(page.check_state(&changed), Some(State::Checking)));
            let current = page.check_generation;
            page.finish_check(&changed.id, current, Err(Error::Timeout), cx);
            assert!(matches!(
                page.check_state(&changed),
                Some(State::Complete(Err(Error::Timeout)))
            ));
            page.profiles.update(cx, |profiles, cx| {
                profiles.remove("checked", cx).expect("remove");
            });
        })
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(page.read(cx).checks.is_empty()));
}
