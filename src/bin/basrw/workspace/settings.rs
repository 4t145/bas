use super::*;

pub(super) fn config_option_hint(option: &basr::acp::SessionConfigOption) -> String {
    let mut hint = option.name.clone();
    if let basr::acp::SessionConfigKind::Boolean(boolean) = &option.kind {
        hint.push_str(if boolean.current_value {
            " · 开启"
        } else {
            " · 关闭"
        });
    }
    if let Some(description) = option
        .description
        .as_deref()
        .filter(|description| !description.is_empty())
    {
        hint.push_str(" · ");
        hint.push_str(description);
    }
    hint
}

impl Workspace {
    pub(super) fn authentication_button(&self, cx: &mut Context<Self>) -> AnyElement {
        crate::tooltip::bounded(
            "authentication-hint",
            Button::new("agent-authenticate")
                .xsmall()
                .label(if self.server.ip().is_loopback() {
                    "认证"
                } else {
                    "认证说明"
                })
                .on_click(cx.listener(|this, _, window, cx| this.open_agent_auth(window, cx))),
            self.authentication_hint().unwrap_or_default(),
        )
        .into_any_element()
    }

    pub(super) fn authentication_hint(&self) -> Option<String> {
        self.authentication_required.as_ref().map(|agent| {
            if self.server.ip().is_loopback() {
                format!("{agent} 需要认证。按照会话中的认证步骤完成后会自动重试；草稿已保留。")
            } else {
                format!("{agent} 需要认证。请在远端 daemon（{}）所在主机完成登录后按 F5 重试；不会在本机运行远端登录命令。", self.server)
            }
        })
    }

    pub(super) fn open_agent_auth(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(_agent) = self.authentication_required.clone() else {
            return;
        };
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        self.record_settings_target(cx);
        let result = if self.server.ip().is_loopback() {
            if cx
                .try_global::<crate::settings::agents::check::Service>()
                .is_none()
            {
                crate::settings_window::open_agent(&_agent, cx).map(|_| ())
            } else {
                self.load_authentication_panel(window, cx);
                Ok(())
            }
        } else {
            crate::application::open_message(
                "远端 Agent 需要认证",
                &self.authentication_hint().unwrap_or_default(),
                cx,
            )
            .map(|_| ())
        };
        if let Err(error) = result {
            self.error = Some(format!("无法打开认证说明：{error}"));
        }
        cx.notify();
    }

    pub(super) fn load_authentication_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.authentication_required.clone() else {
            return;
        };
        if !self.server.ip().is_loopback()
            || self.authentication_panel.is_some()
            || self.authentication_load.is_some()
        {
            return;
        }
        let Some(catalog) = cx
            .try_global::<crate::settings::agents::AgentCatalogService>()
            .cloned()
        else {
            return;
        };
        let Some(service) = cx
            .try_global::<crate::settings::agents::check::Service>()
            .cloned()
        else {
            return;
        };
        let path = crate::theme::Editor::current(cx).path().to_owned();
        let epoch = self.epoch;
        self.output_open = true;
        self.authentication_load = Some(cx.spawn_in(window, async move |view, cx| {
            let result = async {
                let catalog = catalog.catalog(false).await?;
                let spec = catalog
                    .entries
                    .into_iter()
                    .find(|entry| entry.id == agent)
                    .and_then(|entry| entry.spec)
                    .ok_or_else(|| {
                        "daemon 的目录中没有这个 Agent 的配置，请重新检测。".to_owned()
                    })?;
                let status = service.request(spec.clone(), true, path.clone()).await;
                Ok::<_, String>((spec, status))
            }
            .await;
            let _ = view.update_in(cx, |this, window, cx| {
                if this.epoch != epoch
                    || this.active.is_some()
                    || this.authentication_required.as_ref() != Some(&agent)
                {
                    return;
                }
                this.authentication_load = None;
                match result {
                    Err(error) => this.error = Some(error),
                    Ok((_, status)) if status.connected() => {
                        this.refresh(cx);
                    }
                    Ok((spec, status)) => {
                        let panel = cx.new(|_| {
                            crate::authentication::Authentication::new(
                                spec,
                                status,
                                path,
                                window.window_handle(),
                            )
                        });
                        let expected = agent.clone();
                        this._subscriptions.push(cx.subscribe_in(
                            &panel,
                            window,
                            move |this, _, event: &crate::authentication::Completed, _, cx| {
                                if event.0.connected()
                                    && this.epoch == epoch
                                    && this.active.is_none()
                                    && this.authentication_required.as_ref() == Some(&expected)
                                {
                                    this.authentication_panel = None;
                                    this.refresh(cx);
                                }
                                cx.notify();
                            },
                        ));
                        this._subscriptions
                            .push(cx.observe(&panel, |_, _, cx| cx.notify()));
                        this.authentication_panel = Some(panel);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn sync_elicitations(
        &mut self,
        pending: Vec<basr::service::pb::PendingElicitation>,
        completed: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.active else {
            return;
        };
        let epoch = self.epoch;
        let ids: std::collections::HashSet<_> = pending
            .iter()
            .map(|pending| pending.request_id.as_str())
            .collect();
        self.elicitation_panels
            .retain(|id, _| ids.contains(id.as_str()) && !completed.contains(id));
        for pending in pending {
            if completed.contains(&pending.request_id) {
                continue;
            }
            if let Some(panel) = self.elicitation_panels.get(&pending.request_id) {
                if pending.answered {
                    panel.update(cx, |panel, cx| {
                        panel.submitting = true;
                        cx.notify();
                    });
                }
                continue;
            }
            let request = pending
                .request
                .as_ref()
                .ok_or_else(|| "Agent 未提供交互请求内容。".to_owned())
                .and_then(|payload| {
                    payload
                        .decode::<serde_json::Value>()
                        .map_err(|error| error.to_string())
                })
                .and_then(|raw| {
                    basr::authentication::elicitation::parse_request(&raw)
                        .map_err(|error| error.to_string())
                });
            let request = match request {
                Ok(request) => request,
                Err(error) => {
                    self.error = Some(error);
                    continue;
                }
            };
            let id = pending.request_id;
            let panel = cx.new(|cx| {
                let mut panel =
                    crate::authentication::elicitation::Panel::new(id.clone(), request, window, cx);
                panel.submitting = pending.answered;
                panel
            });
            self._subscriptions.push(cx.subscribe(
                &panel,
                move |this, _, answer: &crate::authentication::elicitation::Answered, cx| {
                    if this.active == Some(session)
                        && this.epoch == epoch
                        && this.elicitation_panels.contains_key(&answer.request_id)
                    {
                        this.command(Command::RespondElicitation {
                            session,
                            request_id: answer.request_id.clone(),
                            response: answer.response.clone(),
                        });
                    }
                    cx.notify();
                },
            ));
            self._subscriptions
                .push(cx.observe(&panel, |_, _, cx| cx.notify()));
            self.elicitation_panels.insert(id, panel);
            self.output_open = true;
        }
        cx.notify();
    }

    pub(super) fn open_agent_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        self.record_settings_target(cx);
        if let Err(error) = crate::settings_window::open_catalog(cx) {
            self.error = Some(format!("无法打开 Agent 安装目录：{error}"));
        }
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        self.record_settings_target(cx);
        if let Err(error) = crate::settings_window::open(cx) {
            self.error = Some(format!("无法打开设置窗口：{error}"));
        }
        cx.notify();
    }

    pub(super) fn record_settings_target(&self, cx: &mut Context<Self>) {
        cx.update_global::<crate::settings::AgentChanges, _>(|changes, _| {
            changes.target = Some(self.window.window_id());
            changes.select = None;
            changes.ready = None;
            changes.server = None;
        });
    }
}

#[cfg(all(test, feature = "gui-tests"))]
mod tests;
