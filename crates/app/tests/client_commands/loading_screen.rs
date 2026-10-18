use super::*;
use qa_app::{
    modules::{Driver, State},
    renderer::{Context as RenderContext, Renderer},
};

pub(crate) fn loading_screen(native: bool, ui: bool) {
    for (failed, prior_frame) in [(false, true), (false, false), (true, true)] {
        let mut fixture = model_support::Fixture::new(&[
            (
                "gfx/2d/bigchars.tga",
                model_support::tga(128, 128, &vec![[255; 4]; 128 * 128]),
            ),
            (
                "ui/pic.tga",
                model_support::tga(2, 2, &[[90, 20, 10, 255]; 4]),
            ),
        ]);
        let mut initial = data();
        initial[1472] = 1;
        initial[1468..1472].copy_from_slice(&u32::from(failed).to_le_bytes());
        let program = if native {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                native_scene_data(ui, &initial)
            }
            #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
            {
                qvm_scene_data(ui, &initial)
            }
        } else {
            qvm_scene_data(ui, &initial)
        };
        let (mut runtime, ids) = runtime(2);
        runtime.vfs = std::mem::take(&mut fixture.vfs);
        let mut request = request(&mut runtime, ModuleId(1), ids[0], program);
        request.entries = (0..34).collect();
        request.initialize = vec![Export::words(CallbackId(31), &[])];
        request.frame = Export {
            client: (!ui).then_some(ids[0]),
            ..Export::clocked(CallbackId(32))
        };
        let mut requests = vec![request];
        if !ui && !failed {
            let ui_program = if native {
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                {
                    native_scene_data(true, &initial)
                }
                #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
                {
                    qvm_scene_data(true, &initial)
                }
            } else {
                qvm_scene_data(true, &initial)
            };
            let mut peer = super::request(&mut runtime, ModuleId(2), ids[0], ui_program);
            peer.entries = (0..34).collect();
            peer.frame = Export::clocked(CallbackId(32));
            requests.push(peer);
        }
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            requests,
        )
        .unwrap();
        host.local_clients[..2].copy_from_slice(&[Some(ids[0]), Some(ids[1])]);
        host.time = EventTime::from_milliseconds(123);
        let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
        let mut owner = ModelAssets::new(
            &host.runtime.vfs,
            qa_render::Assets::load().unwrap(),
            [profile],
            [],
            Default::default(),
        )
        .unwrap();
        let console = qa_app::renderer::console::ConsoleAssets::load(
            &host.runtime.vfs,
            &mut owner,
            RuleSetId::Quake3,
            qa_render::material::resources::ImageSettings::native(RuleSetId::Quake3),
            Default::default(),
            320,
            200,
        )
        .unwrap();
        let effects = std::sync::Arc::clone(&host.runtime.catalog.registry.effects);
        let bindings = vec![None; effects.len() + 1];
        let effects = qa_render::effects::Effects::load(effects, 8, 2, &bindings).unwrap();
        let configuration = std::sync::Arc::new(qa_core::presentation::RendererConfiguration {
            display: qa_core::presentation::DisplayConfiguration {
                width: 320,
                height: 200,
                ..Default::default()
            },
            ..Default::default()
        });
        host.runtime.model_assets = Some(owner);
        let huds = std::array::from_fn(|_| None);
        let (assets, staged) = host.stage_engine_graphics(&console, &huds).unwrap();
        let (mut renderer, mut scene) = Renderer::load(
            RenderContext::Cpu(configuration),
            320,
            200,
            assets,
            vec![],
            Some(qa_render::cpu::RasterBands::One),
            console,
            huds,
            effects,
            0,
        )
        .unwrap();
        staged.commit();
        host.runtime.scene = Some(scene);
        host.configure_native_scenes(
            &[None, None, None, None, Some(profile)],
            &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
            320,
            200,
        )
        .unwrap();
        host.publish_model_admission(renderer.admitted_assets().unwrap())
            .unwrap();
        scene = host.runtime.scene.take().unwrap();
        let prior = qa_render::scene::Draw2d {
            rect: [161., 4., 12., 8.],
            color: [17, 29, 31, 255],
            ..Default::default()
        };
        if prior_frame {
            assert!(scene.begin_frame());
            assert!(scene.frame_mut().unwrap().draw_2d(prior));
        }
        host.runtime.scene = Some(scene);
        host.queue
            .push(SysEvent {
                time: EventTime(999),
                kind: EventKind::Time,
            })
            .unwrap();
        host.initialize_modules_with_driver(
            Phase::Client,
            &mut Driver::with_renderer(&mut renderer, None, None),
        );
        assert_eq!(host.time, EventTime::from_milliseconds(123));
        assert_eq!(host.queue.pop().unwrap().time, EventTime(999));
        let packet = host.runtime.scene.as_mut().unwrap().finish_frame();
        if prior_frame {
            let packet = packet.unwrap();
            assert!(
                matches!(packet.commands(), [Command::Clear(_), Command::Draw2d(draw)] if draw.rect == prior.rect && draw.color == prior.color)
            );
            assert!(host.runtime.scene.as_mut().unwrap().recycle(packet).is_ok());
        } else {
            assert!(packet.is_none());
        }
        assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
        if failed {
            assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
            assert!(host.module_counts(ModuleId(1)).unwrap().traps > 0);
        } else {
            assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
            assert_eq!(
                probe(&mut host, 0, 33, 1456),
                2,
                "nested UpdateScreen must not recurse"
            );
            assert_eq!(
                probe(&mut host, 0, 33, 1460),
                3,
                "initializer resumes after both presentations"
            );
            assert_eq!(
                probe(&mut host, 0, 33, 1464),
                if ui { 1 } else { 123 },
                "native connect overlay argument or suspended frame clock"
            );
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
            assert_eq!(renderer.sample.stats.rejected, 0);
            assert!(
                renderer.sample.stats.draws_2d > 0,
                "callback uses the actual CPU backend"
            );
            if !ui {
                for (address, value) in [(1456, 4), (1464, 1)] {
                    host.call_module_export(
                        ModuleId(2),
                        Export::words(CallbackId(33), &[address]),
                        ThinkTime::Milliseconds(123),
                    )
                    .unwrap();
                    assert!(
                        matches!(host.module_counts(ModuleId(2)).unwrap().last_result,
                        Some(ModuleResult::Qvm(actual)) if actual as u32 == value)
                            || matches!(host.module_counts(ModuleId(2)).unwrap().last_result,
                            Some(ModuleResult::Native(actual)) if actual as u32 == value)
                    );
                }
                assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
            }
        }
    }
    println!(
        "loading refresh native={native} ui={ui}: callback, recursion, resume, failure cleanup and prior seat preservation passed"
    );
}
