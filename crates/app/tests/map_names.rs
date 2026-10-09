use qa_app::{Runtime, map::NativeEntityText};
use qa_formats::entities::{EntityLump, EntitySyntax};
use std::borrow::Cow;

fn source(bytes: &[u8], syntax: EntitySyntax) -> NativeEntityText {
    NativeEntityText {
        syntax,
        bytes: bytes.into(),
    }
}

#[test]
fn q1_identity_fields_trim_only_native_trailing_spaces_and_keep_exact_case() {
    let source = source(
        br#"{
            "classname  " "func_door"
            "Classname" "upper_key_is_guest"
            "targetname" "first_door"
            "targetname " "second_door"
            "targetname	" "tab_is_guest"
            "target" "linked_door"
            "killtarget" "removed_door"
            "model" "*17"
            "noise4" "doors/stop.wav"
            "map" "next_map"
            "sounds" "3"
            "unknown" "guest_value"
        }"#,
        EntitySyntax::Quake,
    );
    let names = source.catalog_names().unwrap();
    let bytes: Vec<_> = names.iter().map(|name| name.as_ref()).collect();
    assert_eq!(
        bytes,
        [
            b"func_door".as_slice(),
            b"first_door",
            b"second_door",
            b"linked_door",
            b"removed_door",
            b"*17",
            b"doors/stop.wav",
            b"next_map",
        ]
    );
    assert!(names.iter().all(|name| matches!(name, Cow::Borrowed(_))));
    let native = EntityLump::parse(&source.bytes, source.syntax).unwrap();
    assert_ne!(
        native.names.find(b"targetname"),
        native.names.find(b"targetname ")
    );
    assert!(native.names.find(b"Classname").is_some());
}

#[test]
fn level_strings_follow_native_escape_consumption_before_ids_bind() {
    for syntax in [
        EntitySyntax::Quake,
        EntitySyntax::Quake2,
        EntitySyntax::Quake3,
    ] {
        let source = source(
            br#"{
                "classname" "func_door"
                "targetname" "door\nrear"
                "target" "linked\xdoor"
                "model" "models\\door"
                "team" "ignored_in_q1"
                "targetname" "trailing\"
            }"#,
            syntax,
        );
        let names = source.catalog_names().unwrap();
        assert!(matches!(&names[0], Cow::Borrowed(_)));
        assert!(matches!(&names[1], Cow::Owned(_)));
        assert_eq!(names[1].as_ref(), b"door\nrear");
        assert_eq!(names[2].as_ref(), b"linked\\door");
        assert_eq!(names[3].as_ref(), b"models\\door");
        assert_eq!(names.last().unwrap().as_ref(), b"trailing\\");
        let runtime = Runtime::load(64, names.iter().map(|name| name.as_ref())).unwrap();
        assert!(runtime.catalog.names.find(b"door\nrear").is_some());
        assert!(runtime.catalog.names.find(b"door\\nrear").is_none());
        assert!(
            source
                .bytes
                .windows(10)
                .any(|value| value == b"door\\nrear")
        );
    }
}

#[test]
fn q2_folded_keys_collect_alternate_targets_and_temporary_identity_fields() {
    let source = source(
        br#"{
            "CLASSNAME" "target_speaker"
            "TargetName" "first"
            "TARGETNAME" "second"
            "PATHtarget" "path"
            "DEATHtarget" "death"
            "COMBATtarget" "combat"
            "KILLtarget" "kill"
            "NOISE" "world\nnoise.wav"
            "ITEM" "weapon_railgun"
            "SKY" "unit1_"
            "NEXTMAP" "base2"
            "MaP" "custom_q2_exit"
            "sounds" "2"
            "gravity" "800"
            "unknown" "guest"
        }"#,
        EntitySyntax::Quake2,
    );
    let names = source.catalog_names().unwrap();
    let bytes: Vec<_> = names.iter().map(|name| name.as_ref()).collect();
    assert_eq!(
        bytes,
        [
            b"target_speaker".as_slice(),
            b"first",
            b"second",
            b"path",
            b"death",
            b"combat",
            b"kill",
            b"world\nnoise.wav",
            b"weapon_railgun",
            b"unit1_",
            b"base2",
            b"custom_q2_exit",
        ]
    );
    let runtime = Runtime::load(64, names.iter().map(|name| name.as_ref())).unwrap();
    assert!(runtime.catalog.names.find(b"custom_q2_exit").is_some());
}

#[test]
fn q3_noise_is_raw_and_spawn_generated_suffixes_are_not_invented() {
    let source = source(
        br#"{
            "CLASSNAME" "target_speaker"
            "TARGETNAME" "speaker\nrear"
            "MODEL2" "models/lamp.md3"
            "targetShaderNAME" "textures/a"
            "targetShaderNEWname" "textures/b"
            "NoIsE" "world\nwind"
            "noise" "wind"
        }"#,
        EntitySyntax::Quake3,
    );
    let names = source.catalog_names().unwrap();
    assert_eq!(names[1].as_ref(), b"speaker\nrear");
    assert_eq!(names[5].as_ref(), b"world\\nwind");
    assert!(matches!(&names[5], Cow::Borrowed(_)));
    assert_eq!(names[6].as_ref(), b"wind");
    let runtime = Runtime::load(64, names.iter().map(|name| name.as_ref())).unwrap();
    assert!(runtime.catalog.names.find(b"wind").is_some());
    assert!(runtime.catalog.names.find(b"wind.wav").is_none());
}

#[test]
fn runtime_retains_exact_guest_source_after_cold_names_are_released() {
    let mut original = br#"{
        "classname" "worldspawn"
        "targetname" "Door"
        "targetname" "door"
        "UnknownGuest" "line\nraw"
        "UnknownGuest" "second"
        "unknownGuest" "third"
    }"#
    .to_vec();
    original.extend_from_slice(b"\0not tokenized \xff\x80");
    let source = source(&original, EntitySyntax::Quake);
    let names = source.catalog_names().unwrap();
    let mut runtime = Runtime::load(64, names.iter().map(|name| name.as_ref())).unwrap();
    drop(names);
    runtime.entity_sources.push(source);
    assert_eq!(runtime.entity_sources[0].bytes.as_ref(), original);
    assert_eq!(runtime.entity_sources[0].syntax, EntitySyntax::Quake);
    assert_ne!(
        runtime.catalog.names.find(b"Door"),
        runtime.catalog.names.find(b"door")
    );
    let retained = &runtime.entity_sources[0];
    let native = EntityLump::parse(&retained.bytes, retained.syntax).unwrap();
    assert_eq!(native.fields[3].value, b"line\\nraw");
    assert_eq!(native.fields[4].value, b"second");
    assert_eq!(native.fields[5].value, b"third");
    assert_ne!(
        native.names.find(b"UnknownGuest"),
        native.names.find(b"unknownGuest")
    );
}
