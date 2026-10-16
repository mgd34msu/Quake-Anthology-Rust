use qa_app::host::FrameSource;
use qa_core::{
    primitives::SoundEvent,
    sys_events::{EventTime, SysEventQueue},
};
use std::{net::SocketAddr, time::Duration};

#[derive(Debug, PartialEq, Eq)]
pub struct SendAttempt {
    pub socket: u16,
    pub to: SocketAddr,
    pub bytes: Vec<u8>,
    pub admitted: bool,
}

#[derive(Default)]
pub struct Sink {
    pub time: EventTime,
    pub reject: Option<SocketAddr>,
    pub packets: Vec<SendAttempt>,
}

impl FrameSource for Sink {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, _: &mut SysEventQueue) {}
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::ZERO
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, _bank: Option<&qa_audio::Bank>, _: SoundEvent) -> bool {
        true
    }
    fn send_packet(&mut self, socket: u16, to: SocketAddr, bytes: &[u8]) -> bool {
        let admitted = self.reject != Some(to);
        self.packets.push(SendAttempt {
            socket,
            to,
            bytes: bytes.to_vec(),
            admitted,
        });
        admitted
    }
}
