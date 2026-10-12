use qa_core::loopback::Endpoint;
use qa_network::{channel::Channel, commands::packet::Protocol};

fn client() -> Channel {
    Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 16384, 8).unwrap()
}

#[test]
fn selected_commands_have_native_retention_and_execution_separate_from_receipt() {
    let mut channel = client();
    assert_eq!(channel.server_command(0).unwrap(), b"");
    assert!(channel.server_command(1).is_err());
    for sequence in 1..=64 {
        assert!(
            channel
                .receive_command(sequence, b"print retained")
                .unwrap()
        );
    }
    assert_eq!(channel.last_executed_server_command().unwrap(), 0);
    assert!(channel.server_command(0).is_err());
    assert_eq!(channel.server_command(1).unwrap(), b"print retained");
    channel.executed_server_command(64).unwrap();
    assert_eq!(channel.last_executed_server_command().unwrap(), 64);
    channel.executed_server_command(1).unwrap();
    assert_eq!(channel.last_executed_server_command().unwrap(), 1);
    assert!(channel.receive_command(65, b"cp latest").unwrap());
    assert!(channel.server_command(1).is_err());
    assert_eq!(channel.server_command(2).unwrap(), b"print retained");
    assert!(channel.executed_server_command(1).is_err());
    assert_eq!(channel.last_executed_server_command().unwrap(), 1);
    assert!(
        !channel
            .receive_command(65, b"duplicate replacement")
            .unwrap()
    );
    assert_eq!(channel.server_command(65).unwrap(), b"cp latest");
    assert!(channel.server_command(66).is_err());
}

#[test]
fn skipped_or_modulo_colliding_sequences_do_not_select_stale_text() {
    let mut channel = client();
    assert!(channel.receive_command(2, b"two").unwrap());
    assert!(channel.server_command(1).is_err());
    assert_eq!(channel.server_command(2).unwrap(), b"two");
    assert!(channel.receive_command(66, b"sixty-six").unwrap());
    assert!(channel.server_command(2).is_err());
    assert!(channel.server_command(3).is_err());
    assert_eq!(channel.server_command(66).unwrap(), b"sixty-six");
}

#[test]
fn client_command_selection_does_not_exist_on_other_channel_roles() {
    for (protocol, endpoint) in [
        (Protocol::Quake3_68, Endpoint::Server),
        (Protocol::Quake2_34, Endpoint::Client),
    ] {
        let mut channel = Channel::load(protocol.channel(), endpoint, 8192, 8).unwrap();
        assert!(channel.server_command(0).is_err());
        assert!(channel.last_executed_server_command().is_err());
        assert!(channel.executed_server_command(0).is_err());
    }
}
