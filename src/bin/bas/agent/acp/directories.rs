use super::*;

impl Session {
    /// # Errors
    /// Rejects unsupported peers, native reload failures and lost commit acknowledgements.
    pub(super) fn reload_directories(
        &self,
        directories: Vec<PathBuf>,
    ) -> BoxFuture<'static, Result<(), Error>> {
        let capabilities = &self.initialized.agent_capabilities;
        let supported = capabilities
            .session_capabilities
            .additional_directories
            .is_some();
        let load = capabilities.load_session;
        let resume = capabilities.session_capabilities.resume.is_some();
        let agent = self.agent.clone();
        let id = self.acp_session.clone();
        let cwd = self.cwd.clone();
        let connection = self.connection.clone();
        let state = Arc::clone(&self.state);
        let updates = self.updates.clone();
        let activity = self.activity.clone();
        Box::pin(async move {
            let activity = activity
                .outbound()
                .ok_or_else(|| Error::backend("acp", "transport released"))?;
            if !supported || !(load || resume) {
                return Err(Error::Unsupported {
                    agent,
                    operation: "additionalDirectories with session/load or session/resume",
                });
            }
            let request = if load {
                agent_client_protocol::UntypedMessage::new(
                    "session/load",
                    v1::LoadSessionRequest::new(id.clone(), cwd)
                        .additional_directories(directories.clone()),
                )?
            } else {
                agent_client_protocol::UntypedMessage::new(
                    "session/resume",
                    v1::ResumeSessionRequest::new(id.clone(), cwd)
                        .additional_directories(directories.clone()),
                )?
            };
            // The transcript is already stored. The ordered response callback ends
            // suppression only after the new workspace metadata is committed.
            state.lock().await.reloading = load;
            state
                .lock()
                .await
                .subagents
                .lock()
                .expect("subagents lock")
                .invalidate_activity();
            let (done, response) = oneshot::channel();
            let callback_state = Arc::clone(&state);
            let callback_agent = agent.clone();
            let sent = connection
                .send_request(request)
                .on_receiving_result(async move |result| {
                    let _activity = activity;
                    let result = async {
                        let value = result?;
                        let loaded: v1::LoadSessionResponse = serde_json::from_value(value)
                            .map_err(|error| {
                                Error::with_source(
                                    callback_agent,
                                    "invalid ACP reload response",
                                    error,
                                )
                            })?;
                        let config = v1::NewSessionResponse::new(id)
                            .modes(loaded.modes)
                            .config_options(loaded.config_options)
                            .meta(loaded.meta);
                        let (committed, receipt) = oneshot::channel();
                        let subagents = {
                            let state = callback_state.lock().await;
                            state.subagents.lock().expect("subagents lock").clone()
                        };
                        updates
                            .send(Update::Workspace {
                                directories,
                                config,
                                subagents,
                                committed,
                            })
                            .await
                            .map_err(|_| {
                                Error::backend("acp", "workspace update listener closed")
                            })?;
                        receipt.await.map_err(|_| {
                            Error::backend("acp", "workspace update was not committed")
                        })?;
                        Ok(())
                    }
                    .await;
                    callback_state.lock().await.reloading = false;
                    let _ = done.send(result);
                    Ok(())
                });
            if let Err(error) = sent {
                state.lock().await.reloading = false;
                return Err(error.into());
            }
            response
                .await
                .map_err(|_| Error::backend(agent, "workspace reload response lost"))?
        })
    }
}
