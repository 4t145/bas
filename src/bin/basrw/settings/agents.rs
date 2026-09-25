//! Local catalog onboarding; only an explicit connection action starts an ACP probe.

use super::action_button as button;
mod authentication;
mod check;
mod installation;
pub(crate) use authentication::run_helper_if_requested;
use basr::config::{
    Document, Store,
    agent::{AgentSpec, Command, Kind, ValidationError},
    validate_agents,
};
pub(crate) use check::init as init_checks;
use gpui_kit::{
    App, Context, Entity, Render, ScrollHandle, Subscription, Task, TestSupportExt, Window,
    component::{group_box::*, input::*, *},
    div,
    prelude::*,
};
use std::{collections::HashMap, time::Duration};

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

struct Check {
    spec: AgentSpec,
    status: check::Status,
    _task: Option<Task<()>>,
}

#[derive(PartialEq, Eq)]
struct Form<T> {
    id: T,
    program: T,
    args: T,
    grace: T,
}

impl<T> Form<T> {
    fn fields(&self) -> [&T; 4] {
        [&self.id, &self.program, &self.args, &self.grace]
    }
}

impl Form<Entity<InputState>> {
    fn new(window: &mut Window, cx: &mut Context<Agents>) -> Self {
        Self {
            id: cx.new(|cx| InputState::new(window, cx)),
            program: cx.new(|cx| InputState::new(window, cx).placeholder("可执行文件的绝对路径")),
            args: cx.new(|cx| InputState::new(window, cx).placeholder("[\"acp\"]")),
            grace: cx.new(|cx| InputState::new(window, cx)),
        }
    }

    fn values(&self, cx: &App) -> Form<String> {
        Form {
            id: self.id.read(cx).value().to_string(),
            program: self.program.read(cx).value().to_string(),
            args: self.args.read(cx).value().to_string(),
            grace: self.grace.read(cx).value().to_string(),
        }
    }

    fn rows(
        &self,
        embedded: bool,
        connected: bool,
    ) -> Vec<(&'static str, &'static str, &Entity<InputState>)> {
        let mut rows = vec![("agents-id", "Agent ID", &self.id)];
        if !embedded {
            rows.push((
                "agents-program",
                "可执行文件（名称或绝对路径）",
                &self.program,
            ));
            rows.push((
                "agents-args",
                "参数（JSON 字符串数组，例如 [\"含 空格的参数\"]）",
                &self.args,
            ));
        }
        if connected {
            rows.push(("agents-grace", "取消等待时间（毫秒）", &self.grace));
        }
        rows
    }
}

fn values(spec: &AgentSpec) -> Form<String> {
    Form {
        id: spec.id.to_string(),
        program: spec
            .command
            .as_ref()
            .map(|command| command.program.to_string_lossy().into_owned())
            .unwrap_or_default(),
        args: serde_json::to_string(
            &spec
                .command
                .as_ref()
                .map(|command| command.args.as_slice())
                .unwrap_or_default(),
        )
        .expect("string arrays serialize"),
        grace: spec.cancellation_grace.as_millis().to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
enum DraftError {
    #[error("尚未读取配置")]
    NotLoaded,
    #[error("请先选择或添加 Agent")]
    NotSelected,
    #[error("未保存的 Agent 无需删除；请取消编辑")]
    UnsavedDeletion,
    #[error("参数必须是 JSON 字符串数组：{0}")]
    Arguments(#[source] serde_json::Error),
    #[error("取消等待时间必须是正整数毫秒：{0}")]
    Grace(#[source] std::num::ParseIntError),
    #[error("{0}")]
    Validation(#[from] ValidationError),
    #[error("{0}")]
    Store(#[from] basr::config::Error),
}

pub(super) struct Agents {
    store: Store,
    document: Option<Document>,
    selected: Option<usize>,
    draft: Option<AgentSpec>,
    deleting: bool,
    form: Form<Entity<InputState>>,
    message: String,
    /// Section the sidebar asked to reveal; consumed by the next render.
    pub(super) scroll_to: Option<usize>,
    scroll: ScrollHandle,
    checks: HashMap<basr::model::AgentId, Check>,
    show_installation: bool,
    _subscriptions: Vec<Subscription>,
}

/// One navigable section of the agents page.
pub(super) struct Section {
    pub id: String,
    pub title: String,
}

impl Agents {
    pub(super) fn new(store: Store, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let form = Form::new(window, cx);
        let subscriptions = form
            .fields()
            .into_iter()
            .map(|input| cx.subscribe(input, |_, _, _: &InputEvent, cx| cx.notify()))
            .collect();
        let mut page = Self {
            store,
            document: None,
            selected: None,
            draft: None,
            deleting: false,
            form,
            message: String::new(),
            scroll_to: None,
            scroll: ScrollHandle::new(),
            checks: HashMap::new(),
            show_installation: false,
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        page
    }

    fn catalog(&self) -> Vec<AgentSpec> {
        self.document
            .as_ref()
            .map(|document| document.settings.daemon.agents.clone())
            .unwrap_or_default()
    }

    fn open_installation(&mut self, cx: &mut Context<Self>) {
        self.show_installation = true;
        self.scroll_to = Some(self.catalog().len());
        cx.notify();
    }

    pub(super) fn reveal(&mut self, id: &str, cx: &mut Context<Self>) -> Option<usize> {
        let entry = self
            .catalog()
            .into_iter()
            .enumerate()
            .find(|(_, spec)| spec.id.as_str() == id);
        if let Some((index, spec)) = entry {
            self.scroll_to = Some(index);
            self.start_check(spec, true, cx);
            Some(index)
        } else {
            self.message = "本机设置中没有这个 Agent；请确认当前连接的 daemon 与配置文件。".into();
            cx.notify();
            None
        }
    }

    fn start_check(&mut self, spec: AgentSpec, connect: bool, cx: &mut Context<Self>) {
        if spec.kind == Kind::Embedded {
            self.checks.insert(
                spec.id.clone(),
                Check {
                    spec,
                    status: check::Status::Connected(String::new()),
                    _task: None,
                },
            );
            return;
        }
        let Some(service) = cx.try_global::<check::Service>().cloned() else {
            self.checks.insert(
                spec.id.clone(),
                Check {
                    spec,
                    status: check::Status::Failed("本机服务检测未启用。".into()),
                    _task: None,
                },
            );
            return;
        };
        let request = service.request(spec.clone(), connect);
        let expected = spec.clone();
        let task = cx.spawn(async move |page, cx| {
            let result = request.await;
            let _ = page.update(cx, |page, cx| {
                page.apply_check(&expected, result);
                cx.notify();
            });
        });
        self.checks.insert(
            spec.id.clone(),
            Check {
                spec,
                status: if connect {
                    check::Status::Connecting
                } else {
                    check::Status::Checking
                },
                _task: Some(task),
            },
        );
        cx.notify();
    }

    fn apply_check(&mut self, expected: &AgentSpec, result: check::Status) {
        if let Some(check) = self.checks.get_mut(&expected.id)
            && check.spec == *expected
        {
            check.status = result;
        }
    }

    fn status(&self, spec: &AgentSpec) -> Option<&check::Status> {
        self.checks
            .get(&spec.id)
            .filter(|check| check.spec == *spec)
            .map(|check| &check.status)
    }

    fn draft_connected(&self, cx: &App) -> bool {
        let Some(draft) = &self.draft else {
            return false;
        };
        if draft.kind == Kind::Embedded {
            return true;
        }
        self.checks.get(&draft.id).is_some_and(|check| {
            let checked = values(&check.spec);
            let form = self.form.values(cx);
            check.status.connected()
                && checked.id == form.id
                && checked.program == form.program
                && checked.args == form.args
                && check.spec.env == draft.env
        })
    }

    /// Sidebar entries: every configured agent, then the new ACP form.
    pub(super) fn sections(&self) -> Vec<Section> {
        let mut sections: Vec<_> = self
            .catalog()
            .iter()
            .enumerate()
            .map(|(index, spec)| Section {
                id: format!("settings-agent-{index}"),
                title: spec.id.to_string(),
            })
            .collect();
        sections.push(Section {
            id: "settings-agents-add".to_owned(),
            title: "添加 Agent".to_owned(),
        });
        sections
    }

    fn dirty(&self, cx: &App) -> bool {
        self.draft.as_ref().is_some_and(|draft| {
            self.selected.is_none() || self.deleting || self.form.values(cx) != values(draft)
        })
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.dirty(cx) {
            self.message = "有未保存草稿；请先保存或取消编辑，再重新加载。".into();
        } else {
            match self.store.load() {
                Ok(document) => {
                    self.document = Some(document);
                    self.draft = None;
                    self.selected = None;
                    self.deleting = false;
                    self.message.clear();
                    self.checks.clear();
                    for spec in self.catalog() {
                        self.start_check(spec, false, cx);
                    }
                }
                Err(error) => self.message = format!("读取失败：{error}"),
            }
        }
        cx.notify();
    }

    fn edit(
        &mut self,
        spec: AgentSpec,
        selected: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dirty(cx) {
            self.message = "请先保存或取消当前草稿，再选择其他 Agent。".into();
        } else {
            let form = values(&spec);
            for (input, value) in self.form.fields().into_iter().zip(form.fields()) {
                input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            }
            self.draft = Some(spec);
            self.selected = selected;
            self.deleting = false;
            self.message.clear();
            self.scroll_to = Some(selected.unwrap_or_else(|| self.catalog().len()));
        }
        cx.notify();
    }

    /// Build the complete catalog without modifying the document or disk.
    ///
    /// # Errors
    /// Rejects missing editor state, malformed form values, and invalid catalogs.
    fn candidate(&self, cx: &App) -> Result<Vec<AgentSpec>, DraftError> {
        let mut catalog = self
            .document
            .as_ref()
            .ok_or(DraftError::NotLoaded)?
            .settings
            .daemon
            .agents
            .clone();
        let mut spec = self.draft.clone().ok_or(DraftError::NotSelected)?;
        if self.deleting {
            catalog.remove(self.selected.ok_or(DraftError::UnsavedDeletion)?);
        } else {
            let form = self.form.values(cx);
            spec.id = form.id.into();
            if spec.kind == Kind::Acp {
                let program = form.program;
                let args = serde_json::from_str::<Vec<String>>(&form.args)
                    .map_err(DraftError::Arguments)?;
                // Preserve non-UTF8 paths when the user has not edited the displayed program.
                if values(&spec).program != program {
                    spec.command = Some(Command::new(program, args));
                } else if let Some(command) = spec.command.as_mut() {
                    command.args = args;
                }
            }
            if self.draft_connected(cx) {
                let millis = form.grace.parse::<u64>().map_err(DraftError::Grace)?;
                spec.cancellation_grace = Duration::from_millis(millis);
            }
            if let Some(index) = self.selected {
                catalog[index] = spec;
            } else {
                catalog.push(spec);
            }
        }
        validate_agents(&catalog)?;
        Ok(catalog)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let connected = self.draft_connected(cx);
        let deleting = self.deleting;
        let index = self.selected.unwrap_or_else(|| self.catalog().len());
        let result = self.candidate(cx).and_then(|catalog| {
            self.store
                .save_agents(
                    self.document.as_mut().ok_or(DraftError::NotLoaded)?,
                    catalog,
                )
                .map_err(DraftError::Store)
        });
        match result {
            Ok(()) => {
                self.selected = None;
                self.draft = None;
                self.deleting = false;
                self.message = "已保存本机配置。".into();
                let catalog = self.catalog();
                self.checks
                    .retain(|id, _| catalog.iter().any(|spec| spec.id == *id));
                if !deleting && let Some(spec) = catalog.get(index).cloned() {
                    if connected {
                        // Changing an advanced value does not change the checked transport.
                        if let Some(check) = self.checks.get_mut(&spec.id) {
                            check.spec = spec;
                        }
                    } else {
                        self.start_check(spec, true, cx);
                    }
                }
            }
            Err(error) => {
                self.message = format!(
                    "未保存，草稿已保留：{error}。若文件已被外部修改，请取消编辑后重新加载。"
                );
            }
        }
        cx.notify();
    }
}

/// One-line description of how a backend is driven.
fn summary(spec: &AgentSpec) -> String {
    match (spec.kind, spec.command()) {
        (Kind::Embedded, _) => "embedded · 内置 echo 后端，无需外部程序".to_owned(),
        (Kind::Acp, Some(command)) => format!("acp · {command}"),
        (Kind::Acp, None) => "acp · 缺少启动命令".to_owned(),
    }
}

impl Agents {
    /// A section card: title, one-line description, then the section body.
    fn card(
        &self,
        id: String,
        title: String,
        description: String,
        content: gpui_kit::AnyElement,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        div()
            .id(id)
            .test_support()
            .child(
                GroupBox::new()
                    .outline()
                    .title(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .text_color(cx.theme().foreground)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description),
                            ),
                    )
                    .child(content),
            )
            .into_any_element()
    }

    fn summary_content(
        &self,
        index: usize,
        spec: &AgentSpec,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let edit = spec.clone();
        let probe = spec.clone();
        let status = self.status(spec);
        let connected = status.is_some_and(check::Status::connected);
        let busy = status.is_some_and(check::Status::busy);
        let detail = match status {
            Some(check::Status::Installed(path)) => {
                format!("已找到 {path}。连接成功后可进一步配置。")
            }
            Some(check::Status::Connected(path)) if !path.is_empty() => {
                format!("ACP 握手成功 · {path}")
            }
            Some(check::Status::AuthenticationRequired { .. }) => {
                "程序和 ACP 连接正常；请使用下方 Agent 提供的认证方式。".into()
            }
            Some(check::Status::Missing(message)) => {
                format!("{message}。请安装 ACP 工具或填写可执行文件的绝对路径。")
            }
            Some(check::Status::Failed(message)) => message.clone(),
            _ if spec.kind == Kind::Embedded => "内置后端，无需外部工具。".into(),
            _ => "检测使用 basrd 的 PATH；连接只验证握手，不发送聊天消息。".into(),
        };
        let summary = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .child(
                        div()
                            .id(format!("agents-status-{index}"))
                            .test_support()
                            .child(status.map_or("未检测", check::Status::label)),
                    )
                    .child(div().text_color(cx.theme().muted_foreground).child(detail)),
            )
            .when(spec.kind == Kind::Acp, |row| {
                row.child(
                    button(format!("agents-connect-{index}"))
                        .label(if busy {
                            "检测中…"
                        } else if connected {
                            "重新连接"
                        } else {
                            "连接"
                        })
                        .disabled(busy || self.dirty(cx))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_check(probe.clone(), true, cx)
                        })),
                )
            })
            .child(
                button(gpui_kit::SharedString::from(format!("agents-edit-{index}")))
                    .label(if connected { "配置" } else { "配置路径" })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit(edit.clone(), Some(index), window, cx)
                    })),
            )
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(summary)
            .child(self.authentication_content(spec, index, cx))
            .into_any_element()
    }

    fn form_content(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let embedded = self
            .draft
            .as_ref()
            .is_some_and(|spec| spec.kind == Kind::Embedded);
        let connected = self.draft_connected(cx);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .when(!embedded && !connected, |form| form.child(
                div().id("agents-connection-help").test_support().text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("先安装并登录支持 ACP 的程序，填写可执行文件的绝对路径和启动参数，再保存并连接。普通 CLI 不一定支持 ACP；参见安装指引。"),
            ))
            .children(self.form.rows(embedded, connected).into_iter().map(
                |(id, label, input)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().child(label))
                        .child(
                            div()
                                .id(id)
                                .test_support()
                                .child(Input::new(input).disabled(self.deleting)),
                        )
                },
            ))
            .when(self.deleting, |form| {
                form.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "待移除配置：确认后仅从 Agent 目录移除，不删除或停止任何会话；不能移除最后一个 Agent。",
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        button("agents-save")
                            .label(if self.deleting {
                                "确认移除配置"
                            } else if connected {
                                "保存配置"
                            } else {
                                "保存并连接"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    )
                    .child(
                        button("agents-cancel")
                            .label("取消编辑")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.draft = None;
                                this.selected = None;
                                this.deleting = false;
                                this.message =
                                    "已丢弃草稿，磁盘未改变；可重新加载外部修改。".into();
                                cx.notify();
                            })),
                    )
                    .child(
                        button("agents-delete")
                            .label("标记删除")
                            .disabled(self.selected.is_none() || self.deleting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.deleting = true;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn add_content(&self, drafting: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div().flex().child(
                    button("agents-add-custom")
                        .label("添加自定义 ACP")
                        .disabled(self.document.is_none())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.edit(AgentSpec::acp("", Command::new("", [])), None, window, cx)
                        })),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("填写支持 ACP 的程序路径与参数。连接检测由本机 basrd 执行；不会自动安装工具。")
                    .child("认证沿用工具自己的登录状态和环境变量。握手成功不代表模型请求一定可用。"),
            );
        if drafting {
            content = content.child(self.form_content(cx));
        }
        content = content.child(
            div().flex().child(
                button("agents-installation-toggle")
                    .label(if self.show_installation {
                        "收起安装指引"
                    } else {
                        "安装与登录指引"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_installation = !this.show_installation;
                        cx.notify();
                    })),
            ),
        );
        if self.show_installation {
            content = content.child(installation::render(cx));
        }
        content.into_any_element()
    }
}

impl Render for Agents {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.dirty(cx);
        let catalog = self.catalog();
        let drafting = self.draft.is_some() && self.selected.is_none();
        let section_count = catalog.len() + 1;
        if let Some(index) = self.scroll_to.take() {
            self.scroll
                .scroll_to_top_of_item(index.min(section_count.saturating_sub(1)));
        }
        let cards: Vec<gpui_kit::AnyElement> = catalog
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                let content = if self.selected == Some(index) {
                    self.form_content(cx)
                } else {
                    self.summary_content(index, spec, cx)
                };
                self.card(
                    format!("agents-card-{index}"),
                    spec.id.to_string(),
                    summary(spec),
                    content,
                    cx,
                )
            })
            .collect();
        let add = self.card(
            "agents-card-add".to_owned(),
            "添加 Agent".to_owned(),
            "接入自己的 ACP 工具。".to_owned(),
            self.add_content(drafting, cx),
            cx,
        );
        div()
            .id("agents-page")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .p_5()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(div().text_xl().child("Agent 服务"))
                            .child(
                                button("agents-installation-help")
                                    .label("安装指引")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_installation(cx)),
                                    ),
                            )
                            .child(
                                button("agents-reload")
                                    .label("重新检测")
                                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child("本机 ACP 工具 · 不可用的程序不会显示在聊天选择器中。")
                            .child("先连接，再配置。修改只影响新会话；不重启已有会话。")
                            .child(format!("配置文件：{}", self.store.path().display())),
                    )
                    .child(
                        div()
                            .id("agents-dirty")
                            .test_support()
                            .text_sm()
                            .when(dirty, |line| line.child("尚有未保存的修改")),
                    )
                    .child(
                        div()
                            .id("agents-message")
                            .test_support()
                            .text_sm()
                            .child(self.message.clone()),
                    ),
            )
            .child(
                div()
                    .id("agents-sections")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .children(cards)
                    .child(add),
            )
    }
}
