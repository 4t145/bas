use super::{endpoint, label, local_endpoint, tooltip};
use bas::service::Target;

#[test]
fn local_endpoint_uses_platform_address_or_simple_fallback() {
    assert_eq!(
        local_endpoint(Ok(Target::Unix("/run/user/1000/bas.sock".into()))),
        "/run/user/1000/bas.sock"
    );
    assert_eq!(
        local_endpoint(Ok(Target::Pipe(r"\\.\pipe\bas-user".into()))),
        r"\\.\pipe\bas-user"
    );
    assert_eq!(
        local_endpoint(Err(std::io::Error::other("private environment details"))),
        crate::i18n::tr!("connections-local-endpoint")
    );
    assert_eq!(
        endpoint(&Target::Local),
        local_endpoint(Target::Local.resolved())
    );
}

#[test]
fn network_addresses_are_compact_and_security_stays_in_tooltips() {
    for protocol in ["tcp", "mtls"] {
        let target: Target = format!("{protocol}://hostname:{}", bas::service::DEFAULT_PORT)
            .parse()
            .expect("target");
        assert_eq!(endpoint(&target), "hostname");
        let custom: Target = format!("{protocol}://hostname:1234")
            .parse()
            .expect("target");
        assert_eq!(endpoint(&custom), "hostname:1234");
        assert!(tooltip(&target).contains(label(&target)));
    }
    let target: Target = "mtls://hostname".parse().expect("target");
    assert!(tooltip(&target).contains(crate::i18n::tr!("connections-mtls-security")));
}

#[cfg(feature = "gui-tests")]
#[gpui_kit::test]
fn menu_rows_keep_full_labels_inside_fixed_width(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{
        AppContext,
        component::{Root, Side, menu::PopupMenu},
        px, size,
        test::TestWindowExt,
    };
    let profile = bas::config::ConnectionProfile {
        id: "long".into(),
        name: "A very long saved name that must truncate without pushing the check out".into(),
        target: Target::Unix(
            "/run/user/1000/a-very-long-directory-name/another-long-directory/service.sock".into(),
        ),
        note: None,
        tls: None,
    };
    cx.update(gpui_kit::init);
    let window = cx.open_window(size(px(crate::ui::MENU_WIDTH), px(200.)), |window, cx| {
        let menu = PopupMenu::build(window, cx, |menu, _, _| {
            crate::ui::menu(menu)
                .check_side(Side::Right)
                .item(super::menu_item(&profile).checked(true))
        });
        Root::new(menu, window, cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let name = window.find("connection-menu-long-name");
        let address = window.find("connection-menu-long-endpoint");
        assert_eq!(name.label(), Some(profile.name.as_str()));
        assert_eq!(address.label(), Some(endpoint(&profile.target).as_str()));
        assert!(address.bounds().top() >= name.bounds().bottom());
        for item in [name, address] {
            assert!(item.bounds().size.width > px(0.));
            assert!(item.bounds().left() >= px(0.));
            assert!(item.bounds().right() < px(crate::ui::MENU_WIDTH));
        }
        assert_eq!(
            window.find("connection-menu-long").label(),
            Some(format!("{} · {}", profile.name, tooltip(&profile.target)).as_str())
        );
    })
    .expect("menu rows");
}
