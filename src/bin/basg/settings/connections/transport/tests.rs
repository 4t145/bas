use super::Transport;

#[test]
fn tcp_editor_uses_shared_hostname_validation() {
    for input in ["workstation", "tcp://localhost", "[::1]"] {
        let target = Transport::Tcp.target(input).expect("TCP target");
        assert_eq!(target, input.parse().expect("shared syntax"));
        assert_eq!(Transport::from_target(&target), Transport::Tcp);
        assert_eq!(
            Transport::Tcp.target(&Transport::address(&target)),
            Ok(target)
        );
    }
    for input in ["host:65536", "user@host", " host"] {
        assert!(Transport::Tcp.target(input).is_err(), "{input}");
    }
}

#[test]
fn connection_type_choices_match_the_current_platform() {
    let choices = Transport::AVAILABLE;
    assert!(choices.contains(&Transport::Local));
    assert!(choices.contains(&Transport::Tcp));
    assert!(choices.contains(&Transport::Mtls));
    assert_eq!(choices.contains(&Transport::Unix), cfg!(unix));
    assert_eq!(choices.contains(&Transport::Pipe), cfg!(windows));
    assert_eq!(
        choices.len(),
        3 + usize::from(cfg!(unix)) + usize::from(cfg!(windows)),
    );
}
