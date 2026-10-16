#![allow(dead_code)]
use qa_app::host::{FrameHost, FrameSource};
use qa_core::{loopback::Endpoint, primitives::ClientId, sys_events::*};
use qa_network::commands::packet::Protocol;
use qa_render::{
    Assets,
    cpu::{CpuBackend, CpuLimits},
};
use std::time::Duration;

pub struct Source {
    pub cpu: CpuBackend,
    pub enabled: bool,
    pub attempts: usize,
    time: u64,
}
impl Source {
    pub fn load(assets: &Assets) -> Self {
        Self {
            cpu: CpuBackend::load_with_limits(
                64,
                48,
                assets,
                CpuLimits {
                    cache_bytes: 4096,
                    ..Default::default()
                },
            )
            .unwrap(),
            enabled: true,
            attempts: 0,
            time: 100,
        }
    }
    pub fn step(&mut self, host: &mut FrameHost) {
        self.time += 100;
        host.frame(self, true);
    }
}
impl qa_app::renderer::AssetPreparer for Source {
    fn prepare_assets(
        &mut self,
        assets: &Assets,
        scene: Option<qa_render::Limits>,
    ) -> Result<(), String> {
        self.attempts += 1;
        self.cpu
            .prepare_assets(assets, scene)
            .map_err(str::to_owned)
    }
}
impl FrameSource for Source {
    fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
        if self.enabled { Some(self) } else { None }
    }
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(self.time)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(self.time),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped fixture")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

pub fn config(protocol: Protocol, index: usize, text: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 16384];
    let n = qa_network::outputs::configstring(protocol, index as u32, text, &mut bytes).unwrap();
    bytes.truncate(n);
    bytes
}

pub fn queue(host: &mut FrameHost, payloads: impl IntoIterator<Item = Vec<u8>>) {
    let client = ClientId(0);
    let connection = host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap();
    for bytes in payloads {
        connection.channel.queue_reliable(&bytes).unwrap();
    }
    let time = EventTime::from_milliseconds(100);
    let mut sent = 0;
    loop {
        let Some(packet) = connection.channel.prepare_output(time).unwrap() else {
            break;
        };
        host.runtime
            .loopback
            .send(Endpoint::Server, client, packet.bytes)
            .unwrap();
        connection.channel.submitted(time).unwrap();
        sent += 1;
        if !connection.channel.pending_fragments() {
            break;
        }
    }
    assert!(
        sent > 0,
        "native reliable payload must actually enter the loopback"
    );
}

pub fn static_scene(
    host: &FrameHost,
    client: ClientId,
    protocol: Protocol,
) -> (
    qa_app::client_scene::SceneCounts,
    Vec<qa_render::SceneEntity>,
) {
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let resources = &host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state;
    let player = qa_core::primitives::PlayerState::default();
    let projection = qa_network::projection::EntityProjection::load(protocol);
    let player_projection = qa_network::projection::PlayerProjection::load(protocol, &[]);
    let input = qa_app::client_scene::ClientSceneInput {
        client,
        current: None,
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(protocol.rule_set()),
        player_projection: &player_projection,
        bindings: &resources.models,
        models: owner.catalog(),
        static_entities: resources.static_entities(),
        player: &player,
        weapon_view_angles: Default::default(),
        alias_roll: Default::default(),
        interpolation: Default::default(),
        gun_animation: None,
        gun_previous: None,
        view_entity: None,
    };
    let mut front = qa_render::FrontEnd::load(Default::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let counts = input.append(
        &mut frame,
        owner.assets(),
        &qa_render::Refdef::default(),
        true,
        None,
    );
    let packet = frame.finish();
    let entities = (0..counts.models)
        .map(|index| *packet.entity(index))
        .collect();
    (counts, entities)
}
