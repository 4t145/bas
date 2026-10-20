use super::*;

fn remote<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .extensions_mut()
        .insert(tonic::transport::server::TcpConnectInfo {
            local_addr: Some(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                bas::service::DEFAULT_PORT,
            ))),
            remote_addr: Some("192.0.2.1:12345".parse().expect("remote address")),
        });
    request
}

#[tokio::test]
async fn management_capabilities_do_not_depend_on_peer_transport() {
    let registry = registry(100);
    let service = Service::new(registry.clone());
    for request in [
        Request::new(pb::InstallAgentRequest {
            id: "fixture".into(),
        }),
        remote(pb::InstallAgentRequest {
            id: "fixture".into(),
        }),
    ] {
        let error = service
            .install_agent(request)
            .await
            .err()
            .expect("install denied before acquiring manager");
        assert_eq!(error.code(), Code::Unimplemented);
    }
    assert_eq!(
        service
            .remove_agent(remote(pb::RemoveAgentRequest {
                id: "fixture".into()
            }))
            .await
            .expect_err("remove denied")
            .code(),
        Code::Unimplemented
    );
    assert_eq!(
        service
            .authenticate_agent(remote(pb::AuthenticateAgentRequest::default()))
            .await
            .expect_err("auth denied")
            .code(),
        Code::InvalidArgument
    );
    registry.shutdown().await;
}

#[test]
fn reloading_identical_specs_does_not_invalidate_a_checked_generation() {
    let registry = registry(100);
    let initial = registry.catalog();
    registry.replace_catalog(Arc::new(Catalog::with_defaults()));
    assert!(Arc::ptr_eq(&initial, &registry.catalog()));
}
