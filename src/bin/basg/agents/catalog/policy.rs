use super::*;

impl Service {
    pub(crate) fn acp_configuration(
        &self,
        save: Option<pb::SaveAcpConfigurationRequest>,
    ) -> impl Future<Output = Result<pb::AcpConfiguration, String>> + use<> {
        let server = self.server.clone();
        let (send, receive) = oneshot::channel();
        let task = AbortOnDropHandle::new(self.runtime.spawn(async move {
            let result = tokio::time::timeout(CATALOG_TIMEOUT, async {
                let mut client = Client::connect_options(&server)
                    .await
                    .map_err(|e| e.to_string())?;
                let response = match save {
                    Some(request) => client.save_acp_configuration(request).await,
                    None => {
                        client
                            .get_acp_configuration(pb::GetAcpConfigurationRequest {})
                            .await
                    }
                };
                response
                    .map(|response| response.into_inner())
                    .map_err(|error| {
                        if error.code() == tonic::Code::Unimplemented {
                            crate::i18n::tr!("acp-policy-upgrade").into()
                        } else {
                            error.message().to_owned()
                        }
                    })
            })
            .await
            .unwrap_or_else(|_| Err(crate::i18n::tr!("agent-config-timeout").into()));
            let _ = send.send(result);
        }));
        async move {
            let _task = task;
            receive
                .await
                .unwrap_or_else(|_| Err(crate::i18n::tr!("agent-config-ended").into()))
        }
    }
}
