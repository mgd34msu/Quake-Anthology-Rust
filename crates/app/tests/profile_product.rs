//! Retail BSP syntax must not select a client's settings product.
use qa_app::{client_policy::ClientPolicy, map};
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_formats::archive::ArchiveReader;
use std::path::PathBuf;

struct FixtureDirectory(PathBuf);
impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires QA_RETAIL_ROOT retail content"]
fn settings_follow_the_actual_product_with_foreign_bsp_syntax() -> Result<(), String> {
    let retail = PathBuf::from(std::env::var_os("QA_RETAIL_ROOT").ok_or("retail root")?);
    let fixture = FixtureDirectory(
        std::env::temp_dir().join(format!("qa-client-profile-{}", std::process::id())),
    );
    std::fs::create_dir(&fixture.0).map_err(|e| e.to_string())?;
    for (source, name, destination, client, reader, expected) in [
        (
            "q1/id1",
            "e1m1",
            "q2/rerelease/baseq2",
            RuleSetId::Quake2Rerelease,
            RuleSetId::Quake,
            "q2/rerelease/baseq2",
        ),
        (
            "q2/baseq2",
            "base1",
            "q1/rerelease/id1",
            RuleSetId::Quake,
            RuleSetId::Quake2,
            "q1/rerelease/id1",
        ),
        (
            "q3a/baseq3",
            "q3dm1",
            "q2/baseq2",
            RuleSetId::Quake2,
            RuleSetId::Quake3,
            "q2/baseq2",
        ),
    ] {
        let mut source_vfs = Vfs::default();
        source_vfs
            .mount_product(&retail.join(source), 0)
            .map_err(|e| format!("{e:?}"))?;
        let file = source_vfs
            .open(format!("maps/{name}.bsp").as_bytes())
            .ok_or("retail map")?;
        let mut bytes = vec![
            0;
            usize::try_from(source_vfs.length(file).map_err(|e| format!("{e:?}"))?)
                .map_err(|e| e.to_string())?
        ];
        let read = source_vfs
            .read_into_reusing(file, &mut bytes, &mut ArchiveReader::default())
            .map_err(|e| format!("{e:?}"))?;
        assert_eq!(read, bytes.len());
        let root = fixture.0.join(destination);
        std::fs::create_dir_all(root.join("maps")).map_err(|e| e.to_string())?;
        std::fs::write(root.join("maps/foreign.bsp"), &bytes).map_err(|e| e.to_string())?;
        let mut vfs = Vfs::default();
        vfs.mount_directory(&root, 0)
            .map_err(|e| format!("{e:?}"))?;
        let input = map::read(&vfs, "foreign")?;
        assert_eq!(input.native_source, reader);
        assert_eq!(input.client_rules, Some(client));
        // A different client module must retain this recognized product key.
        assert_eq!(input.profile_product(RuleSetId::Quake3), expected);

        let unknown = root.join("custom");
        std::fs::create_dir_all(unknown.join("maps")).map_err(|e| e.to_string())?;
        std::fs::write(unknown.join("maps/foreign.bsp"), &bytes).map_err(|e| e.to_string())?;
        let mut vfs = Vfs::default();
        vfs.mount_directory(&unknown, 0)
            .map_err(|e| format!("{e:?}"))?;
        let input = map::read(&vfs, "foreign")?;
        assert_eq!(input.client_rules, None);
        assert!(ClientPolicy::select(None, input.client_rules, None, None).is_err());
        let policy = ClientPolicy::select(Some(RuleSetId::Quake), input.client_rules, None, None)?;
        assert_eq!(input.profile_product(policy.client), "q1/custom");
        assert_eq!(
            input.profile_product(RuleSetId::Quake2Rerelease),
            "q2/rerelease/custom"
        );
        println!(
            "{source}/{name}: reader={}, client={}, settings={expected}",
            reader.name(),
            client.name()
        );
    }
    Ok(())
}
