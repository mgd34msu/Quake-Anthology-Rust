use qa_core::primitives::ClientId;
use qa_session::clients::{Server, UserinfoStamp};

pub fn publish(server: &mut Server, client: ClientId) -> UserinfoStamp {
    let record = server.userinfo_record(client).unwrap();
    assert!(record.dirty());
    let stamp = record.stamp;
    let bytes = record.text.to_vec();
    server.events.peer_info(stamp.user_id, &bytes, &[]).unwrap();
    assert!(server.mark_userinfo_queued(client, stamp));
    stamp
}
