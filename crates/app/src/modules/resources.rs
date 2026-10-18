//! Resource publication uses the same retained output ring as module messages.
use super::{FrameHost, State};
use qa_core::{
    configstrings::ConfigLayout,
    events::OutputTarget,
    primitives::{ClientId, RuleSetId},
};

struct ModelPreparer<'a> {
    scene: &'a mut Option<crate::renderer::SceneComposer>,
    backend: &'a mut dyn crate::renderer::AssetPreparer,
    failed: bool,
}
impl ModelPreparer<'_> {
    fn prepare(
        &mut self,
        assets: &qa_render::Assets,
        required: Option<qa_render::Limits>,
        choices: usize,
    ) -> Result<(), String> {
        let result = (|| {
            let capacity = self
                .scene
                .as_mut()
                .map(|scene| scene.stage_model_choices(assets, required, choices))
                .transpose()?;
            self.backend.prepare_assets(
                assets,
                capacity
                    .as_ref()
                    .map(|capacity| capacity.limits())
                    .or(required),
            )?;
            if let Some(capacity) = capacity {
                capacity.commit();
            }
            Ok(())
        })();
        self.failed |= result.is_err();
        result
    }
}
impl crate::renderer::AssetPreparer for ModelPreparer<'_> {
    fn target_size(&self) -> Option<[u32; 2]> {
        self.backend.target_size()
    }
    fn prepare_assets(
        &mut self,
        assets: &qa_render::Assets,
        required: Option<qa_render::Limits>,
    ) -> Result<(), String> {
        self.prepare(assets, required, 0)
    }
    fn prepare_model_assets(
        &mut self,
        assets: &qa_render::Assets,
        per_entity: usize,
    ) -> Result<(), String> {
        self.prepare(assets, None, per_entity)
    }
}

impl FrameHost {
    pub fn publish_model_admission(
        &mut self,
        counts: qa_render::assets::AssetCounts,
    ) -> Result<(), qa_compat::services::CallError> {
        let owner = self
            .runtime
            .model_assets
            .as_mut()
            .ok_or(qa_compat::services::CallError::File)?;
        owner.publish_admission(counts)?;
        self.refresh_model_consumers();
        Ok(())
    }

    pub(crate) fn refresh_model_consumers(&mut self) {
        let Some(owner) = self.runtime.model_assets.as_ref() else {
            return;
        };
        let catalog = owner.catalog();
        for resources in self.runtime.client_resources.iter_mut().flatten() {
            let search = resources.state.file_search().clone();
            resources.state.rejected += u64::from(resources.state.models.admission_published(
                &resources.state.configs,
                catalog,
                Some(&search),
            ));
            resources.state.complete_static_models(catalog);
        }
    }

    fn refresh_player_skins(&mut self, preparer: &mut dyn crate::renderer::AssetPreparer) {
        if !self.player_skin_views.refresh_pending()
            || self
                .runtime
                .scene
                .as_mut()
                .is_none_or(|scene| !scene.asset_stage_ready())
            || self
                .runtime
                .model_assets
                .as_ref()
                .is_none_or(|owner| owner.initial_registration())
        {
            return;
        }
        let Some(request) = self.player_skin_views.take_refresh() else {
            return;
        };
        let Some(mut owner) = self.runtime.model_assets.take() else {
            return;
        };
        let mut vfs = std::mem::take(&mut self.runtime.vfs);
        let ui_fullscreen = self.renderer_fullscreen_ui();
        let result = (|| {
            let mounts: Vec<_> = vfs
                .mounts()
                .filter(|mount| mount.kind == qa_content::vfs::MountKind::Directory)
                .map(|mount| mount.id)
                .collect();
            for mount in mounts {
                vfs.refresh_directory_prefix(mount, b"skins")
                    .map_err(|error| format!("skin directory refresh: {error}"))?;
            }
            if let Some(modules) = &mut self.modules {
                modules.prepare_files(&vfs)?;
            }
            owner.image_transaction(
                &vfs,
                None,
                crate::model_assets::ImageRefreshMode::IndexedSkins,
                |owner, _| {
                    let mut clients = Vec::new();
                    for (index, resources) in self.runtime.client_resources.iter().enumerate() {
                        let Some(resources) = resources else {
                            continue;
                        };
                        let Some(seat) = self.runtime.snapshots[index]
                            .as_ref()
                            .and_then(|binding| binding.seat())
                        else {
                            continue;
                        };
                        clients.push((
                            index,
                            resources.state.stage_indexed_skin_refresh(
                                &self.player_skin_views,
                                &request,
                                &self.console.cvars,
                                seat,
                                owner,
                                &vfs,
                                preparer,
                                ui_fullscreen,
                            )?,
                        ));
                    }
                    let scene = self
                        .runtime
                        .scene
                        .as_mut()
                        .ok_or("missing skin refresh scene")?;
                    let capacity = scene.stage_model_choices(
                        owner.assets(),
                        None,
                        owner.catalog().max_choice_nodes(),
                    )?;
                    // Backend success is the last fallible operation. The receipt
                    // owns all staged frontend and CLIENT selectors until then.
                    preparer.prepare_assets(owner.assets(), Some(capacity.limits()))?;
                    capacity.commit();
                    for (index, client) in clients {
                        if let Some(resources) = self.runtime.client_resources[index].as_mut() {
                            resources.state.commit_image_refresh(client);
                        }
                    }
                    Ok(())
                },
            )
        })();
        self.runtime.vfs = vfs;
        self.runtime.model_assets = Some(owner);
        match result {
            Ok(()) => {
                self.player_skin_views.commit_refresh(request);
                for resources in self.runtime.client_resources.iter_mut().flatten() {
                    resources.startup.skins_prepared();
                }
            }
            Err(error) => {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("Player skin refresh rejected: {error}\n"));
            }
        }
    }

    pub fn register_client_model_assets(
        &mut self,
        mut preparer: Option<&mut dyn crate::renderer::AssetPreparer>,
    ) {
        self.flush_client_downloads();
        let resource_settings_ready = if let Err(error) = self.refresh_resource_settings() {
            let _ = self
                .runtime
                .server
                .events
                .console_formatted(format_args!("Image search setting rejected: {error}\n"));
            false
        } else {
            true
        };
        if resource_settings_ready {
            self.finish_client_downloads(|owner, vfs, path, scene, contexts| {
                let (changed, append) = owner.downloaded(
                    vfs,
                    path,
                    |owner, models_changed| {
                        let Some(scene) = scene.as_mut() else {
                            return Ok(None);
                        };
                        let Some(scene) =
                            scene.stage_downloaded_hud(vfs, owner, path, models_changed)?
                        else {
                            return Ok(None);
                        };
                        let graphs = contexts.stage_graphics(scene.registrations())?;
                        Ok(Some((scene, graphs)))
                    },
                    |assets, append| {
                        let backend = preparer
                            .as_deref_mut()
                            .ok_or("download refresh requires a resource preparer")?;
                        backend.prepare_assets(assets, append.map(|append| append.0.limits()))
                    },
                )?;
                if let Some((scene, graphs)) = append {
                    for graph in graphs {
                        graph.commit();
                    }
                    scene.commit();
                }
                Ok(changed)
            });
            self.advance_q3_package_downloads();
            self.advance_startup_files();
            if let Some(preparer) = preparer.as_mut() {
                self.refresh_player_skins(&mut **preparer);
            }
            self.advance_qw_skin_downloads();
        }
        let mut published = false;
        for index in 0..self.runtime.client_resources.len() {
            if self.runtime.client_resources[index]
                .as_ref()
                .is_some_and(|resources| !resources.startup.allow_models())
            {
                continue;
            }
            published |= self.prepare_received_hud(ClientId(index as u32), &mut preparer);
            if !self.prepare_received_world(ClientId(index as u32), &mut preparer) {
                continue;
            }
            let ui_fullscreen = self.renderer_fullscreen_ui();
            let runtime = &mut self.runtime;
            let Some(resources) = runtime.client_resources[index].as_mut() else {
                continue;
            };
            let source = self
                .modules
                .as_ref()
                .and_then(|modules| modules.row(runtime.server.clients[index].module))
                .map_or(resources.state.layout.rules, |module| {
                    module.request.context.native_rules
                });
            resources
                .state
                .models
                .player_options(crate::client_scene::PlayerModelOptions {
                    no_skins: self.console.cvars.integer_in(
                        self.player_model_options.0.canonical(),
                        resources.state.layout.rules,
                    ),
                    visible_weapons: self.console.cvars.integer_in(
                        self.player_model_options.1.canonical(),
                        resources.state.layout.rules,
                    ) != 0,
                });
            let seat = runtime.snapshots[index]
                .as_ref()
                .and_then(|binding| binding.seat());
            self.player_skin_views.update(
                &self.console.cvars,
                seat,
                &mut resources.state.models,
                None,
            );
            let search = resources.state.file_search().clone();
            let mut backend_failed = false;
            let rejected = if let Some(owner) = runtime.model_assets.as_mut() {
                if owner.initial_registration() {
                    resources.state.models.register_player_models(
                        &resources.state.configs,
                        owner,
                        &runtime.vfs,
                        source,
                        ui_fullscreen,
                        Some(&search),
                    )
                } else if let Some(preparer) = preparer.as_deref_mut() {
                    let mut preparer = ModelPreparer {
                        scene: &mut runtime.scene,
                        backend: preparer,
                        failed: false,
                    };
                    let (rejected, changed) = resources.state.prepare_models(
                        owner,
                        &runtime.vfs,
                        &mut preparer,
                        ui_fullscreen,
                    );
                    published |= changed;
                    backend_failed = preparer.failed;
                    rejected
                } else {
                    resources.state.models.refresh_player_models(
                        &resources.state.configs,
                        owner.catalog(),
                        Some(&search),
                    )
                }
            } else {
                0
            };
            resources.state.rejected += u64::from(rejected);
            let reject_startup =
                backend_failed && resources.startup.q3().is_some() && resources.awaiting_startup();
            if reject_startup {
                self.reject_client_startup(
                    ClientId(index as u32),
                    format_args!("model backend admission failed"),
                );
            }
        }
        if published {
            self.refresh_model_consumers();
        }
        if let Err(error) = self.prepare_graphics_contexts() {
            let _ = self
                .runtime
                .server
                .events
                .console_formatted(format_args!("engine graphics: {error}\n"));
        }
    }

    fn prepare_received_hud(
        &mut self,
        client: ClientId,
        preparer: &mut Option<&mut dyn crate::renderer::AssetPreparer>,
    ) -> bool {
        let slot = client.0 as usize;
        let Some(resources) = self.runtime.client_resources[slot].as_ref() else {
            return false;
        };
        if !resources.state.hud_pending() {
            return false;
        }
        let Some(binding) = self.runtime.snapshots[slot].as_ref() else {
            return false;
        };
        let Some(seat) = binding.seat() else {
            return false;
        };
        let Some((bound, row, _)) = self.local_seat(seat.index()) else {
            return false;
        };
        let lifetime = row.entity;
        let output = binding.output;
        let rules = row.client_rules;
        if bound != client
            || row.connection != Some(qa_session::clients::Connection::Local)
            || row.output != Some(output)
            || self.runtime.server.entities.resolve(lifetime).is_none()
        {
            return false;
        }
        let ui_fullscreen = self.renderer_fullscreen_ui();
        let Some(scene) = self.runtime.scene.as_mut() else {
            return false;
        };
        if scene.has_hud(rules, Some(seat), resources.state.file_source()) {
            if let Some(resources) = self.runtime.client_resources[slot].as_mut() {
                resources.state.take_hud_attempt();
            }
            return false;
        }
        if resources.state.presentation_reset_pending()
            || !scene.asset_stage_ready()
            || self
                .runtime
                .model_assets
                .as_ref()
                .is_none_or(|owner| owner.initial_registration())
        {
            return false;
        }
        let Some(preparer) = preparer.as_deref_mut() else {
            return false;
        };
        let Some([width, height]) = preparer.target_size() else {
            return false;
        };
        if width == 0 || height == 0 {
            return false;
        }
        let before = self
            .runtime
            .model_assets
            .as_ref()
            .map(|owner| owner.assets().counts());
        let result = (|| {
            let resources = self.runtime.client_resources[slot]
                .as_mut()
                .ok_or("missing CLIENT resources")?;
            let epoch = resources.state.models.revision();
            resources.state.take_hud_attempt();
            let settings = crate::render_settings::image_settings(&self.console.cvars, rules)?;
            let owner = self
                .runtime
                .model_assets
                .as_mut()
                .ok_or("missing model assets")?;
            let reuse_font = scene.hud_font(rules, Some(seat), resources.state.file_source());
            let contexts = &mut self.renderer_contexts;
            let client_profile = resources.state.models.profile();
            let reset = resources.state.completed_presentation_reset().flatten();
            let (append, _) = owner.prepare_hud(
                &self.runtime.vfs,
                rules,
                settings,
                crate::render_settings::material_finish(&self.console.cvars)?
                    .compile(ui_fullscreen),
                width,
                reuse_font,
                Some(resources.state.file_search()),
                |view, candidate| {
                    let profile = candidate.profile();
                    contexts.ensure_owner(
                        qa_render::shader::registration::ShaderOwner::Engine,
                        profile,
                        resources.state.files(),
                        view.assets,
                        (0, 0),
                        None,
                    )?;
                    if client_profile == Some(profile) {
                        contexts.ensure_owner(
                            qa_render::shader::registration::ShaderOwner::Client(client, lifetime),
                            profile,
                            resources.state.files(),
                            view.assets,
                            (0, 0),
                            reset,
                        )?;
                    }
                    let font = candidate.font();
                    let graphs = contexts.stage_graphics(
                        candidate
                            .registrations()
                            .map(move |registration| font.registered(registration)),
                    )?;
                    let scene = scene.stage_hud(candidate, Some(seat))?;
                    Ok((scene, graphs))
                },
                |assets, append| {
                    if self.runtime.server.clients.get(slot).is_none_or(|row| {
                        row.entity != lifetime
                            || row.output != Some(output)
                            || row.client_rules != rules
                    }) || resources.state.models.revision() != epoch
                    {
                        return Err("CLIENT HUD admission lifetime changed".to_owned());
                    }
                    preparer.prepare_assets(assets, Some(append.0.limits()))
                },
            )?;
            for graph in append.1 {
                graph.commit();
            }
            append.0.commit();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            if self.runtime.client_resources[slot]
                .as_ref()
                .is_some_and(|resources| {
                    resources.startup.q3().is_some() && resources.awaiting_startup()
                })
            {
                self.reject_client_startup(client, format_args!("HUD admission: {error}"));
            } else {
                if let Some(resources) = self.runtime.client_resources[slot].as_mut() {
                    resources.state.rejected += 1;
                }
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("CLIENT {} HUD: {error}\n", client.0));
            }
        }
        self.runtime
            .model_assets
            .as_ref()
            .map(|owner| owner.assets().counts())
            != before
    }

    fn prepare_received_world(
        &mut self,
        client: ClientId,
        preparer: &mut Option<&mut dyn crate::renderer::AssetPreparer>,
    ) -> bool {
        use qa_compat::services::{LoadedWorldBinding, ModelLookup};
        use qa_network::ingress::ConnectionPhase;
        let slot = client.0 as usize;
        let Some(resources) = self
            .runtime
            .client_resources
            .get(slot)
            .and_then(Option::as_ref)
        else {
            return true;
        };
        if !resources.state.world_managed() {
            return true;
        }
        if self
            .runtime
            .snapshots
            .get(slot)
            .and_then(Option::as_ref)
            .is_none_or(|binding| binding.presentation_closed())
        {
            return false;
        }
        let Some(request) = resources.state.pending_world() else {
            return resources.state.world_ready();
        };
        let search = resources.state.file_search().clone();
        let revision = request.revision;
        let config_row = request.row;
        let epoch = resources.state.models.revision();
        let Some(binding) = self.runtime.snapshots.get(slot).and_then(Option::as_ref) else {
            return false;
        };
        let Some(seat) = binding.seat() else {
            return false;
        };
        let protocol = binding.protocol;
        let Some(row) = self.runtime.server.clients.get(slot) else {
            return false;
        };
        let lifetime = row.entity;
        let output = binding.output;
        let source_rules = resources
            .state
            .models
            .profile()
            .map_or(resources.state.layout.rules, |profile| profile.source_rules);
        let presentation_rules = self.local_presentation_rules[seat.index()]
            .or_else(|| {
                resources
                    .state
                    .models
                    .profile()
                    .map(|profile| profile.rules)
            })
            .unwrap_or(row.client_rules);
        if self.local_clients[seat.index()] != Some(client)
            || row.connection != Some(qa_session::clients::Connection::Local)
            || row.output != Some(output)
            || self.runtime.server.entities.resolve(lifetime).is_none()
            || resources.state.presentation_reset_pending()
            || self
                .runtime
                .network
                .get(client, qa_core::loopback::Endpoint::Client)
                .is_none_or(|connection| !matches!(connection.phase, ConnectionPhase::Connected))
            || self
                .runtime
                .model_assets
                .as_ref()
                .is_none_or(|owner| owner.initial_registration())
        {
            return false;
        }
        let cached = (|| {
            let profile = resources.state.models.profile()?;
            if profile.source_rules != source_rules || profile.rules != presentation_rules {
                return None;
            }
            let selected = resources.state.models.world_binding()?.0;
            let owner = self.runtime.model_assets.as_ref()?;
            let world = request
                .with_name(|name| owner.catalog().loaded_world(name, Some(&search)))
                .ok()??;
            if world != selected || world.collision.is_none() {
                return None;
            }
            if !self.runtime.scene.as_ref()?.has_world(world.render) {
                return None;
            }
            let presentation = owner
                .view()
                .world_presentation(world.render, Some(profile), None, Some(&search))
                .filter(|entry| {
                    entry.compile_policy.collapse_vertex()
                        == profile.finish.compile(false).collapse_vertex()
                })?;
            Some((profile, world, presentation))
        })();
        if cached.is_none()
            && (preparer.is_none()
                || self
                    .runtime
                    .scene
                    .as_mut()
                    .is_none_or(|scene| !scene.asset_stage_ready()))
        {
            return false;
        }
        let trace_rules = self
            .runtime
            .server
            .clients
            .iter()
            .filter(|row| row.connection.is_some())
            .map(|row| row.player.trace_rules)
            .chain(
                self.runtime
                    .prediction
                    .iter()
                    .map(|prediction| prediction.player.trace_rules),
            )
            .chain(
                self.modules
                    .iter()
                    .flat_map(|modules| modules.rows.iter().flatten())
                    .map(|row| row.request.context.trace_rules),
            )
            .map(|rules| qa_world::collision::trace_policy(rules).0)
            .find(|rules| rules.hull_shape == qa_world::collision::HullShape::ExactBox)
            .unwrap_or_else(|| {
                qa_world::collision::trace_policy(
                    self.runtime.prediction[seat.index()].player.trace_rules,
                )
                .0
            });
        let previous_ui_fullscreen = self.renderer_fullscreen_ui();
        self.set_renderer_fullscreen_ui(false);
        let before = self
            .runtime
            .model_assets
            .as_ref()
            .map(|owner| owner.assets().counts());
        let result = (|| {
            if let Some(cached) = cached {
                return Ok(cached);
            }
            let preparer = preparer
                .as_deref_mut()
                .ok_or("CLIENT world admission requires a resource preparer")?;
            let runtime = &mut self.runtime;
            let resources = runtime.client_resources[slot]
                .as_mut()
                .ok_or("missing CLIENT resources")?;
            let request = resources
                .state
                .pending_world()
                .ok_or("CLIENT world request changed")?;
            let path = request.virtual_path();
            let path = path?;
            let owner = runtime
                .model_assets
                .as_mut()
                .ok_or("missing model assets")?;
            let scene = runtime.scene.as_mut().ok_or("missing scene owner")?;
            let image_settings = resources
                .state
                .models
                .profile()
                .filter(|profile| profile.rules == presentation_rules)
                .map_or_else(
                    || {
                        crate::render_settings::image_settings(
                            &self.console.cvars,
                            presentation_rules,
                        )
                    },
                    |profile| Ok(profile.image_settings),
                )?;
            let (profile, _) = owner.prepare_profile(
                &runtime.vfs,
                source_rules,
                presentation_rules,
                image_settings,
                crate::render_settings::material_finish(&self.console.cvars)?,
                Some(&search),
                |assets| preparer.prepare_assets(assets, None),
            )?;
            let current = || {
                runtime.server.clients.get(slot).is_some_and(|row| {
                    row.entity == lifetime
                        && row.output == Some(output)
                        && row.connection == Some(qa_session::clients::Connection::Local)
                        && runtime.server.entities.resolve(lifetime).is_some()
                }) && runtime.snapshots[slot].as_ref().is_some_and(|binding| {
                    binding.output == output
                        && binding.seat() == Some(seat)
                        && binding.protocol == protocol
                }) && runtime
                    .network
                    .get(client, qa_core::loopback::Endpoint::Client)
                    .is_some_and(|connection| connection.phase.is_connected())
                    && resources.state.models.revision() == epoch
                    && resources
                        .state
                        .configs
                        .config(config_row)
                        .is_some_and(|row| row.revision == revision)
                    && !resources.state.presentation_reset_pending()
            };
            if !current() {
                return Err("CLIENT world admission lifetime changed".to_owned());
            }
            let mut presentation =
                qa_render::material::world_load::WorldPresentationProfile::native(
                    profile.rules,
                    profile.palette,
                );
            presentation.image_settings = profile.image_settings;
            presentation.finish = profile.finish;
            if let Some(world) = owner.catalog().loaded_world(path.as_bytes(), Some(&search)) {
                if world.collision.is_none() {
                    return Err("CLIENT world has no collision model".to_owned());
                }
                if let Some(presentation) = owner
                    .view()
                    .world_presentation(world.render, Some(profile), None, Some(&search))
                    .filter(|entry| {
                        scene.has_world(world.render)
                            && entry.compile_policy.collapse_vertex()
                                == profile.finish.compile(false).collapse_vertex()
                    })
                {
                    return Ok((profile, world, presentation));
                }
                let (loaded, append, _) = owner.prepare_world_presentation(
                    &runtime.vfs,
                    world.render,
                    profile,
                    presentation,
                    false,
                    Some(&search),
                    |view, loaded| scene.stage_world(view.assets, loaded),
                    |assets, append| {
                        if !current() {
                            return Err("CLIENT world admission lifetime changed".to_owned());
                        }
                        preparer.prepare_assets(assets, Some(append.limits()))
                    },
                )?;
                let presentation = loaded.presentations[0];
                append.commit();
                return Ok((profile, world, presentation));
            }
            let input = match resources
                .startup
                .q2_mut()
                .and_then(|state| state.world_input.take())
            {
                Some(input)
                    if input.virtual_path == path && input.search().same_selection(&search) =>
                {
                    input
                }
                _ => crate::map::read(runtime.vfs.read_view(Some(&search)), &path)?,
            };
            let (loaded, (append, scratches), _) = owner.prepare_world(
                &runtime.vfs,
                &mut runtime.geometry,
                input,
                crate::model_assets::WorldRegistration {
                    search: Some(search.clone()),
                    profile,
                    presentation,
                    options: crate::render_settings::world_load_options(
                        &self.console.cvars,
                        runtime.renderer_configuration.as_deref(),
                    )?,
                    trace_rules,
                    ui_fullscreen: false,
                },
                |view, store, loaded| {
                    let append = scene.stage_world(view.assets, &loaded.render)?;
                    let destinations = crate::runtime_trace_scratches(
                        &mut runtime.collision,
                        &mut runtime.client_collision,
                    )
                    .chain(
                        self.modules
                            .iter_mut()
                            .flat_map(|modules| modules.rows.iter_mut().flatten())
                            .map(|row| &mut row.scratch),
                    );
                    let scratches = crate::TraceScratchStage::prepare(store, destinations);
                    Ok((append, scratches))
                },
                |assets, (append, _)| {
                    if !current() {
                        return Err("CLIENT world admission lifetime changed".to_owned());
                    }
                    preparer.prepare_assets(assets, Some(append.limits()))
                },
            )?;
            let presentation = loaded.render.presentations[0];
            append.commit();
            scratches.commit();
            Ok((
                profile,
                LoadedWorldBinding {
                    render: loaded.render.world,
                    collision: Some(loaded.collision),
                },
                presentation,
            ))
        })();
        let result = result.and_then(|(profile, world, presentation)| {
            self.prepare_client_renderer_world(
                client,
                lifetime,
                profile,
                world.render,
                presentation,
                preparer,
            )
            .map(|ready| ready.then_some((profile, world, presentation)))
        });
        if self
            .runtime
            .model_assets
            .as_ref()
            .map(|owner| owner.assets().counts())
            != before
        {
            self.refresh_model_consumers();
        }
        match result {
            Ok(Some((profile, world, presentation))) => {
                self.activate_received_world(
                    client,
                    seat,
                    profile,
                    world,
                    presentation,
                    (config_row, revision),
                );
                true
            }
            Ok(None) => {
                self.set_renderer_fullscreen_ui(previous_ui_fullscreen);
                false
            }
            Err(error) => {
                self.set_renderer_fullscreen_ui(previous_ui_fullscreen);
                if let Some(resources) = self.runtime.client_resources[slot].as_mut() {
                    resources.state.take_world_attempt();
                }
                if self.runtime.client_resources[slot]
                    .as_ref()
                    .is_some_and(|resources| resources.awaiting_startup())
                {
                    self.reject_client_startup(client, format_args!("{error}"));
                } else {
                    if let Some(resources) = self.runtime.client_resources[slot].as_mut() {
                        resources.state.rejected += 1;
                    }
                    let _ = self
                        .runtime
                        .server
                        .events
                        .console_formatted(format_args!("CLIENT world admission: {error}\n"));
                }
                false
            }
        }
    }

    fn prepare_client_renderer_world(
        &mut self,
        client: ClientId,
        lifetime: qa_core::primitives::EntityId,
        profile: crate::model_assets::ModelProfile,
        world: qa_render::world::WorldId,
        presentation: qa_render::material::world_load::LoadedWorldPresentation,
        preparer: &mut Option<&mut dyn crate::renderer::AssetPreparer>,
    ) -> Result<bool, String> {
        let count = self
            .modules
            .as_ref()
            .map_or(0, |modules| modules.rows.len());
        for index in 0..count {
            let module = qa_core::primitives::ModuleId(index as u16);
            let Some(row) = self.module_row(module).filter(|row| {
                row.produces_client_view && row.client_connection == Some((client, lifetime))
            }) else {
                continue;
            };
            let Some(selected_profile) = row.scene_role.profile() else {
                continue;
            };
            let files = self.client_renderer_files(row)?;
            let same_files = self.runtime.client_resources[client.0 as usize]
                .as_ref()
                .is_some_and(|resources| {
                    files.view().same_selection(resources.state.file_search())
                });
            let selected_profile = if selected_profile.source_rules == profile.source_rules
                && selected_profile.rules == profile.rules
            {
                profile
            } else {
                selected_profile
            };
            let selected = if selected_profile == profile && same_files {
                Some(presentation)
            } else {
                self.runtime.model_assets.as_ref().and_then(|owner| {
                    owner
                        .view()
                        .world_presentation(world, Some(selected_profile), None, Some(files.view()))
                        .filter(|entry| {
                            entry.compile_policy.collapse_vertex()
                                == selected_profile.finish.compile(false).collapse_vertex()
                        })
                })
            };
            let selected = if let Some(selected) = selected {
                selected
            } else {
                if self
                    .runtime
                    .scene
                    .as_mut()
                    .is_none_or(|scene| !scene.asset_stage_ready())
                {
                    return Ok(false);
                }
                let Some(preparer) = preparer.as_deref_mut() else {
                    return Ok(false);
                };
                let owner = self
                    .runtime
                    .model_assets
                    .as_mut()
                    .ok_or("missing model assets")?;
                let scene = self.runtime.scene.as_mut().ok_or("missing scene owner")?;
                let mut policy = qa_render::material::world_load::WorldPresentationProfile::native(
                    selected_profile.rules,
                    selected_profile.palette,
                );
                policy.image_settings = selected_profile.image_settings;
                policy.finish = selected_profile.finish;
                let (loaded, append, _) = owner.prepare_world_presentation(
                    &self.runtime.vfs,
                    world,
                    selected_profile,
                    policy,
                    false,
                    Some(files.view()),
                    |view, loaded| scene.stage_world(view.assets, loaded),
                    |assets, append| preparer.prepare_assets(assets, Some(append.limits())),
                )?;
                let selected = loaded.presentations[0];
                append.commit();
                selected
            };
            let row = self.module_row(module).ok_or("missing CLIENT renderer")?;
            if row.scene_role.profile() == Some(selected_profile)
                && row.scene_role.presentation_for(world) == Some(selected.id)
            {
                continue;
            }
            self.module_row_mut(module)
                .ok_or("missing CLIENT renderer")?
                .scene_role
                .adopt_client_world(selected_profile, world, selected);
        }
        Ok(true)
    }

    fn activate_received_world(
        &mut self,
        client: ClientId,
        seat: qa_core::sys_events::SeatId,
        profile: crate::model_assets::ModelProfile,
        world: qa_compat::services::LoadedWorldBinding,
        presentation: qa_render::material::world_load::LoadedWorldPresentation,
        request: (usize, u64),
    ) {
        let slot = client.0 as usize;
        let owner = self.runtime.model_assets.as_ref();
        if let (Some(resources), Some(owner)) =
            (self.runtime.client_resources[slot].as_mut(), owner)
        {
            resources.state.rejected += u64::from(resources.state.bind_models(
                owner.catalog(),
                profile,
                Some((world.render, presentation.id)),
            ));
            resources.state.world_admitted(request.0, request.1);
        }
        self.local_worlds[seat.index()] = Some((world.render, presentation.id));
        self.local_presentations[seat.index()] = presentation.defaults;
        if let Some(collision) = self.runtime.client_collision[slot].as_mut() {
            collision.invalidate();
        }
    }
    /// Module renderer provenance is independent of the SERVER publisher's
    /// configstring profile. Bind it before CLIENT initialization freezes assets.
    pub fn configure_native_scenes(
        &mut self,
        defaults: &[Option<crate::model_assets::ModelProfile>; 5],
        seats: &[[Option<crate::model_assets::ModelProfile>; 5];
             qa_core::sys_events::SeatId::COUNT],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if self.modules.as_ref().is_some_and(|modules| {
            modules.rows.iter().flatten().any(|row| {
                row.request.phase == super::Phase::Client
                    && row
                        .renderer_generation
                        .and_then(super::RendererGeneration::stamp)
                        .is_some()
            })
        }) {
            return Err(
                "module renderer configuration requires a cold CLIENT generation".to_owned(),
            );
        }
        let Some(modules) = self.modules.as_mut() else {
            return Ok(());
        };
        let count = self.local_clients.iter().flatten().count();
        for row in modules
            .rows
            .iter_mut()
            .flatten()
            .filter(|row| row.request.phase == super::Phase::Client)
        {
            let source = row.request.context.native_rules as usize;
            let seat = super::renderer_client(
                &self.runtime,
                &self.local_clients,
                &row.request,
                row.client_connection,
            )
            .map(|client| client.seat)
            .or_else(|| {
                row.request
                    .program
                    .imports()
                    .and_then(qa_compat::abi::CallTable::command_export)
                    .filter(|entry| entry.role == qa_compat::abi::CommandRole::Ui)
                    .map(|_| row.request.context.console.seat)
            });
            let profile = seat
                .and_then(|seat| seats[seat.index()][source])
                .or(defaults[source])
                .ok_or("missing module renderer profile")?;
            let (viewport, world, presentation) = if let Some(seat) = seat {
                let view_index = self.local_clients[..seat.index()].iter().flatten().count();
                (
                    crate::renderer::viewport(width, height, view_index, count),
                    self.local_worlds[seat.index()],
                    self.local_presentations[seat.index()],
                )
            } else {
                (
                    qa_render::Viewport {
                        x: 0,
                        y: 0,
                        width,
                        height,
                    },
                    None,
                    Default::default(),
                )
            };
            let context = row.request.context.console;
            let rails = ["r_railCoreWidth", "r_railWidth", "r_railSegmentLength"]
                .map(|name| self.console.cvars.bind(name, context));
            let [Some(core), Some(ring), Some(length)] = rails else {
                return Err("missing rail renderer cvars".into());
            };
            let lighting = ["r_ambientScale", "r_directedScale"]
                .map(|name| self.console.cvars.bind(name, context));
            let [Some(ambient), Some(directed)] = lighting else {
                return Err("missing entity lighting cvars".into());
            };
            let software_waterwarp = self
                .console
                .cvars
                .bind("r_waterwarp", context)
                .ok_or("missing software water warp cvar")?;
            let curve_error = crate::render_settings::curve_error_view(&self.console.cvars)?;
            let model_lod = crate::render_settings::model_lod_views(&self.console.cvars)?;
            let view_policy =
                crate::renderer::ViewPolicyViews::bind(&self.console.cvars, profile.rules)?;
            row.scene_role.configure(
                profile,
                viewport,
                seat,
                world,
                presentation,
                [core, ring, length],
                [ambient, directed],
                software_waterwarp,
                curve_error,
                model_lod,
                self.model_controls,
                view_policy,
            );
        }
        let count = modules.rows.len();
        for index in 0..count {
            let module = qa_core::primitives::ModuleId(index as u16);
            if self
                .module_row(module)
                .is_some_and(|row| row.renderer_context.is_some())
            {
                self.bind_module_renderer_context(module)?;
            }
        }
        Ok(())
    }
    /// Native Q3 SERVER storage supplies its own gamestate. Encoding borrows
    /// the source rows; only the existing output ring retains the message.
    pub(crate) fn publish_module_game_states(&mut self) {
        let Some(modules) = &mut self.modules else {
            return;
        };
        for row in modules.rows.iter_mut().flatten() {
            if row.state != State::Running {
                continue;
            }
            if let Some(info) = &mut row.server_info
                && !info.prepare(
                    &self.console.cvars,
                    &self.runtime.vfs,
                    modules.storage.file_search(row.request.context.module),
                    modules.storage.file_source(row.request.context.module),
                )
            {
                self.runtime.server.events.reject_publication();
            }
        }
        let runtime = &mut self.runtime;
        for (index, client) in runtime.server.clients.iter().enumerate() {
            let Some(output) = client.output else {
                continue;
            };
            if !client.is_active() || runtime.server.events.needs_resync(output) {
                continue;
            }
            let Some(row) = modules.row(client.module).filter(|row| {
                row.request.phase == super::Phase::Server
                    && row.request.context.native_rules == RuleSetId::Quake3
            }) else {
                continue;
            };
            let id = ClientId(index as u32);
            let Some(connection) = runtime
                .network
                .get_mut(id, qa_core::loopback::Endpoint::Server)
            else {
                continue;
            };
            if connection
                .commands
                .as_ref()
                .map(|commands| commands.protocol)
                != Some(qa_network::commands::packet::Protocol::Quake3_68)
            {
                continue;
            }
            let publication = &mut runtime.config_publications[index];
            publication.connection(client.module, ConfigLayout::QUAKE3);
            let model_world = crate::client_resources::model_world(
                client.connection,
                runtime.client_resources[index].as_ref(),
                runtime.collision.as_ref().map(|world| world.geometry),
            );
            publication.model_world(model_world);
            publication.require_game_state();
            let Some(info) = row.server_info.as_ref().filter(|info| info.ready) else {
                continue;
            };
            let channel = &mut connection.channel;
            let Ok(mut context) = channel.command_context() else {
                continue;
            };
            context.set_system_info(info.text(ConfigLayout::Q3_SYSTEM_INFO));
            let source = modules.storage.file_search(client.module);
            let key = Some((
                context.checksum_feed,
                modules.storage.file_source(client.module),
                runtime.vfs.revision(),
            ));
            if publication.pure_source != key {
                let view = runtime.vfs.read_view(source);
                let feed = context.checksum_feed;
                let loaded = match view.q3_loaded_pure_checksums(feed) {
                    Ok(loaded) => loaded,
                    Err(_) => {
                        runtime.server.events.reject_publication();
                        continue;
                    }
                };
                let policy = qa_network::pure::Policy::new(
                    feed,
                    [b"vm/cgame.qvm".as_slice(), b"vm/ui.qvm"]
                        .map(|name| view.q3_file_pure_checksum(name, feed)),
                    loaded,
                );
                let changed = publication
                    .pure
                    .refresh_policy(policy, feed, context.server_id)
                    || publication.pure_source.map(|key| key.1) != key.map(|key| key.1);
                publication.pure_source = key;
                if changed && publication.game_state_published() {
                    publication.restart_game_state();
                }
            }
            publication.pure.set_enabled(info.pure);
            if row.state != State::Running || publication.game_state_published() {
                continue;
            }
            let Some(native_client) = runtime.snapshots[index]
                .as_ref()
                .and_then(|binding| binding.native_client())
            else {
                continue;
            };
            let command_sequence = channel.command_state().map_or(0, |state| state.queued);
            let configs = (0..row.request.configstrings.max(2)).filter_map(|native_index| {
                if native_index <= ConfigLayout::Q3_SYSTEM_INFO {
                    Some((native_index, info.text(native_index)))
                } else {
                    modules
                        .storage
                        .configstring(client.module, native_index)
                        .ok()
                        .map(|(text, _)| (native_index, text))
                }
            });
            let mut writer = qa_network::message::Writer::new(
                &mut runtime.output_native_bytes,
                qa_network::message::Encoding::Q3,
            );
            if qa_network::channel::commands::write_gamestate(
                &mut writer,
                command_sequence,
                native_client,
                context.checksum_feed,
                configs,
                // These connections currently use zero snapshot baselines.
                // Their first snapshot supplies full zero-base deltas.
                std::iter::empty(),
            )
            .is_err()
            {
                runtime.server.events.reject_publication();
                continue;
            }
            let (length, body_bits) = (writer.size(), writer.bit_position());
            let delivery = qa_core::primitives::NativeDelivery {
                module: client.module,
                source: RuleSetId::Quake3,
                target: qa_core::primitives::NativeTarget::Client(id),
                origin: client.player.body.position,
                reliable: true,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: false,
            };
            let Ok(sequence) = runtime.server.events.game_state(
                delivery,
                command_sequence,
                body_bits,
                &runtime.output_native_bytes[..length],
            ) else {
                continue;
            };
            let _ = channel.set_command_context(context);
            publication
                .pure
                .gamestate(context.checksum_feed, context.server_id);
            publication.publish_game_state(sequence);
            publication.server_info = [0, 1].map(|index| Some(info.revision(index)));
            for native_index in 0..row.request.configstrings {
                if let Ok((text, revision)) =
                    modules.storage.configstring(client.module, native_index)
                {
                    publication.seen(native_index, revision);
                    if ConfigLayout::QUAKE3.resources[0].contains(native_index) {
                        let ordinal =
                            native_index - ConfigLayout::QUAKE3.resources[0].first as usize;
                        publication.primary_model(
                            ordinal as u32,
                            !text.is_empty(),
                            modules
                                .model_name(client.module, ordinal as u32, model_world, None)
                                .and_then(|model| model.name),
                        );
                    }
                    if ConfigLayout::QUAKE3.resources[1].contains(native_index) {
                        let ordinal =
                            native_index - ConfigLayout::QUAKE3.resources[1].first as usize;
                        let sample = modules
                            .storage
                            .sound_id(
                                client.module,
                                ConfigLayout::QUAKE3.resources[1],
                                ordinal as i32,
                            )
                            .ok()
                            .flatten();
                        publication.primary_sound(ordinal as u32, !text.is_empty(), sample);
                    }
                    publication.published(native_index, Some(sequence));
                }
            }
            for native_index in [ConfigLayout::Q3_SERVER_INFO, ConfigLayout::Q3_SYSTEM_INFO] {
                publication.published(native_index, Some(sequence));
            }
        }
    }
    /// Cold registration only. CLIENT rendering never calls module storage.
    pub fn model_precaches(&self) -> Vec<(RuleSetId, &[u8], Option<&qa_content::vfs::SearchView>)> {
        let mut names = Vec::new();
        let Some(modules) = &self.modules else {
            return names;
        };
        for row in modules.rows.iter().flatten() {
            if row.state != State::Running {
                continue;
            }
            let rules = row.request.context.native_rules;
            let Some(models) = modules
                .storage
                .resources(row.request.context.module)
                .map(|resources| resources[0])
            else {
                continue;
            };
            for index in models.range().skip(1) {
                if let Ok((text, _)) = modules
                    .storage
                    .configstring(row.request.context.module, index)
                    && !text.is_empty()
                    && !text.starts_with(b"*")
                {
                    names.push((
                        rules,
                        text,
                        modules.storage.file_search(row.request.context.module),
                    ));
                }
            }
        }
        names
    }

    /// Provenance comes from each client's publishing module, never its wire
    /// protocol or selected presentation. This is a cold registration query.
    pub fn client_model_sources(&self) -> Vec<(ClientId, RuleSetId)> {
        self.runtime
            .server
            .clients
            .iter()
            .enumerate()
            .filter_map(|(index, client)| {
                client.connection.as_ref()?;
                let row = self
                    .modules
                    .as_ref()?
                    .row(client.module)
                    .filter(|row| row.state == State::Running)?;
                Some((ClientId(index as u32), row.request.context.native_rules))
            })
            .collect()
    }

    pub fn bind_client_model_assets(
        &mut self,
        profiles: impl IntoIterator<
            Item = (
                ClientId,
                crate::model_assets::ModelProfile,
                Option<(
                    qa_render::world::WorldId,
                    qa_render::world::WorldPresentationId,
                )>,
            ),
        >,
    ) {
        let Some(owner) = self.runtime.model_assets.as_mut() else {
            return;
        };
        owner.end_initial_registration();
        let models = owner.catalog();
        for resources in self.runtime.client_resources.iter_mut().flatten() {
            resources.state.models.reset();
        }
        for (client, profile, world) in profiles {
            let Some(resources) = self
                .runtime
                .client_resources
                .get_mut(client.0 as usize)
                .and_then(Option::as_mut)
            else {
                continue;
            };
            resources.state.rejected +=
                u64::from(resources.state.bind_models(models, profile, world));
        }
        for resources in self.runtime.client_resources.iter_mut().flatten() {
            resources.state.complete_static_models(models);
        }
    }

    /// A row is recorded as published only after its text lease reaches the
    /// ring. Rejected rows are retried; successful rows are not duplicated.
    pub(crate) fn configure_output_resources(&mut self) {
        let capacity = self
            .modules
            .as_ref()
            .map_or(0, |modules| modules.rows.len());
        for publication in &mut self.runtime.config_publications {
            publication.configure_sources(
                capacity,
                self.modules
                    .as_ref()
                    .map_or(0, |modules| modules.storage.resource_name_capacity()),
            );
        }
    }
    pub(crate) fn publish_module_configs(&mut self) {
        let modules = self.modules.as_ref();
        let runtime = &mut self.runtime;
        // The window shell and isolated channel consumers have no loaded
        // SERVER. Native modules or registered collision geometry supply it.
        let publish_limits = modules.is_some() || runtime.geometry.geometries().next().is_some();
        for (index, client) in runtime.server.clients.iter().enumerate() {
            let Some(output) = client.output else {
                continue;
            };
            if !client.is_active() || runtime.server.events.needs_resync(output) {
                continue;
            }
            let Some(protocol) = runtime
                .network
                .get(ClientId(index as u32), qa_core::loopback::Endpoint::Server)
                .and_then(|connection| connection.commands.as_ref())
                .map(|commands| commands.protocol)
            else {
                continue;
            };
            let Some(target) = protocol.config_layout() else {
                continue;
            };
            let publication = &mut runtime.config_publications[index];
            publication.connection(client.module, target);
            let model_world = crate::client_resources::model_world(
                client.connection,
                runtime.client_resources[index].as_ref(),
                runtime.collision.as_ref().map(|world| world.geometry),
            );
            publication.model_world(model_world);
            runtime.native_numbers[index].refresh(
                &runtime.server.entities,
                runtime.server.entities.columns.native_entity[client.entity.slot as usize]
                    .map(|native| native.module),
                protocol,
                runtime
                    .server
                    .clients
                    .iter()
                    .filter(|client| client.is_server_active())
                    .map(|client| client.entity),
            );
            if publish_limits && let Some(destination) = target.max_clients() {
                let count = runtime.native_numbers[index].player_count();
                if publication.needs_client_limit(count) {
                    let mut text = qa_core::text::FixedText::<16>::default();
                    use std::fmt::Write;
                    if write!(&mut text, "{count}").is_err() {
                        continue;
                    }
                    let Ok(sequence) = runtime.server.events.publish_configstring(
                        Some(OutputTarget::Client(ClientId(index as u32))),
                        client.module,
                        target.rules,
                        destination as u32,
                        destination as u32,
                        text.as_bytes(),
                    ) else {
                        continue;
                    };
                    publication.published_client_limit(destination, count, sequence);
                }
            }
            let Some(modules) = modules else { continue };
            if target.rules == RuleSetId::Quake3
                && publication.game_state_published()
                && let Some(info) = modules
                    .row(client.module)
                    .and_then(|row| row.server_info.as_ref())
                && info.ready
            {
                let system_published = info.publish(
                    publication,
                    &mut runtime.server.events,
                    ClientId(index as u32),
                    client.module,
                );
                if system_published
                    && let Some(connection) = runtime
                        .network
                        .get_mut(ClientId(index as u32), qa_core::loopback::Endpoint::Server)
                    && let Ok(mut context) = connection.channel.command_context()
                {
                    context.set_system_info(info.text(ConfigLayout::Q3_SYSTEM_INFO));
                    let _ = connection.channel.set_command_context(context);
                }
            }

            let changed = modules
                .rows
                .iter()
                .flatten()
                .filter(|row| row.state == State::Running)
                .any(|row| {
                    modules
                        .storage
                        .config_revision(row.request.context.module)
                        .is_ok_and(|revision| {
                            publication.begin(row.request.context.module, revision)
                        })
                });
            if !changed {
                continue;
            }
            let mut complete = true;
            // General rows and native resource claims belong to the primary
            // namespace; secondary filenames use its free destination slots.
            if let Some(row) = modules
                .row(client.module)
                .filter(|row| row.state == State::Running)
            {
                let source = ConfigLayout::for_rules(row.request.context.native_rules);
                for native_index in 0..row.request.configstrings.min(source.count) {
                    let Ok((text, revision)) =
                        modules.storage.configstring(client.module, native_index)
                    else {
                        continue;
                    };
                    if !publication.needs(native_index, revision) {
                        continue;
                    }
                    if source.rules == RuleSetId::Quake3
                        && target.rules == RuleSetId::Quake3
                        && row.server_info.is_some()
                        && native_index <= ConfigLayout::Q3_SYSTEM_INFO
                    {
                        // SERVER info is projected from engine cvars, not a
                        // module's empty or private copy of those two rows.
                        publication.seen(native_index, revision);
                        continue;
                    }
                    let target_index = target.project_index(source, native_index);
                    if source.resources[0].contains(native_index) {
                        let ordinal = native_index - source.resources[0].first as usize;
                        if ordinal != 0 && !publication.primary_model_fits(ordinal as u32) {
                            if let Some(name) = modules
                                .model_name(client.module, ordinal as u32, model_world, None)
                                .and_then(|model| model.name)
                                && !publish_model(
                                    publication,
                                    &mut runtime.server.events,
                                    output,
                                    ClientId(index as u32),
                                    client.module,
                                    source.rules,
                                    native_index as u32,
                                    target,
                                    protocol,
                                    name,
                                    text,
                                )
                            {
                                complete = false;
                                break;
                            }
                            publication.seen(native_index, revision);
                            continue;
                        }
                        publication.primary_model(
                            ordinal as u32,
                            !text.is_empty(),
                            modules
                                .model_name(client.module, ordinal as u32, model_world, None)
                                .and_then(|model| model.name),
                        );
                    }
                    let is_sound = source.resources[1].contains(native_index);
                    let mut sample = None;
                    if is_sound {
                        let ordinal = native_index - source.resources[1].first as usize;
                        sample = modules
                            .storage
                            .sound_id(client.module, source.resources[1], ordinal as i32)
                            .ok()
                            .flatten();
                        publication.primary_sound(ordinal as u32, !text.is_empty(), sample);
                    }
                    let Some(destination) = target_index else {
                        publication.unsupported += 1;
                        publication.seen(native_index, revision);
                        continue;
                    };
                    if target.max_clients() == Some(destination) {
                        // Player ordinals belong to this connection's native
                        // namespace, independently of the publishing module.
                        publication.seen(native_index, revision);
                        continue;
                    }
                    // Preserve exact text only when both native filename
                    // codecs agree, including Quake's literal prefix bytes.
                    let converted;
                    let text = if is_sound
                        && !text.is_empty()
                        && !qa_audio::same_sound_codec(source.rules, target.rules)
                    {
                        let Some(name) = runtime
                            .sound_assets
                            .as_deref()
                            .map(crate::audio::SoundAssets::bank)
                            .zip(sample)
                            .and_then(|(bank, sample)| {
                                qa_audio::native_sound_name(target.rules, bank, sample)
                            })
                        else {
                            publication.unsupported += 1;
                            publication.published(destination, None);
                            publication.seen(native_index, revision);
                            continue;
                        };
                        converted = name;
                        converted.as_bytes()
                    } else {
                        text
                    };
                    if protocol
                        .config_capacity(destination)
                        .is_none_or(|limit| text.len() > limit)
                    {
                        publication.unsupported += 1;
                        publication.published(destination, None);
                        publication.seen(native_index, revision);
                        continue;
                    }
                    if runtime.server.events.needs_resync(output) {
                        complete = false;
                        break;
                    }
                    match runtime.server.events.publish_configstring(
                        Some(OutputTarget::Client(ClientId(index as u32))),
                        client.module,
                        source.rules,
                        native_index as u32,
                        destination as u32,
                        text,
                    ) {
                        Ok(sequence) => {
                            publication.published(destination, Some(sequence));
                            publication.seen(native_index, revision);
                        }
                        Err(_) => {
                            complete = false;
                            break;
                        }
                    }
                }
            }
            publication.release_aliases();
            publication.release_model_aliases();
            for row in modules
                .rows
                .iter()
                .flatten()
                .filter(|row| row.state == State::Running)
            {
                let module = row.request.context.module;
                let Some(models) = modules
                    .storage
                    .resources(module)
                    .map(|resources| resources[0])
                else {
                    continue;
                };
                for ordinal in 1..models.count {
                    if module == client.module
                        && (!publication.primary_model_fits(ordinal)
                            || publication.model_alias_fits(ordinal))
                    {
                        continue;
                    }
                    let Some(name) = modules
                        .model_name(module, ordinal, model_world, None)
                        .and_then(|model| model.name)
                    else {
                        continue;
                    };
                    let Some(text) = modules.storage.resource_name(name) else {
                        continue;
                    };
                    if !publish_model(
                        publication,
                        &mut runtime.server.events,
                        output,
                        ClientId(index as u32),
                        module,
                        row.request.context.native_rules,
                        models.first + ordinal,
                        target,
                        protocol,
                        name,
                        text,
                    ) {
                        complete = false;
                        break;
                    }
                }
            }
            if let Some(bank) = runtime
                .sound_assets
                .as_deref()
                .map(crate::audio::SoundAssets::bank)
            {
                // Running secondary module resources get first claim on free
                // slots; resolving a PCM handle never rebuilds or hashes a path.
                for row in modules.rows.iter().flatten().filter(|row| {
                    row.state == State::Running && row.request.context.module != client.module
                }) {
                    let module = row.request.context.module;
                    let Some(sounds) = modules
                        .storage
                        .resources(module)
                        .map(|resources| resources[1])
                    else {
                        continue;
                    };
                    for ordinal in 1..sounds.count {
                        let Some(sound) = modules
                            .storage
                            .sound_id(module, sounds, ordinal as i32)
                            .ok()
                            .flatten()
                        else {
                            continue;
                        };
                        if !publish_sound(
                            publication,
                            &mut runtime.server.events,
                            output,
                            ClientId(index as u32),
                            module,
                            row.request.context.native_rules,
                            sounds.first + ordinal,
                            target,
                            protocol,
                            bank,
                            sound,
                        ) {
                            complete = false;
                            break;
                        }
                    }
                }
                // Registered PCM names are immutable. Preserve old numeric
                // events after a primary row rewrite by relocating their sample,
                // including when its module no longer declares the old ordinal.
                for sound in 1..bank.len() {
                    if !publish_sound(
                        publication,
                        &mut runtime.server.events,
                        output,
                        ClientId(index as u32),
                        client.module,
                        target.rules,
                        target.resources[1].first,
                        target,
                        protocol,
                        bank,
                        qa_core::primitives::SoundId(sound as u32),
                    ) {
                        complete = false;
                        break;
                    }
                }
            }
            if complete {
                for row in modules
                    .rows
                    .iter()
                    .flatten()
                    .filter(|row| row.state == State::Running)
                {
                    if let Ok(revision) =
                        modules.storage.config_revision(row.request.context.module)
                    {
                        publication.finish(row.request.context.module, revision);
                    }
                }
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Source provenance and destination namespace stay independent at publication"
)]
fn publish_sound(
    publication: &mut crate::client_resources::ConfigPublication,
    events: &mut qa_core::events::EventRing,
    output: qa_core::events::OutputConsumerId,
    client: ClientId,
    module: qa_core::primitives::ModuleId,
    source: RuleSetId,
    source_index: u32,
    target: ConfigLayout,
    protocol: qa_network::commands::packet::Protocol,
    bank: &qa_audio::Bank,
    sound: qa_core::primitives::SoundId,
) -> bool {
    if bank.sample(sound).is_none() {
        return true;
    }
    let Some(ordinal) = publication.reserve_sound(sound) else {
        return true;
    };
    let index = target.resources[1].first as usize + ordinal as usize;
    // Ready and already-staged rows need no second config record. Pending with
    // no sequence is a new reservation or a row whose publication was rejected.
    if !publication.unpublished(index) {
        return true;
    }
    let Some(name) = qa_audio::native_sound_name(target.rules, bank, sound) else {
        publication.unsupported += 1;
        publication.published(index, None);
        return true;
    };
    if protocol
        .config_capacity(index)
        .is_none_or(|limit| name.as_bytes().len() > limit)
    {
        publication.unsupported += 1;
        publication.published(index, None);
        return true;
    }
    if events.needs_resync(output) {
        return false;
    }
    match events.publish_configstring(
        Some(OutputTarget::Client(client)),
        module,
        source,
        source_index,
        index as u32,
        name.as_bytes(),
    ) {
        Ok(sequence) => {
            publication.published(index, Some(sequence));
            true
        }
        Err(_) => false,
    }
}

impl super::Modules {
    pub(crate) fn model_name(
        &self,
        module: qa_core::primitives::ModuleId,
        ordinal: u32,
        destination: Option<qa_core::primitives::GeometryId>,
        usage: Option<qa_core::primitives::ModelUse>,
    ) -> Option<crate::client_resources::ModelName> {
        let row = self.row(module).filter(|row| row.state == State::Running)?;
        let revision = self.storage.config_revision(module).ok()?;
        let source = row.request.context.native_rules;
        use qa_gameplay::rules::appearance::{ModelInput, model_input};
        let ordinal = match model_input(source, ordinal, usage) {
            ModelInput::Player => {
                return Some(crate::client_resources::ModelName {
                    name: None,
                    revision,
                    source,
                });
            }
            ModelInput::Resource(ordinal) | ModelInput::LinkedTranslucent(ordinal) => ordinal,
        };
        let name = self.storage.bound_model_name(module, ordinal)?;
        let text = self.storage.resource_name(name)?;
        if text.starts_with(b"*") && (destination.is_none() || row.geometry != destination) {
            return None;
        }
        Some(crate::client_resources::ModelName {
            name: Some(name),
            revision,
            source,
        })
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Publication borrows source name identity, destination policy and the existing retained ring together"
)]
fn publish_model(
    publication: &mut crate::client_resources::ConfigPublication,
    events: &mut qa_core::events::EventRing,
    output: qa_core::events::OutputConsumerId,
    client: ClientId,
    module: qa_core::primitives::ModuleId,
    source: RuleSetId,
    source_index: u32,
    target: ConfigLayout,
    protocol: qa_network::commands::packet::Protocol,
    name: qa_core::primitives::NameId,
    text: &[u8],
) -> bool {
    let Some(ordinal) = publication.reserve_model(name) else {
        publication.unsupported += 1;
        return true;
    };
    let index = target.resources[0].first as usize + ordinal as usize;
    if !publication.unpublished(index) {
        return true;
    }
    if protocol
        .config_capacity(index)
        .is_none_or(|limit| text.len() > limit)
    {
        publication.unsupported += 1;
        publication.published(index, None);
        return true;
    }
    if events.needs_resync(output) {
        return false;
    }
    match events.publish_configstring(
        Some(OutputTarget::Client(client)),
        module,
        source,
        source_index,
        index as u32,
        text,
    ) {
        Ok(sequence) => {
            publication.published(index, Some(sequence));
            true
        }
        Err(_) => false,
    }
}
