//! Deterministic visual fixture, without network requests or user log contents.

use super::*;

pub(crate) fn open(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    let store = Store::new();
    super::super::init(store.clone(), cx);
    for page in 1..=70 {
        store.record(
            Level::Debug,
            "basg::connection::history",
            format!(
                "Loaded history page session=preview pages={page} loaded={}",
                page * 256
            ),
        );
    }
    store.record(
        Level::Info,
        "basg::connection::history",
        "Conversation history loaded session=preview pages=75 loaded=19168 elapsed_ms=110000",
    );
    store.record(
        Level::Warn,
        "basg::connections",
        "Connection check failed endpoint=tcp://workstation error=connection refused",
    );
    store.record(
        Level::Error,
        "basg::ui",
        "读取历史失败\n服务暂时不可用。可以重新加载，未发送的草稿会保留。",
    );
    super::open(cx)
}
