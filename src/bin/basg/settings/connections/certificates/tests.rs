use super::*;
use gpui_kit::{TestAppContext, WindowHandle, component::Root, px, size, test::TestWindowExt};

fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, WindowHandle<Root>, Entity<Page>) {
    let directory = tempfile::tempdir().expect("settings directory");
    let path = directory.path().join("settings.json");
    std::fs::write(&path, "{}").expect("settings");
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::configuration::init(Some(path), cx);
        crate::theme::init(None, cx);
        crate::application::init(cx);
        bind_keys(cx);
    });
    let mut page = None;
    // Minimum settings width minus the sidebar.
    let window = cx.open_window(size(px(510.), px(700.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut page = Page::new(window, cx);
            page.add(window, cx);
            page.select_transport(Transport::Mtls, window, cx);
            page
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    (directory, window, page.expect("page"))
}

fn open_picker(cx: &mut TestAppContext, window: WindowHandle<Root>, field: Field) {
    cx.update_window(window.into(), |_, window, cx| {
        window.click(format!("{}-pick", field.id()), cx);
        window.simulate_next_frame(cx);
    })
    .expect("picker window");
    assert!(cx.did_prompt_for_paths());
}

#[gpui_kit::test]
fn selections_are_editable_references_and_only_save_persists(cx: &mut TestAppContext) {
    let (directory, window, page) = setup(cx);
    let settings = directory.path().join("settings.json");
    for (field, path) in [
        (Field::Ca, "/certs/ca.pem "),
        (Field::Certificate, "/certs/client.pem"),
        (Field::Key, "/certs/client.key"),
    ] {
        open_picker(cx, window, field);
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files);
            assert!(!options.directories);
            assert!(!options.multiple);
            Some(vec![path.into()])
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(field.input(page.read(cx)).read(cx).value().as_str(), path);
            assert!(!page.read(cx).certificate_picker_open);
        });
        assert_eq!(std::fs::read_to_string(&settings).expect("settings"), "{}");
    }
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.name
                .update(cx, |input, cx| input.set_value("Secure", window, cx));
            page.target
                .update(cx, |input, cx| input.set_value("example.test", window, cx));
            page.tls_key
                .update(cx, |input, cx| input.set_value("/edited.key", window, cx));
            page.save(window, cx);
        });
    })
    .expect("save");
    let saved = bas::config::Store::at(settings)
        .load()
        .expect("saved")
        .settings
        .frontend;
    let tls = saved
        .connections
        .last()
        .expect("profile")
        .tls
        .as_ref()
        .expect("TLS");
    assert_eq!(tls.ca, std::path::PathBuf::from("/certs/ca.pem "));
    assert_eq!(
        tls.certificate,
        std::path::PathBuf::from("/certs/client.pem")
    );
    assert_eq!(tls.private_key, std::path::PathBuf::from("/edited.key"));
}

#[gpui_kit::test]
fn cancellation_and_late_results_leave_drafts_untouched(cx: &mut TestAppContext) {
    let (directory, window, page) = setup(cx);
    cx.update_window(window.into(), |_, window, cx| {
        page.read(cx)
            .tls_ca
            .clone()
            .update(cx, |input, cx| input.set_value("/original.pem", window, cx));
    })
    .expect("initial value");
    open_picker(cx, window, Field::Ca);
    // A second request must not replace the native prompt's owner.
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| {
            page.choose_certificate(Field::Key, window, cx)
        });
    })
    .expect("concurrent request");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            page.read(cx).tls_ca.read(cx).value().as_str(),
            "/original.pem"
        )
    });

    for replace_editor in [false, true] {
        open_picker(cx, window, Field::Ca);
        cx.update_window(window.into(), |_, window, cx| {
            page.update(cx, |page, cx| {
                if replace_editor {
                    page.cancel(window, cx);
                    page.add(window, cx);
                } else {
                    page.select_transport(Transport::Tcp, window, cx);
                }
                page.select_transport(Transport::Mtls, window, cx);
                page.tls_ca.update(cx, |input, cx| {
                    input.set_value("/new-draft.pem", window, cx)
                });
            });
        })
        .expect("change owner");
        cx.simulate_path_prompt_response(|_| Some(vec!["/stale.pem".into()]));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(
                page.read(cx).tls_ca.read(cx).value().as_str(),
                "/new-draft.pem"
            );
            assert!(!page.read(cx).certificate_picker_open);
            assert!(page.read(cx).tls_key.read(cx).value().is_empty());
        });
    }
    assert_eq!(
        std::fs::read_to_string(directory.path().join("settings.json")).expect("settings"),
        "{}"
    );
}

#[gpui_kit::test]
fn narrow_editor_keeps_named_pickers_inside_the_existing_fields(cx: &mut TestAppContext) {
    let (_directory, window, _page) = setup(cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        for field in [Field::Ca, Field::Certificate, Field::Key] {
            let control = window.find(field.id()).bounds();
            let picker = window.find(format!("{}-pick", field.id()));
            assert_eq!(picker.label(), Some(field.picker_label()));
            assert!(picker.visible());
            assert!(picker.bounds().left() >= control.left());
            assert!(picker.bounds().right() <= control.right());
            assert!(control.right() <= px(510.));
            assert!(picker.bounds().size.width < control.size.width);
        }
        assert!(window.find("connection-tls-server-name").visible());
        assert!(window.try_find("connection-tls-server-name-pick").is_none());
    })
    .expect("layout");
}

#[cfg(unix)]
#[gpui_kit::test]
fn non_utf8_selection_is_rejected_without_replacing_the_input(cx: &mut TestAppContext) {
    use std::os::unix::ffi::OsStringExt;
    let (_directory, window, page) = setup(cx);
    open_picker(cx, window, Field::Key);
    cx.simulate_path_prompt_response(|_| {
        Some(vec![
            std::ffi::OsString::from_vec(b"/invalid-\xff.key".to_vec()).into(),
        ])
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(page.read(cx).tls_key.read(cx).value().is_empty());
        assert!(matches!(page.read(cx).feedback, Feedback::Error(_)));
    });
}
