//! Local catalog editor. Nothing in this page launches an agent or contacts a daemon.

use super::action_button as button;
use basr::config::{
    Document, Store,
    agent::{AgentSpec, Command, Kind, ValidationError},
    validate_agents,
};
use gpui_kit::{
    App, Context, Entity, Render, Subscription, TestSupportExt, Window,
    component::{input::*, *},
    div,
    prelude::*,
};
use std::time::Duration;

#[cfg(all(test, feature = "gui-tests"))]
mod tests;

#[derive(Clone, Copy)]
enum Preset {
    Codex,
    OpenCode,
    Pi,
    Custom,
}

impl Preset {
    const ALL: [Self; 4] = [Self::Codex, Self::OpenCode, Self::Pi, Self::Custom];

    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
            Self::Custom => "自定义 ACP",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Codex => "agents-preset-codex",
            Self::OpenCode => "agents-preset-opencode",
            Self::Pi => "agents-preset-pi",
            Self::Custom => "agents-preset-custom",
        }
    }

    fn spec(self) -> AgentSpec {
        let (id, program, args) = match self {
            Self::Codex => ("codex", "codex-acp", vec![]),
            Self::OpenCode => ("opencode", "opencode", vec!["acp".to_owned()]),
            Self::Pi => ("pi", "pi-acp", vec![]),
            Self::Custom => ("", "", vec![]),
        };
        AgentSpec::acp(id, Command::new(program, args))
    }
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
            program: cx.new(|cx| InputState::new(window, cx)),
            args: cx.new(|cx| InputState::new(window, cx)),
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

    fn rows(&self, embedded: bool) -> Vec<(&'static str, &'static str, &Entity<InputState>)> {
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
        rows.push((
            "agents-grace",
            "取消等待时间 cancellation_grace_ms",
            &self.grace,
        ));
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
    #[error("请先选择 Agent 或模板")]
    NotSelected,
    #[error("未保存的模板无需删除；请取消编辑")]
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
    _subscriptions: Vec<Subscription>,
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
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        page
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
                    self.message = "已读取本机配置；未验证 daemon 是否已应用。".into();
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
            self.message = "请先保存或取消当前草稿，再选择其他 Agent / 模板。".into();
        } else {
            let form = values(&spec);
            for (input, value) in self.form.fields().into_iter().zip(form.fields()) {
                input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            }
            self.draft = Some(spec);
            self.selected = selected;
            self.deleting = false;
            self.message.clear();
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
            let millis = form.grace.parse::<u64>().map_err(DraftError::Grace)?;
            spec.cancellation_grace = Duration::from_millis(millis);
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
                self.message = "已保存到本机配置文件；未验证 daemon 是否已应用。".into();
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

impl Render for Agents {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.dirty(cx);
        let catalog = self
            .document
            .as_ref()
            .map(|document| document.settings.daemon.agents.clone())
            .unwrap_or_default();
        let embedded = self
            .draft
            .as_ref()
            .is_some_and(|spec| spec.kind == Kind::Embedded);
        div().id("agents-page").test_support().size_full().overflow_y_scroll().p_4()
            .flex().flex_col().gap_3()
            .child(div().text_lg().child("ACP agents · 本机 Agent 服务"))
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .flex().flex_col().gap_1()
                .child("仅编辑此电脑的 basrd 配置，不修改远端服务器。")
                .child("本机 basrd 自动加载，新会话生效；现有会话不受影响。显式 --agents / --agents-file 覆盖时不跟随此文件。")
                .child(format!("配置文件：{}", self.store.path().display())))
            .child(button("agents-reload").label("重新加载").on_click(cx.listener(|this, _, _, cx| this.reload(cx))))
            .child(div().flex().flex_wrap().gap_2().children(Preset::ALL.map(|preset| {
                button(preset.id()).label(format!("添加 {}", preset.label()))
                    .disabled(self.document.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| this.edit(preset.spec(), None, window, cx)))
            })))
            .child(div().flex().flex_wrap().gap_2().children(catalog.into_iter().enumerate().map(|(index, spec)| {
                button(gpui_kit::SharedString::from(format!("agents-select-{index}"))).label(format!("{} · {}", spec.id, spec.kind))
                    .on_click(cx.listener(move |this, _, window, cx| this.edit(spec.clone(), Some(index), window, cx)))
            })))
            .when(self.draft.is_some(), |view| {
                view.child(if embedded { "embedded 旧条目（非 ACP）：保留类型，不编辑外部命令。" } else { "ACP 命令草稿" })
                    .children(self.form.rows(embedded).into_iter().map(|(id, label, input)| {
                        div().flex().flex_col().gap_1().child(label)
                            .child(div().id(id).test_support().child(Input::new(input).disabled(self.deleting)))
                    }))
                    .child(div().flex().flex_wrap().gap_2()
                        .child(button("agents-save").label(if self.deleting { "确认移除配置" } else { "保存 Agent" }).on_click(cx.listener(|this, _, _, cx| this.save(cx))))
                        .child(button("agents-cancel").label("取消编辑").on_click(cx.listener(|this, _, _, cx| {
                            this.draft = None;
                            this.selected = None;
                            this.deleting = false;
                            this.message = "已丢弃草稿，磁盘未改变；可重新加载外部修改。".into();
                            cx.notify();
                        })))
                        .child(button("agents-delete").label("标记删除")
                            .disabled(self.selected.is_none() || self.deleting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.deleting = true;
                                cx.notify();
                            }))))
            })
            .when(self.deleting, |view| view.child("待移除配置：确认后仅从 Agent 目录移除，不删除或停止任何会话；不能移除最后一个 Agent。"))
            .child(div().id("agents-dirty").test_support().child(if dirty {
                "尚未确认：请保存或取消当前操作；关闭窗口不会应用草稿。"
            } else { "无未保存更改" }))
            .child(div().id("agents-message").test_support().child(self.message.clone()))
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .flex().flex_col().gap_1()
                .child("模板只准备草稿，不下载、不安装、不启动。请自行安装依赖，或填写服务可访问的绝对路径；此页面不检测可执行文件是否就绪。")
                .child("Codex 使用 codex-acp；OpenCode 使用 opencode acp；Pi 需要 Pi 本体与 pi-acp adapter。")
                .child("命令名称由 basrd 进程的 PATH 解析，可能与当前终端不同；也可使用绝对路径。")
                .child("认证交给各 Agent 原有登录 / 环境；既有 env 原样保留，不在此收集 API key。"))
    }
}
