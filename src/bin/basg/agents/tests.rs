use super::*;
use gpui_kit::BorrowAppContext;

#[gpui_kit::test]
fn services_and_coordination_initialize_without_a_settings_page(cx: &mut gpui_kit::TestAppContext) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("isolated runtime");
    let address: bas::service::tls::ConnectionOptions =
        "127.0.0.1".parse().expect("fixture address");
    cx.update(|cx| {
        init(address.clone(), runtime.handle().clone(), cx);
        assert_eq!(cx.global::<check::Service>().address(), address);
        assert_eq!(cx.global::<catalog::Service>().address(), address);
        cx.update_global::<Changes, _>(|changes, _| {
            changes.server = Some(address.clone());
            changes.ready = Some("fixture".into());
            changes.revision = 7;
        });
        init(address.clone(), runtime.handle().clone(), cx);
        let changes = cx.global::<Changes>();
        assert_eq!(changes.server, Some(address));
        assert_eq!(changes.ready.as_deref(), Some("fixture"));
        assert_eq!(
            changes.revision, 7,
            "service setup preserves shared coordination"
        );
    });
}
