use super::super::*;

impl Workspace {
    fn on_workspace_auth_open(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_auth(window, cx);
    }

    fn on_workspace_auth_changed(
        &mut self,
        _: Entity<crate::authentication::Authentication>,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }

    pub(in crate::workspace) fn authentication_button(&self, cx: &mut Context<Self>) -> AnyElement {
        crate::ui::tooltip::bounded(
            "authentication-hint",
            gds_uim::el! {
                Button("agent-authenticate") {
                    xsmall;
                    label: $(if self.backend.server.ip().is_loopback() {
                        crate::i18n::tr!("auth-action")
                    } else {
                        crate::i18n::tr!("auth-instructions")
                    });
                    on_click: $cx.listener(Self::on_workspace_auth_open);
                }
            },
            if self.backend.server.ip().is_loopback() {
                crate::i18n::tr!("auth-action")
            } else {
                crate::i18n::tr!("auth-view-instructions")
            },
        )
        .into_any_element()
    }

    pub(in crate::workspace) fn authentication_hint(&self) -> Option<String> {
        self.selected.authentication.required.as_ref().map(|agent| {
            if self.backend.server.ip().is_loopback() {
                crate::i18n::tr!("auth-local-help", agent = (agent).to_string())
            } else {
                crate::i18n::tr!(
                    "auth-remote-help",
                    agent = (agent).to_string(),
                    server = (self.backend.server).to_string()
                )
            }
        })
    }

    pub(in crate::workspace) fn open_agent_auth(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.selected.authentication.required.clone() else {
            return;
        };
        self.close_reply_preview(window, cx);
        self.close_menu(window, cx);
        self.record_settings_target(cx);
        let result = if self.backend.server.ip().is_loopback() {
            if cx.try_global::<crate::agents::check::Service>().is_none() {
                crate::settings::window::open_agent(&agent, cx).map(|_| ())
            } else {
                self.load_authentication_panel(window, cx);
                Ok(())
            }
        } else {
            crate::application::open_message(
                crate::i18n::tr!("auth-remote-title"),
                &crate::i18n::tr!(
                    "auth-remote-help",
                    agent = agent.as_str(),
                    server = self.backend.server.to_string()
                ),
                cx,
            )
            .map(|_| ())
        };
        if let Err(error) = result {
            self.selected.error = Some(crate::i18n::tr!(
                "auth-instructions-error",
                error = (error).to_string()
            ));
        }
        cx.notify();
    }

    pub(in crate::workspace) fn load_authentication_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.selected.authentication.required.clone() else {
            return;
        };
        if !self.backend.server.ip().is_loopback()
            || self.selected.authentication.panel.is_some()
            || self.selected.authentication.load.is_some()
        {
            return;
        }
        let Some(catalog) = cx.try_global::<crate::agents::catalog::Service>().cloned() else {
            return;
        };
        let Some(service) = cx.try_global::<crate::agents::check::Service>().cloned() else {
            return;
        };
        let path = crate::configuration::store(cx).path().to_owned();
        let epoch = self.selected.epoch;
        self.window.output_open = true;
        self.selected.authentication.load = Some(cx.spawn_in(window, async move |view, cx| {
            let result = async {
                let catalog = catalog.catalog(false).await?;
                let spec = catalog
                    .entries
                    .into_iter()
                    .find(|entry| entry.id == agent)
                    .and_then(|entry| entry.spec)
                    .ok_or_else(|| crate::i18n::tr!("agent-catalog-missing").to_owned())?;
                let status = service.request(spec.clone(), true, path.clone()).await;
                Ok::<_, String>((spec, status))
            }
            .await;
            let _ = view.update_in(cx, |this, window, cx| {
                if this.selected.epoch != epoch
                    || this.selected.id.is_some()
                    || this.selected.authentication.required.as_ref() != Some(&agent)
                {
                    return;
                }
                this.selected.authentication.load = None;
                match result {
                    Err(error) => this.selected.error = Some(error),
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
                        let completed = cx.subscribe_in(
                            &panel,
                            window,
                            move |this, _, event: &crate::authentication::Completed, _, cx| {
                                if event.0.connected()
                                    && this.selected.epoch == epoch
                                    && this.selected.id.is_none()
                                    && this.selected.authentication.required.as_ref()
                                        == Some(&expected)
                                {
                                    this.selected.authentication.panel = None;
                                    this.refresh(cx);
                                }
                                cx.notify();
                            },
                        );
                        let changed = cx.observe(&panel, Self::on_workspace_auth_changed);
                        this.selected.authentication.panel =
                            Some(state::ObservedPanel::new(panel, vec![completed, changed]));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
