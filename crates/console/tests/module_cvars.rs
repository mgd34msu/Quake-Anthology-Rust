use qa_console::{
    cvars::{Cvars, RuntimePolicy, WriteError},
    views::{Context, Role, RuleSetId},
};

fn context(source: RuleSetId) -> Context {
    Context {
        source,
        role: Role::Game,
        ..Context::default()
    }
}

#[test]
fn native_info_uses_selected_scope_source_names_and_module_rows_once() {
    use qa_console::catalog::Scope;
    use qa_core::text::info_value;
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut cvars = Cvars::new().unwrap();
        let client = Context {
            source,
            ..Context::default()
        };
        let server = Context {
            side: Scope::Server,
            role: Role::Game,
            ..client
        };
        for (context, value, flags) in [(client, "client-secret", 2), (server, "server-secret", 4)]
        {
            let password = cvars.bind("password", context).unwrap();
            cvars.full_set(password, value, flags).unwrap();
        }
        cvars
            .register("_qa_info", Some("module-value"), 2, client)
            .unwrap();
        cvars.register("_qa_empty", Some(""), 2, client).unwrap();
        let mut info = [0; 2048];
        let rules = qa_core::text::InfoRules {
            bytes: info.len(),
            key_bytes: 64,
            value_bytes: 256,
            filter: qa_core::text::InfoBytes::Ascii7Printable,
        };
        let length = cvars.info_string(client, 2, rules, &mut info).unwrap();
        let bytes = &info[..length];
        let info = std::str::from_utf8(bytes).unwrap();
        let password = info_value(bytes, b"password", false).unwrap();
        assert_eq!(&bytes[password], b"client-secret");
        assert_eq!(info.matches("\\password\\").count(), 1);
        let module = info_value(bytes, b"_qa_info", false).unwrap();
        assert_eq!(&bytes[module], b"module-value");
        assert!(info_value(bytes, b"_qa_empty", false).is_none());
        assert!(info_value(bytes, b"skin", false).is_some());
        assert_eq!(
            cvars
                .info_string(client, 2, rules, &mut [0; 1])
                .unwrap_err(),
            WriteError::Capacity
        );
    }
}

#[test]
fn native_info_omits_fields_outside_the_supplied_budget_and_keeps_later_fields() {
    use qa_core::text::{InfoBytes, InfoRules, info_value};
    let mut cvars = Cvars::new().unwrap();
    let client = Context {
        source: RuleSetId::Quake2Rerelease,
        ..Default::default()
    };
    for name in ["_qa_first", "_qa_second", "_qa_third"] {
        cvars
            .register(name, Some(&"x".repeat(230)), 2, client)
            .unwrap();
    }
    cvars
        .register("_qa_field_limit", Some(&"y".repeat(256)), 2, client)
        .unwrap();
    cvars
        .register(&"k".repeat(64), Some("invalid-key"), 2, client)
        .unwrap();
    cvars
        .register("_qa_after", Some("kept"), 2, client)
        .unwrap();
    let rules = InfoRules {
        bytes: 512,
        key_bytes: 64,
        value_bytes: 256,
        filter: InfoBytes::Ascii7Printable,
    };
    let mut output = [0xa5; 600];
    let length = cvars.info_string(client, 2, rules, &mut output).unwrap();
    assert!(length < rules.bytes);
    assert_eq!(output[length], 0);
    assert_eq!(&output[rules.bytes..], &[0xa5; 88]);
    let info = &output[..length];
    assert!(info_value(info, b"_qa_first", false).is_some());
    assert!(info_value(info, b"_qa_third", false).is_none());
    assert!(info_value(info, b"_qa_field_limit", false).is_none());
    assert!(info_value(info, &vec![b'k'; 64], false).is_none());
    let after = info_value(info, b"_qa_after", false).unwrap();
    assert_eq!(&info[after], b"kept");
}

#[test]
fn native_info_keeps_binding_conversion_failures_scoped_to_the_caller() {
    let mut cvars = Cvars::new().unwrap();
    let client = Context {
        source: RuleSetId::Quake,
        ..Default::default()
    };
    let gamma = cvars.bind("gamma", client).unwrap();
    cvars.full_set(gamma, "1", 2).unwrap();
    cvars.set_text(gamma.canonical(), "0").unwrap();
    let mut output = [0; 512];
    assert!(matches!(
        cvars.info_string(
            client,
            2,
            qa_core::text::InfoRules {
                bytes: output.len(),
                key_bytes: 64,
                value_bytes: 256,
                filter: qa_core::text::InfoBytes::Exact,
            },
            &mut output
        ),
        Err(WriteError::Conversion(_))
    ));
}

#[test]
fn native_reset_uses_source_and_alias_defaults_without_resetting_other_sources() {
    for source in RuleSetId::ALL {
        for name in ["sensitivity", "gamma"] {
            let mut cvars = Cvars::new().unwrap();
            let context = context(source);
            let view = cvars.bind(name, context).unwrap();
            if !cvars.default_available(view.canonical(), source) {
                continue;
            }
            let original = cvars.read(view).unwrap().as_str().to_owned();
            cvars.force_write(view, "0.8").unwrap();
            cvars.reset_view(view, true).unwrap();
            assert_eq!(cvars.read(view).unwrap().as_str(), original);
            if name == "gamma" {
                cvars.set_text(view.canonical(), "0").unwrap();
                assert!(cvars.read(view).is_err());
                cvars.reset_view(view, true).unwrap();
                assert_eq!(cvars.read(view).unwrap().as_str(), original);
            }
        }
    }
    let mut cvars = Cvars::new().unwrap();
    let q3 = cvars
        .bind("sensitivity", context(RuleSetId::Quake3))
        .unwrap();
    let q1 = cvars
        .bind("sensitivity", context(RuleSetId::Quake))
        .unwrap();
    cvars.force_write(q3, "9").unwrap();
    cvars.reset_view(q3, true).unwrap();
    assert_eq!(cvars.read(q3).unwrap().as_str(), "5");
    assert_eq!(cvars.read(q1).unwrap().as_str(), "5");
    cvars.reset_view(q1, true).unwrap();
    assert_eq!(cvars.read(q3).unwrap().as_str(), "3");
    assert_eq!(cvars.read(q1).unwrap().as_str(), "3");
}

#[test]
fn native_reset_preserves_protection_latches_and_equal_value_short_circuit() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::Quake3);
    let readonly = cvars
        .register("_qa_reset_rom", Some("1"), 64, context)
        .unwrap();
    cvars.force_write(readonly, "2").unwrap();
    assert_eq!(cvars.reset_view(readonly, false), Err(WriteError::ReadOnly));
    assert_eq!(cvars.read(readonly).unwrap().as_str(), "2");
    cvars.reset_view(readonly, true).unwrap();
    assert_eq!(cvars.read(readonly).unwrap().as_str(), "1");
    let latched = cvars
        .register("_qa_reset_latch", Some("1"), 32, context)
        .unwrap();
    cvars.set_latch_active(latched.canonical(), true);
    cvars.write(latched, "2").unwrap();
    let generation = cvars.view_update_generation(latched);
    cvars.reset_view(latched, true).unwrap();
    assert_eq!(cvars.view_update_generation(latched), generation);
    assert_eq!(cvars.latched(latched).unwrap().unwrap().as_str(), "2");
    cvars.apply_latches().unwrap();
    cvars.reset_view(latched, false).unwrap();
    assert_eq!(cvars.read(latched).unwrap().as_str(), "2");
    assert_eq!(cvars.latched(latched).unwrap().unwrap().as_str(), "1");
    cvars.reset_view(latched, true).unwrap();
    assert_eq!(cvars.read(latched).unwrap().as_str(), "1");
    assert!(cvars.latched(latched).unwrap().is_none());
}

#[test]
fn native_registration_fills_an_unresolved_source_default_without_overwriting_values() {
    let mut cvars = Cvars::new().unwrap();
    let rr = context(RuleSetId::Quake2Rerelease);
    let classic = context(RuleSetId::Quake2);
    let unresolved = cvars.bind("maxentities", rr).unwrap();
    assert!(!cvars.default_available(unresolved.canonical(), rr.source));
    let query = cvars.register("maxentities", None, 32, rr).unwrap();
    assert_eq!(query.canonical(), unresolved.canonical());
    assert!(!cvars.default_available(query.canonical(), rr.source));
    assert!(matches!(
        cvars.register("_qa_missing_default", None, 0, rr),
        Err(WriteError::MissingDefault)
    ));
    let old_count = cvars.entries().count();
    let view = cvars.register("maxentities", Some("8192"), 32, rr).unwrap();
    assert_eq!(view.canonical(), unresolved.canonical());
    assert_eq!(cvars.entries().count(), old_count);
    assert_eq!(cvars.read(view).unwrap().as_str(), "8192");
    assert_eq!(cvars.numeric(view).unwrap(), 8192.0);
    assert!(cvars.native_default_available(view.canonical(), rr.source));
    let other = cvars.bind("maxentities", classic).unwrap();
    assert_eq!(cvars.read(other).unwrap().as_str(), "1024");
    let generation = cvars.view_generation(view);
    cvars.register("MAXENTITIES", Some("4096"), 32, rr).unwrap();
    assert_eq!(cvars.view_generation(view), generation);
    assert_eq!(cvars.read(view).unwrap().as_str(), "8192");
    cvars.force_write(view, "2048").unwrap();
    cvars.reset(view.canonical());
    assert_eq!(cvars.read(view).unwrap().as_str(), "8192");
    let mut cvars = Cvars::new().unwrap();
    let user = cvars.bind("maxentities", rr).unwrap();
    cvars.force_write(user, "4096").unwrap();
    cvars.register("maxentities", Some("8192"), 32, rr).unwrap();
    assert_eq!(cvars.read(user).unwrap().as_str(), "4096");
    cvars.reset(user.canonical());
    assert_eq!(cvars.read(user).unwrap().as_str(), "8192");
    let empty = cvars
        .register("_qa_explicit_empty", Some(""), 0, rr)
        .unwrap();
    assert!(cvars.default_available(empty.canonical(), rr.source));
    assert_eq!(cvars.read(empty).unwrap().as_str(), "");
}

#[test]
fn module_names_share_handles_and_listing_in_every_source() {
    let mut cvars = Cvars::new().unwrap();
    let count = cvars.entries().count();
    let first = cvars
        .register(
            "_QA_ModuleSpeed",
            Some("12.5"),
            0,
            context(RuleSetId::Quake2Rerelease),
        )
        .unwrap();
    for source in RuleSetId::ALL {
        let view = cvars.bind("_qa_modulespeed", context(source)).unwrap();
        assert_eq!(view.canonical(), first.canonical());
        assert_eq!(cvars.read(view).unwrap().as_str(), "12.5");
        assert_eq!(cvars.numeric(view).unwrap().to_bits(), 12.5f32.to_bits());
        assert_eq!(cvars.integer_view(view).unwrap(), 12);
        assert!(cvars.native_default_available(view.canonical(), source));
    }
    cvars.write(first, "27.25").unwrap();
    for source in RuleSetId::ALL {
        assert_eq!(
            cvars.value_in(first.canonical(), source).to_bits(),
            27.25f32.to_bits()
        );
        assert_eq!(cvars.integer_in(first.canonical(), source), 27);
    }
    let rows = cvars
        .entries()
        .filter(|(handle, _, _, _, _)| *handle == first.canonical())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "_QA_ModuleSpeed");
    assert_eq!(rows[0].2, "27.25");
    assert!(rows[0].3.is_none());
    assert_eq!(cvars.entries().count(), count + 1);
}

#[test]
fn registration_preserves_catalog_aliases_defaults_and_written_values() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::Quake2);
    let before = cvars.bind("fov", context).unwrap();
    let initial = cvars.read(before).unwrap().as_str().to_owned();
    let view = cvars.register("FOV", Some("13"), 0, context).unwrap();
    assert_eq!(view.canonical(), before.canonical());
    assert_eq!(cvars.read(view).unwrap().as_str(), initial);
    cvars.write(view, "117").unwrap();
    let generation = cvars.view_generation(view);
    let after = cvars.register("fov", Some("14"), 0, context).unwrap();
    assert_eq!(cvars.read(after).unwrap().as_str(), "117");
    assert_eq!(cvars.view_generation(after), generation);
    let count = cvars.entries().count();
    assert_eq!(
        cvars
            .register("cg_fov", Some("90"), 0, context)
            .unwrap()
            .canonical(),
        view.canonical()
    );
    assert_eq!(cvars.entries().count(), count);
}

#[test]
fn q3_native_registration_adopts_user_reset_default_without_replacing_current_value() {
    let mut cvars = Cvars::new().unwrap();
    let q3 = context(RuleSetId::Quake3);
    let user = cvars
        .register("_QA_UserCreated", Some("42"), 128 | 1, q3)
        .unwrap();
    cvars.write(user, "9").unwrap();
    let count = cvars.entries().count();
    let native = cvars
        .register("_qa_usercreated", Some("3"), 32, q3)
        .unwrap();
    assert_eq!(native.canonical(), user.canonical());
    assert_eq!(cvars.entries().count(), count);
    assert_eq!(cvars.read(native).unwrap().as_str(), "9");
    assert_eq!(cvars.numeric(native).unwrap(), 9.0);
    assert_eq!(cvars.flags(native), 1 | 32);
    let q2 = cvars
        .bind("_qa_usercreated", context(RuleSetId::Quake2))
        .unwrap();
    assert_eq!(cvars.flags(q2), 128 | 1);
    cvars.reset(native.canonical());
    assert_eq!(cvars.read(native).unwrap().as_str(), "3");
    assert_eq!(cvars.read(q2).unwrap().as_str(), "42");
    cvars.register("_qa_usercreated", Some("4"), 0, q3).unwrap();
    assert_eq!(cvars.read(native).unwrap().as_str(), "3");

    // Cvar_Get can also produce a user-created row directly. Its initial
    // current string must survive adoption even before an explicit write.
    let direct = cvars
        .register("_qa_direct_user", Some("8"), 128, q3)
        .unwrap();
    cvars.register("_qa_direct_user", Some("2"), 0, q3).unwrap();
    assert_eq!(cvars.read(direct).unwrap().as_str(), "8");
    assert_eq!(cvars.flags(direct), 0);
    cvars.reset(direct.canonical());
    assert_eq!(cvars.read(direct).unwrap().as_str(), "2");
}

#[test]
fn user_set_and_module_registration_share_one_row_and_native_ownership() {
    for source in RuleSetId::ALL {
        let mut cvars = Cvars::new().unwrap();
        let engine = Context {
            source,
            ..Context::default()
        };
        let count = cvars.entries().count();
        let user = cvars
            .set_user("_QA_UserSetting", "27", engine, None, 1)
            .unwrap();
        let replacement = cvars
            .set_user("_qa_usersetting", "31", engine, None, 0)
            .unwrap();
        assert_eq!(replacement.canonical(), user.canonical());
        assert_eq!(cvars.entries().count(), count + 1);
        assert_eq!(cvars.numeric(replacement).unwrap(), 31.0);
        assert_eq!(cvars.integer_view(replacement).unwrap(), 31);
        assert_eq!(
            cvars.flags(replacement),
            1 | if source == RuleSetId::Quake3 { 128 } else { 0 }
        );
        let native = cvars
            .register("_qa_usersetting", Some("3"), 0, context(source))
            .unwrap();
        assert_eq!(cvars.read(native).unwrap().as_str(), "31");
        cvars.reset_view(native, true).unwrap();
        assert_eq!(
            cvars.read(native).unwrap().as_str(),
            if source == RuleSetId::Quake3 {
                "3"
            } else {
                "27"
            }
        );
        let alias = cvars.set_user("FOV", "107", engine, None, 0).unwrap();
        assert_eq!(alias.canonical(), cvars.find("cg_fov").unwrap());
        assert_eq!(cvars.entries().count(), count + 1);
        assert_eq!(cvars.read(alias).unwrap().as_str(), "107");
    }
}

#[test]
fn q3_user_setting_checks_native_name_before_binding_another_source_row() {
    let mut cvars = Cvars::new().unwrap();
    let q2 = Context {
        source: RuleSetId::Quake2,
        ..Context::default()
    };
    let foreign = cvars.register("bad;name", Some("original"), 0, q2).unwrap();
    let q3 = Context::default();
    let user = cvars
        .set_user("bad;name", "replacement", q3, None, 0)
        .unwrap();
    assert_ne!(user.canonical(), foreign.canonical());
    assert_eq!(
        user.canonical(),
        cvars.bind("BADNAME", q3).unwrap().canonical()
    );
    assert_eq!(cvars.read(user).unwrap().as_str(), "replacement");
    assert_eq!(cvars.read(foreign).unwrap().as_str(), "original");
}

#[test]
fn user_default_adoption_requires_q3_nonempty_non_user_registration() {
    let mut cvars = Cvars::new().unwrap();
    let q3 = context(RuleSetId::Quake3);
    let user = cvars
        .register("_qa_still_user", Some("7"), 128, q3)
        .unwrap();
    for (default, flags) in [(None, 0), (Some(""), 0), (Some("3"), 128)] {
        cvars
            .register("_qa_still_user", default, flags, q3)
            .unwrap();
        assert_eq!(cvars.flags(user), 128);
        cvars.reset(user.canonical());
        assert_eq!(cvars.read(user).unwrap().as_str(), "7");
    }
    let q2 = context(RuleSetId::Quake2);
    let other = cvars
        .register("_qa_other_source", Some("6"), 128, q2)
        .unwrap();
    cvars
        .register("_qa_other_source", Some("3"), 0, q2)
        .unwrap();
    assert_eq!(cvars.flags(other), 128);
    cvars.reset(other.canonical());
    assert_eq!(cvars.read(other).unwrap().as_str(), "6");
    let module = cvars
        .register("_qa_native_default", Some("5"), 0, q3)
        .unwrap();
    cvars
        .register("_qa_native_default", Some("3"), 0, q3)
        .unwrap();
    cvars.reset(module.canonical());
    assert_eq!(cvars.read(module).unwrap().as_str(), "5");
    let catalog = cvars.bind("fov", q3).unwrap();
    let original = cvars.read(catalog).unwrap().as_str().to_owned();
    cvars.register("fov", Some("3"), 0, q3).unwrap();
    cvars.reset(catalog.canonical());
    assert_eq!(cvars.read(catalog).unwrap().as_str(), original);
}

#[test]
fn module_values_use_the_existing_latch_flag_and_reset_paths() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::Quake3);
    let view = cvars
        .register("_qa_latched", Some("10"), 32, context)
        .unwrap();
    let generation = cvars.view_generation(view);
    cvars.set_runtime_policy(RuntimePolicy {
        server_active: true,
        ..cvars.runtime_policy()
    });
    cvars.write(view, "20").unwrap();
    assert_eq!(cvars.read(view).unwrap().as_str(), "10");
    assert_eq!(cvars.view_generation(view), generation);
    cvars.apply_latches().unwrap();
    assert_eq!(cvars.read(view).unwrap().as_str(), "20");
    assert!(cvars.view_generation(view) > generation);
    cvars.reset(view.canonical());
    assert_eq!(cvars.read(view).unwrap().as_str(), "10");
    cvars.full_set(view, "30", 64).unwrap();
    assert_eq!(cvars.write(view, "40"), Err(WriteError::ReadOnly));
    let generation = cvars.view_generation(view);
    let merged = cvars
        .register("_QA_LATCHED", Some("99"), 1, context)
        .unwrap();
    assert_eq!(cvars.flags(merged), 65);
    assert_eq!(cvars.read(merged).unwrap().as_str(), "30");
    assert!(cvars.view_generation(merged) > generation);
    let generation = cvars.view_generation(merged);
    cvars
        .register("_qa_latched", Some("88"), 1, context)
        .unwrap();
    assert_eq!(cvars.view_generation(merged), generation);
    cvars.full_set(view, "50", 0).unwrap();
    cvars.write(view, "60").unwrap();
    assert_eq!(cvars.read(view).unwrap().as_str(), "60");
}

#[test]
fn bad_info_and_capacity_failures_keep_live_rows_and_existing_bindings() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::QuakeWorld);
    let initial = cvars.entries().count();
    for (name, default) in [("", "1"), ("bad;name", "1"), ("_qa_bad", "bad\\info")] {
        assert!(cvars.register(name, Some(default), 6, context).is_err());
        assert_eq!(cvars.entries().count(), initial);
    }
    let first = cvars
        .register("_qa_module_0", Some("7"), 0, context)
        .unwrap();
    for index in 1..1024 {
        cvars
            .register(&format!("_qa_module_{index}"), Some("0"), 0, context)
            .unwrap();
    }
    assert!(matches!(
        cvars.register("_qa_capacity", Some("0"), 0, context),
        Err(WriteError::Capacity)
    ));
    assert_eq!(cvars.entries().count(), initial + 1024);
    assert!(cvars.bind("_qa_capacity", context).is_none());
    assert_eq!(
        cvars
            .register("_QA_MODULE_0", Some("9"), 0, context)
            .unwrap()
            .canonical(),
        first.canonical()
    );
    assert_eq!(cvars.read(first).unwrap().as_str(), "7");
}
#[test]
fn native_views_count_each_change_and_keep_alias_contexts_and_latches() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::Quake3);
    let view = cvars
        .register("_qa_vm_count", Some("1"), 32, context)
        .unwrap();
    cvars.force_write(view, "2").unwrap();
    cvars.force_write(view, "3").unwrap();
    let handle = cvars.native_view(view).unwrap();
    assert_eq!(cvars.read(view).unwrap().as_str(), "3");
    assert_eq!(cvars.native_snapshot(handle, view.context()).unwrap().1, 1);
    assert_eq!(cvars.native_view(view).unwrap(), handle);
    cvars.set_latch_active(view.canonical(), true);
    cvars.write(view, "4").unwrap();
    assert_eq!(cvars.native_snapshot(handle, view.context()).unwrap().1, 2);
    cvars.write(view, "4").unwrap();
    assert_eq!(cvars.native_snapshot(handle, view.context()).unwrap().1, 2);
    cvars.write(view, "5").unwrap();
    assert_eq!(cvars.native_snapshot(handle, view.context()).unwrap().1, 3);
    cvars.apply_latches().unwrap();
    assert_eq!(cvars.native_snapshot(handle, view.context()).unwrap().1, 4);
    assert_eq!(cvars.read(view).unwrap().as_str(), "5");
    cvars.force_write(view, "6").unwrap();
    cvars.force_write(view, "7").unwrap();
    assert_eq!(cvars.native_snapshot(handle, view.context()).unwrap().1, 6);
    cvars
        .register("_qa_vm_count", Some("ignored"), 1, context)
        .unwrap();
    assert_eq!(cvars.native_snapshot(handle, view.context()).unwrap().1, 6);
    let other = cvars
        .register("_qa_vm_other", Some("9"), 0, context)
        .unwrap();
    cvars.force_write(other, "10").unwrap();
    assert_eq!(cvars.native_snapshot(handle, view.context()).unwrap().1, 6);
    assert!(cvars.native_snapshot(u32::MAX, context).is_none());

    let view = cvars.bind("cg_fov", context).unwrap();
    let alias = cvars.bind("fov", context).unwrap();
    let first = cvars.native_view(view).unwrap();
    let second = cvars.native_view(alias).unwrap();
    let before = cvars.native_snapshot(first, view.context()).unwrap().1;
    let alias_before = cvars.native_snapshot(second, alias.context()).unwrap().1;
    cvars.force_write(alias, "103").unwrap();
    assert_eq!(
        cvars.native_snapshot(first, view.context()).unwrap().1,
        before + 1
    );
    assert_eq!(
        cvars.native_snapshot(second, alias.context()).unwrap().1,
        alias_before + 1
    );
}

#[test]
fn q3_registration_maps_only_native_delimiters_to_badname_before_lookup() {
    let mut cvars = Cvars::new().unwrap();
    let q2 = context(RuleSetId::Quake2);
    let q3 = context(RuleSetId::Quake3);
    let foreign = cvars.register("bad;name", Some("q2"), 0, q2).unwrap();
    let bad = cvars.register("bad;name", Some("first"), 0, q3).unwrap();
    assert_ne!(bad.canonical(), foreign.canonical());
    for name in ["bad\\name", "bad\"name", "bad;name"] {
        let view = cvars.register(name, Some("ignored"), 0, q3).unwrap();
        assert_eq!(view.canonical(), bad.canonical());
        assert_eq!(cvars.read(view).unwrap().as_str(), "first");
    }
    assert_eq!(cvars.read(foreign).unwrap().as_str(), "q2");
    assert!(cvars.bind("bad\\name", q3).is_none());
    assert_eq!(
        cvars.bind("BADNAME", q3).unwrap().canonical(),
        bad.canonical()
    );
    // Cvar_ValidateString does not impose info-string control-byte rules.
    let control = cvars.register("raw\u{1}name", Some("raw"), 0, q3).unwrap();
    assert_ne!(control.canonical(), bad.canonical());
}

#[test]
fn file_selection_metadata_does_not_split_cvar_policy_or_native_handles() {
    use qa_core::primitives::FileSearchId;
    let mut cvars = Cvars::new().unwrap();
    let first = Context {
        file_search: FileSearchId::new(1),
        ..Context::default()
    };
    let second = Context {
        file_search: FileSearchId::new(2),
        ..first
    };
    cvars.select_context(first);
    assert_eq!(cvars.context().file_search, None);
    let one = cvars
        .native_view(cvars.bind("s_volume", first).unwrap())
        .unwrap();
    cvars.select_context(second);
    let two = cvars
        .native_view(cvars.bind("s_volume", second).unwrap())
        .unwrap();
    assert_eq!(one, two);
    assert_eq!(cvars.context().file_search, None);
    assert_eq!(
        Cvars::with_context(first).unwrap().context().file_search,
        None
    );
}

#[test]
fn native_info_generations_ignore_telemetry_and_pending_latches_but_track_membership() {
    let mut cvars = Cvars::new().unwrap();
    let q3 = context(RuleSetId::Quake3);
    let telemetry = cvars
        .register("_qa_info_telemetry", Some("0"), 0, q3)
        .unwrap();
    let user = cvars
        .register("_qa_live_user", Some("before"), 2, q3)
        .unwrap();
    let server = cvars
        .register("_qa_live_server", Some("before"), 4, q3)
        .unwrap();
    let system = cvars
        .register("_qa_live_system", Some("before"), 8 | 32, q3)
        .unwrap();
    let generations = [2, 4, 8].map(|flag| cvars.info_generation(q3.source, flag));
    cvars.force_write(telemetry, "1").unwrap();
    assert_eq!(
        [2, 4, 8].map(|flag| cvars.info_generation(q3.source, flag)),
        generations
    );
    cvars.set_latch_active(system.canonical(), true);
    cvars.write(system, "after").unwrap();
    assert_eq!(
        [2, 4, 8].map(|flag| cvars.info_generation(q3.source, flag)),
        generations
    );
    cvars.apply_latch(system).unwrap();
    assert!(cvars.info_generation(q3.source, 8) > generations[2]);
    assert_eq!(cvars.info_generation(q3.source, 2), generations[0]);
    assert_eq!(cvars.info_generation(q3.source, 4), generations[1]);
    cvars.force_write(server, "after").unwrap();
    assert!(cvars.info_generation(q3.source, 4) > generations[1]);
    let user_before = cvars.info_generation(q3.source, 2);
    // FullSet removes the old info key even when its value is unchanged.
    cvars.full_set(user, "before", 0).unwrap();
    assert!(cvars.info_generation(q3.source, 2) > user_before);
    let user_removed = cvars.info_generation(q3.source, 2);
    cvars.force_write(user, "unpublished").unwrap();
    assert_eq!(cvars.info_generation(q3.source, 2), user_removed);
    cvars.register("_qa_live_user", None, 2, q3).unwrap();
    assert!(cvars.info_generation(q3.source, 2) > user_removed);
    cvars.reset_lookup_count();
    for _ in 0..10 {
        assert_eq!(
            cvars.info_generation(q3.source, 6),
            cvars
                .info_generation(q3.source, 2)
                .max(cvars.info_generation(q3.source, 4))
        );
    }
    assert_eq!(cvars.lookup_count(), 0);
}

#[test]
fn native_info_generations_follow_alias_operands_server_scope_and_other_seats() {
    use qa_core::sys_events::SeatId;
    let mut cvars = Cvars::new().unwrap();
    let q1 = context(RuleSetId::Quake);
    let color = cvars.bind("_cl_color", q1).unwrap();
    cvars.full_set(color, "18", 2).unwrap();
    let before = cvars.info_generation(q1.source, 2);
    let secondary = cvars.bind("color2", q1).unwrap();
    cvars.force_write(secondary, "1").unwrap();
    assert_ne!(cvars.read(color).unwrap().as_str(), "18");
    assert!(cvars.info_generation(q1.source, 2) > before);
    for source in RuleSetId::ALL {
        let server = Context {
            side: qa_console::catalog::Scope::Server,
            source,
            ..Context::default()
        };
        if source != RuleSetId::Quake {
            let password = cvars.bind("password", server).unwrap();
            cvars.full_set(password, "old", 4).unwrap();
            let before = cvars.info_generation(source, 4);
            cvars.force_write(password, "new").unwrap();
            assert!(cvars.info_generation(source, 4) > before, "{source:?}");
        }
        let seat = Context {
            source,
            seat: SeatId::new(1).unwrap(),
            ..Context::default()
        };
        let name = cvars
            .bind(
                if source == RuleSetId::Quake {
                    "_cl_name"
                } else {
                    "name"
                },
                seat,
            )
            .unwrap();
        cvars.full_set(name, "seat two", 2).unwrap();
        let before = cvars.info_generation(source, 2);
        cvars.force_write(name, "renamed seat two").unwrap();
        assert!(cvars.info_generation(source, 2) > before, "{source:?}");
    }
}
