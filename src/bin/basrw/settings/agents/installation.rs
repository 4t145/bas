//! Daemon-owned ACP discovery and installation. The GUI never executes registry commands.

use std::{net::SocketAddr, path::Path, time::Duration};

use basr::{
    management::{Catalog, Entry, Source, Status},
    service::{Client, pb},
};
use futures::{
    StreamExt,
    channel::{mpsc, oneshot},
};
use gpui_kit::{
    App, Context, FontWeight, Global, Hsla, IntoElement, ParentElement, Styled, Task,
    TestSupportExt, div, prelude::*, px,
};
use gpui_kit::{
    assets::IconName,
    component::{button::*, input::*, *},
};
use tokio::runtime::Handle;
use tokio_util::task::AbortOnDropHandle;

use super::Agents;

const CATALOG_TIMEOUT: Duration = Duration::from_secs(45);
const REGISTRY_URL: &str = "https://github.com/agentclientprotocol/registry";
const ICON_SIZE: f32 = 22.;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Filter {
    #[default]
    All,
    Installed,
    NotInstalled,
}

impl Filter {
    fn matches(self, entry: &Entry) -> bool {
        let installed = entry.spec.is_some()
            || entry.installed_version.is_some()
            || matches!(entry.status, Status::Installed | Status::Missing);
        match self {
            Self::All => true,
            Self::Installed => installed,
            Self::NotInstalled => !installed,
        }
    }
}

pub(super) struct Palette {
    pub background: Hsla,
    card: Hsla,
    border: Hsla,
    accent: Hsla,
    action_fill: Hsla,
    action_foreground: Hsla,
    muted: Hsla,
}

impl Palette {
    pub(super) fn current(cx: &App) -> Self {
        Self {
            background: cx.theme().background,
            card: cx.theme().muted,
            border: cx.theme().border,
            accent: cx.theme().primary,
            action_fill: cx.theme().primary,
            action_foreground: cx.theme().primary_foreground,
            muted: cx.theme().muted_foreground,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Service {
    server: SocketAddr,
    runtime: Handle,
}
impl Global for Service {}

pub(super) fn init(server: SocketAddr, runtime: Handle, cx: &mut App) {
    cx.set_global(Service { server, runtime });
}

pub(super) struct Operation {
    message: String,
    busy: bool,
    _task: Option<Task<()>>,
}

struct Updates {
    receiver: mpsc::UnboundedReceiver<Result<pb::AgentOperation, String>>,
    _task: AbortOnDropHandle<()>,
}

impl Service {
    pub(crate) fn new(server: SocketAddr, runtime: Handle) -> Self {
        Self { server, runtime }
    }

    pub(super) fn local(&self) -> bool {
        self.server.ip().is_loopback()
    }

    pub(crate) fn catalog(
        &self,
        refresh: bool,
    ) -> impl Future<Output = Result<Catalog, String>> + use<> {
        let server = self.server;
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(CATALOG_TIMEOUT, async {
                let mut client = Client::connect_at(server)
                    .await
                    .map_err(|error| error.to_string())?;
                let response = client
                    .list_agent_catalog(pb::ListAgentCatalogRequest { refresh })
                    .await
                    .map_err(|error| error.message().to_owned())?
                    .into_inner();
                serde_json::from_slice(&response.catalog_json).map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("读取 Agent 目录超时，请重试。".into()));
            let _ = send.send(result);
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Err("Agent 目录任务已结束。".into()))
        }
    }

    fn install(&self, id: String) -> Updates {
        let server = self.server;
        let (send, receiver) = mpsc::unbounded();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = async {
                if !server.ip().is_loopback() {
                    return Err("请在 daemon 所在主机安装 Agent。".into());
                }
                let mut client = Client::connect_at(server)
                    .await
                    .map_err(|error| error.to_string())?;
                let mut stream = client
                    .install_agent(pb::InstallAgentRequest { id })
                    .await
                    .map_err(|error| error.message().to_owned())?
                    .into_inner();
                while let Some(operation) = stream
                    .message()
                    .await
                    .map_err(|error| error.message().to_owned())?
                {
                    let complete = operation.complete;
                    if send.unbounded_send(Ok(operation)).is_err() {
                        return Ok(());
                    }
                    if complete {
                        return Ok(());
                    }
                }
                Err("安装未完成；请刷新目录后重试。".to_owned())
            }
            .await;
            if let Err(error) = result {
                let _ = send.unbounded_send(Err(error));
            }
        }));
        Updates {
            receiver,
            _task: task,
        }
    }

    fn remove(&self, id: String) -> impl Future<Output = Result<(), String>> + use<> {
        let server = self.server;
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = async {
                if !server.ip().is_loopback() {
                    return Err("请在 daemon 所在主机移除 Agent。".into());
                }
                let mut client = Client::connect_at(server)
                    .await
                    .map_err(|error| error.to_string())?;
                client
                    .remove_agent(pb::RemoveAgentRequest { id })
                    .await
                    .map_err(|error| error.message().to_owned())?;
                Ok(())
            }
            .await;
            let _ = send.send(result);
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Err("移除任务已结束。".into()))
        }
    }
}

/// Both sides must name the same file before the GUI writes local configuration.
pub(super) fn settings_match(catalog: &Catalog, path: &Path) -> bool {
    catalog.settings_path.as_ref().is_some_and(|daemon_path| {
        let canonical =
            |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        canonical(daemon_path) == canonical(path)
    })
}

fn matches_query(entry: &Entry, query: &str) -> bool {
    let text = format!(
        "{} {} {} {}",
        entry.id, entry.name, entry.description, entry.repository
    )
    .to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

fn status_label(entry: &Entry) -> &'static str {
    match entry.status {
        Status::NotInstalled => "未安装",
        Status::Installed => "已安装",
        Status::Missing => "配置的程序不可用",
        Status::Unsupported => "此平台暂不支持安装",
    }
}

impl Agents {
    pub(super) fn notify_agent_change(&self, ready: Option<String>, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<Service>().cloned() else {
            return;
        };
        if cx.try_global::<crate::settings::AgentChanges>().is_none() {
            return;
        }
        cx.update_global::<crate::settings::AgentChanges, _>(|changes, _| {
            changes.server = Some(service.server);
            changes.ready = ready;
            changes.select = None;
            changes.revision += 1;
        });
    }

    pub(super) fn begin_chat(&self, id: String, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<Service>().cloned() else {
            return;
        };
        if cx.try_global::<crate::settings::AgentChanges>().is_none() {
            return;
        }
        cx.update_global::<crate::settings::AgentChanges, _>(|changes, _| {
            changes.server = Some(service.server);
            changes.ready = Some(id.clone());
            changes.select = Some(id);
            changes.revision += 1;
        });
    }

    pub(super) fn refresh_registry(&mut self, refresh: bool, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<Service>().cloned() else {
            return;
        };
        if self.registry_loading {
            return;
        }
        self.registry_loading = true;
        let request = service.catalog(refresh);
        self.registry_task = Some(cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                page.registry_loading = false;
                match result {
                    Ok(catalog) => page.apply_catalog(catalog, service.local(), cx),
                    Err(error) => {
                        page.local_editable = false;
                        page.message = format!("无法读取 daemon 的 Agent 目录：{error}");
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn apply_catalog(&mut self, catalog: Catalog, local: bool, cx: &mut Context<Self>) {
        if let Some(service) = cx.try_global::<Service>().cloned() {
            crate::agent_metadata::publish(service.server, &catalog, cx);
        }
        self.local_editable = local && settings_match(&catalog, self.store.path());
        self.registry = Some(catalog);
        if self.local_editable {
            if !self.dirty(cx) {
                match self.store.load() {
                    Ok(document) => self.document = Some(document),
                    Err(error) => self.message = format!("读取本机配置失败：{error}"),
                }
            }
            let specs = self.catalog();
            self.checks
                .retain(|id, _| specs.iter().any(|spec| spec.id == *id));
            for spec in specs {
                if self
                    .checks
                    .get(&spec.id)
                    .is_none_or(|check| check.spec != spec)
                {
                    self.start_check(spec, false, cx);
                }
            }
        } else {
            self.message = if local {
                "daemon 使用的配置文件与本机设置不同。目录可以查看；请让 basrw 与 basrd 使用相同的 --settings 路径后再编辑或安装。".into()
            } else {
                "正在查看远端 daemon 的 Agent 目录。安装、移除和认证需在 daemon 所在主机完成。"
                    .into()
            };
        }
    }

    fn managed_allowed(&self, cx: &App) -> bool {
        self.local_editable
            && !self.dirty(cx)
            && self
                .registry
                .as_ref()
                .is_some_and(|catalog| catalog.can_install)
    }

    fn install(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.managed_allowed(cx) || self.operations.values().any(|operation| operation.busy) {
            return;
        }
        let Some(service) = cx.try_global::<Service>().cloned() else {
            return;
        };
        self.start_install_updates(id.clone(), service.install(id), cx);
    }

    fn start_install_updates(&mut self, id: String, updates: Updates, cx: &mut Context<Self>) {
        let Updates {
            mut receiver,
            _task: install_task,
        } = updates;
        let key = id.clone();
        let task = cx.spawn(async move |page, cx| {
            // Keep the RPC producer alive; capturing only `receiver` drops its abort guard.
            let _install_task = install_task;
            while let Some(update) = receiver.next().await {
                let finished = match &update {
                    Ok(operation) => operation.complete,
                    Err(_) => true,
                };
                let _ = page.update(cx, |page, cx| {
                    let message = match &update {
                        Ok(operation) if operation.complete => "安装完成".into(),
                        Ok(operation) => operation.message.clone(),
                        Err(error) => format!("安装失败：{error}"),
                    };
                    if let Some(operation) = page.operations.get_mut(&key) {
                        operation.message = message;
                        operation.busy = !finished;
                    }
                    if let Ok(operation) = update
                        && operation.complete
                    {
                        page.notify_agent_change(None, cx);
                        if let Ok(spec) =
                            serde_json::from_slice::<basr::config::AgentSpec>(&operation.spec_json)
                        {
                            page.start_check(spec, true, cx);
                        }
                    }
                    if finished {
                        page.refresh_registry(false, cx);
                    }
                    cx.notify();
                });
                if finished {
                    return;
                }
            }
            let _ = page.update(cx, |page, cx| {
                if let Some(operation) = page.operations.get_mut(&key)
                    && operation.busy
                {
                    operation.busy = false;
                    operation.message =
                        "安装连接已结束，尚未收到完成结果。请刷新目录后重试。".into();
                }
                page.refresh_registry(false, cx);
                cx.notify();
            });
        });
        self.operations.insert(
            id,
            Operation {
                message: "正在准备安装…".into(),
                busy: true,
                _task: Some(task),
            },
        );
        cx.notify();
    }

    fn cancel_install(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(operation) = self.operations.get_mut(id) {
            operation._task.take();
            operation.busy = false;
            operation.message = "已取消，正在刷新安装状态。".into();
        }
        self.refresh_registry(false, cx);
        cx.notify();
    }

    fn remove_managed(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.managed_allowed(cx) || self.operations.values().any(|operation| operation.busy) {
            return;
        }
        let Some(service) = cx.try_global::<Service>().cloned() else {
            return;
        };
        let request = service.remove(id.clone());
        let key = id.clone();
        let task = cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                if let Some(operation) = page.operations.get_mut(&key) {
                    operation.busy = false;
                    operation.message = match result {
                        Ok(()) => "已移除安装和启动配置；已有会话仍保留。".into(),
                        Err(error) => format!("移除失败：{error}"),
                    };
                }
                page.notify_agent_change(None, cx);
                page.refresh_registry(false, cx);
                cx.notify();
            });
        });
        self.operations.insert(
            id,
            Operation {
                message: "正在移除…".into(),
                busy: true,
                _task: Some(task),
            },
        );
        cx.notify();
    }

    fn registry_icon(entry: &Entry, palette: &Palette) -> gpui_kit::AnyElement {
        div()
            .size(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .text_color(palette.accent)
            .child(
                crate::agent_icon::AgentIcon::new(
                    format!("agents-icon-{}", entry.id),
                    entry.icon.clone(),
                )
                .size(px(ICON_SIZE))
                .text_color(palette.accent),
            )
            .into_any_element()
    }

    fn registry_links(&self, entry: &Entry, palette: &Palette) -> gpui_kit::AnyElement {
        let repository = entry.repository.clone();
        let license = entry.license_url.clone();
        div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap_2()
            .text_xs()
            .text_color(palette.muted)
            .child(div().child(format!("ID: {}", entry.id)))
            .when(!repository.is_empty(), |links| {
                links.child(
                    Button::new(format!("agents-repository-{}", entry.id))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Github)
                        .tooltip("Repository")
                        .text_color(palette.muted)
                        .on_click(move |_, _, cx| cx.open_url(&repository)),
                )
            })
            .when(!entry.license.is_empty(), |links| {
                links.child(
                    Button::new(format!("agents-license-{}", entry.id))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Scale)
                        .tooltip(format!("License: {}", entry.license))
                        .text_color(palette.muted)
                        .disabled(license.is_empty())
                        .on_click(move |_, _, cx| cx.open_url(&license)),
                )
            })
            .into_any_element()
    }

    fn registry_actions(&self, entry: &Entry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let status = entry.spec.as_ref().and_then(|spec| self.status(spec));
        let busy = self
            .operations
            .get(&entry.id)
            .is_some_and(|operation| operation.busy);
        let any_busy = self.operations.values().any(|operation| operation.busy);
        let update = entry
            .installed_version
            .as_ref()
            .is_some_and(|version| version != &entry.version);
        let palette = Palette::current(cx);
        let action = if busy {
            let id = entry.id.clone();
            super::button(format!("agents-install-cancel-{}", entry.id))
                .label("取消")
                .on_click(cx.listener(move |this, _, _, cx| this.cancel_install(&id, cx)))
        } else if entry.source == Source::Registry
            && (matches!(entry.status, Status::NotInstalled | Status::Missing) || update)
        {
            let id = entry.id.clone();
            let disabled =
                !self.managed_allowed(cx) || any_busy || entry.status == Status::Unsupported;
            super::button(format!("agents-install-{}", entry.id))
                .primary()
                .label(if update { "更新" } else { "安装" })
                .icon(IconName::Download)
                .when(!disabled, |button| {
                    button
                        .bg(palette.action_fill)
                        .text_color(palette.action_foreground)
                })
                .disabled(disabled)
                .on_click(cx.listener(move |this, _, _, cx| this.install(id.clone(), cx)))
        } else if let Some(spec) = entry.spec.clone() {
            let id = entry.id.clone();
            let embedded = spec.kind == basr::config::agent::Kind::Embedded;
            super::button(format!("agents-registry-check-{}", entry.id))
                .label(if embedded {
                    "开始对话"
                } else {
                    status.map_or("检测", super::check::Status::check_label)
                })
                .tooltip(if embedded {
                    "使用内置后端开始对话。".into()
                } else {
                    self.check_detail(&spec)
                })
                .text_color(palette.accent)
                .disabled(
                    !self.local_editable
                        || self.dirty(cx)
                        || status.is_some_and(super::check::Status::busy),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if embedded {
                        this.begin_chat(id.clone(), cx);
                    } else {
                        this.start_check(spec.clone(), true, cx);
                    }
                }))
        } else {
            super::button(format!("agents-registry-unavailable-{}", entry.id))
                .label(status_label(entry))
                .disabled(true)
        };
        let remove = entry.id.clone();
        div()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .child(action)
            .when(
                entry
                    .spec
                    .as_ref()
                    .is_some_and(|spec| spec.kind != basr::config::agent::Kind::Embedded)
                    && status.is_some_and(super::check::Status::connected)
                    && !busy,
                |actions| {
                    let id = entry.id.clone();
                    actions.child(
                        Button::new(format!("agents-registry-start-chat-{}", entry.id))
                            .ghost()
                            .xsmall()
                            .label("开始对话")
                            .text_color(palette.accent)
                            .disabled(!self.local_editable || self.dirty(cx))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.begin_chat(id.clone(), cx)),
                            ),
                    )
                },
            )
            .when(entry.installed_version.is_some() && !busy, |actions| {
                actions.child(
                    Button::new(format!("agents-remove-{}", entry.id))
                        .ghost()
                        .xsmall()
                        .label("移除")
                        .disabled(!self.managed_allowed(cx) || any_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_managed(remove.clone(), cx)
                        })),
                )
            })
            .into_any_element()
    }

    fn registry_card(&self, entry: &Entry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = Palette::current(cx);
        let status = entry.spec.as_ref().and_then(|spec| self.status(spec));
        let has_authentication = entry
            .spec
            .as_ref()
            .is_some_and(|spec| self.authentications.contains_key(&spec.id));
        let status_label = if entry.source == Source::Embedded {
            "内置可用"
        } else {
            status.map_or("已安装 · 未检测", super::check::Status::label)
        };
        let check_detail = entry.spec.as_ref().map(|spec| self.check_detail(spec));
        let operation = self.operations.get(&entry.id);
        let detail = operation
            .filter(|operation| operation.busy)
            .map(|operation| operation.message.clone())
            .or_else(|| match status.filter(|_| !has_authentication) {
                Some(
                    super::check::Status::Failed(message) | super::check::Status::Missing(message),
                ) => Some(message.clone()),
                _ => None,
            })
            .or_else(|| operation.map(|operation| operation.message.clone()))
            .or_else(|| {
                (!entry.message.is_empty()).then(|| {
                    if entry.status == Status::Installed && entry.source == Source::Registry {
                        "安装完成".into()
                    } else {
                        entry.message.clone()
                    }
                })
            });
        div()
            .id(format!("agents-registry-{}", entry.id))
            .test_support()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .bg(palette.card)
            .border_1()
            .border_color(palette.border)
            .rounded_md()
            .child(
                div()
                    .id(format!("agents-registry-heading-{}", entry.id))
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(Self::registry_icon(entry, &palette))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(entry.name.clone()),
                            )
                            .when(!entry.version.is_empty(), |title| {
                                title.child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.muted)
                                        .child(entry.version.clone()),
                                )
                            })
                            .when(
                                !has_authentication
                                    && (status.is_some() || entry.status == Status::Installed),
                                |title| {
                                    title.child(crate::tooltip::bounded(
                                        format!("agents-registry-status-hint-{}", entry.id),
                                        div()
                                            .id(format!("agents-registry-status-{}", entry.id))
                                            .test_support()
                                            .text_xs()
                                            .text_color(palette.accent)
                                            .child(status_label),
                                        check_detail.clone().unwrap_or_else(|| {
                                            "点击「检测」检查程序是否能正常启动、是否需要登录。"
                                                .into()
                                        }),
                                    ))
                                },
                            ),
                    )
                    .child(self.registry_actions(entry, cx)),
            )
            .child(
                div()
                    .id(format!("agents-registry-details-{}", entry.id))
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .text_xs()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_color(palette.muted)
                            .child(entry.description.clone()),
                    )
                    .child(self.registry_links(entry, &palette)),
            )
            .when_some(detail, |card, detail| {
                card.child(
                    div()
                        .id(format!("agents-operation-{}", entry.id))
                        .test_support()
                        .text_xs()
                        .text_color(palette.accent)
                        .min_w_0()
                        .child(detail),
                )
            })
            .when_some(
                entry.spec.as_ref().filter(|spec| {
                    matches!(
                        status,
                        Some(super::check::Status::AuthenticationRequired { .. })
                    ) || self.authentications.contains_key(&spec.id)
                }),
                |card, spec| {
                    let index = self
                        .catalog()
                        .iter()
                        .position(|configured| configured == spec)
                        .unwrap_or(0);
                    card.child(
                        div()
                            .pt_2()
                            .border_t_1()
                            .border_color(palette.border)
                            .child(self.authentication_content(spec, index, cx)),
                    )
                },
            )
            .into_any_element()
    }

    pub(super) fn registry_header(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let palette = Palette::current(cx);
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().text_xl().child("ACP Registry"))
                            .child(
                                Button::new("agents-registry-learn-more")
                                    .ghost()
                                    .xsmall()
                                    .label("了解更多")
                                    .text_color(palette.accent)
                                    .icon(IconName::ExternalLink)
                                    .on_click(|_, _, cx| cx.open_url(REGISTRY_URL)),
                            ),
                    )
                    .child(
                        Button::new("agents-configure")
                            .ghost()
                            .xsmall()
                            .label("高级配置")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_installation = false;
                                this.scroll_to = Some(0);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("agents-registry-search")
                    .test_support()
                    .w_full()
                    .p_1()
                    .bg(palette.card)
                    .border_1()
                    .border_color(palette.border)
                    .rounded_md()
                    .child(
                        Input::new(&self.registry_search)
                            .appearance(false)
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).small())
                            .aria_label("搜索 ACP Agent"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        ButtonGroup::new("agents-registry-filters")
                            .outline()
                            .small()
                            .children([
                                Button::new("agents-filter-all")
                                    .label("全部")
                                    .selected(self.registry_filter == Filter::All)
                                    .when(self.registry_filter == Filter::All, |button| {
                                        button
                                            .bg(palette.action_fill)
                                            .text_color(palette.action_foreground)
                                            .border_color(palette.action_fill)
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.registry_filter = Filter::All;
                                        cx.notify();
                                    })),
                                Button::new("agents-filter-installed")
                                    .label("已安装")
                                    .selected(self.registry_filter == Filter::Installed)
                                    .when(self.registry_filter == Filter::Installed, |button| {
                                        button
                                            .bg(palette.action_fill)
                                            .text_color(palette.action_foreground)
                                            .border_color(palette.action_fill)
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.registry_filter = Filter::Installed;
                                        cx.notify();
                                    })),
                                Button::new("agents-filter-not-installed")
                                    .label("未安装")
                                    .selected(self.registry_filter == Filter::NotInstalled)
                                    .when(self.registry_filter == Filter::NotInstalled, |button| {
                                        button
                                            .bg(palette.action_fill)
                                            .text_color(palette.action_foreground)
                                            .border_color(palette.action_fill)
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.registry_filter = Filter::NotInstalled;
                                        cx.notify();
                                    })),
                            ]),
                    )
                    .child(
                        Button::new("agents-registry-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .label(if self.registry_loading {
                                "刷新中…"
                            } else {
                                "刷新目录"
                            })
                            .disabled(self.registry_loading)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.refresh_registry(true, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn registry_content(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let query = format!("{} {}", self.query, self.registry_search.read(cx).value());
        let entries: Vec<_> = self
            .registry
            .as_ref()
            .map(|catalog| {
                catalog
                    .entries
                    .iter()
                    .filter(|entry| {
                        (entry.source == Source::Registry || !self.local_editable)
                            && self.registry_filter.matches(entry)
                            && matches_query(entry, &query)
                    })
                    .collect()
            })
            .unwrap_or_default();
        div()
            .id("agents-registry")
            .test_support()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(
                self.registry
                    .as_ref()
                    .and_then(|catalog| catalog.warning.as_ref()),
                |panel, warning| {
                    panel.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(warning.clone()),
                    )
                },
            )
            .when(entries.is_empty(), |panel| {
                panel.child(
                    div()
                        .py_4()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(if self.registry_loading {
                            "正在读取 Agent 目录…"
                        } else if self.registry.is_none() {
                            "连接 basrd 后即可加载可安装的 Agent。"
                        } else {
                            "没有匹配的 Agent。"
                        }),
                )
            })
            .children(
                entries
                    .into_iter()
                    .map(|entry| self.registry_card(entry, cx)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
