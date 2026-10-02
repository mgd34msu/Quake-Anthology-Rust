//! AAS authoring: cluster, optimize, and reachability rebuilds.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/tools/navigation/aas.ts`
//! (`authorAas`, the only export; `regenerate` is donor-private). The
//! cluster/optimize round trip is a complete port: argument parsing, the
//! distinct-output guard, parse, transform, write, reparse verification,
//! and exclusive file creation (`wx`, donor `writeFile` flag).
//!
//! The `--reachability` rebuild needs the application simulation stack to
//! resolve its inputs: content loading (`loadApplicationContent`), the
//! application simulation (`createSimulation`, `admitPlayer`,
//! `movementPlayer`), the bot navigation profile (`botNavigationProfile`),
//! and bot movement prediction (`predictApplicationBotMovement`). Those
//! siblings are owned by the application-simulation lane, so this module
//! ports the rebuild in two injectable pieces instead of calling them:
//! [`regenerate_aas`] ports the `regenerate` core that is `aas.ts` logic
//! (the BSP-checksum gate, forced [`build_aas_reachability`], then
//! [`cluster_aas`]) over caller-resolved simulation inputs, and
//! [`author_aas_with`] runs the donor command line with a caller-supplied
//! rebuild. [`author_aas`] is the exact donor entry point; its
//! `--reachability` path reports [`AasError::ReachabilityUnavailable`]
//! until the owning lane wires a rebuild through [`author_aas_with`].

use std::path::{Component, Path, PathBuf};

use qa_bots::md4::block_checksum;
use qa_bots::scene::{DecodedWorld, SceneQueries};
use qa_bots::{
    build_aas_reachability, cluster_aas, optimize_aas, parse_aas, write_aas, AasAsset, AasReachabilityOptions,
    BotMovementPrediction, BotTravelPredictionResult, BotsError, NavigationProfile,
};
use thiserror::Error;

/// Usage line, preserved from the donor.
pub const USAGE: &str = "Usage: aas input.aas output.aas [--cluster] [--optimize] [--reachability --game PRODUCT --map NAME --movement q1|q2|q3 [--content-root PATH]]";

/// Sibling surface the `--reachability` rebuild waits on.
pub const REACHABILITY_SIBLINGS: &str = "application content loading (loadApplicationContent), the application simulation (createSimulation/admitPlayer/movementPlayer), the bot navigation profile (botNavigationProfile), and bot movement prediction (predictApplicationBotMovement)";

/// Failure of AAS authoring.
#[derive(Debug, Error)]
pub enum AasError {
    /// Argument-shape failure; the message is [`USAGE`].
    #[error("{0}")]
    Usage(String),
    /// Input and output resolve to the same file.
    #[error("AAS authoring requires a distinct output file")]
    DistinctOutput,
    /// Map bytes do not match the asset's BSP checksum.
    #[error("Input AAS belongs to a different BSP checksum")]
    ChecksumMismatch,
    /// `--reachability` without a wired application simulation stack.
    #[error(
        "AAS reachability rebuild needs the application simulation stack ({REACHABILITY_SIBLINGS}); wire a rebuild through author_aas_with"
    )]
    ReachabilityUnavailable,
    /// Filesystem failure (reads and the exclusive output write).
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// AAS parse, transform, checksum, or serialize failure.
    #[error(transparent)]
    Bots(#[from] BotsError),
}

impl AasError {
    /// Build the usage failure.
    #[must_use]
    pub fn usage() -> Self {
        Self::Usage(USAGE.to_string())
    }
}

/// Parsed `--reachability` options (donor `options` map entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachabilityRequest {
    /// Donor `--game` value.
    pub game: String,
    /// Donor `--map` value.
    pub map: String,
    /// Donor `--movement` value (the donor does not validate it; the
    /// application options layer does).
    pub movement: String,
    /// Donor `--content-root` value, if given.
    pub content_root: Option<String>,
    /// Donor `[...options].flatMap(([key, value]) => [key, value])`: the
    /// option pairs in command-line encounter order.
    forwarded: Vec<String>,
}

impl ReachabilityRequest {
    /// Option pairs forwarded to the application command, in donor order.
    #[must_use]
    pub fn forwarded_args(&self) -> &[String] {
        &self.forwarded
    }
}

/// Resolved simulation inputs for [`regenerate_aas`]: the values the
/// donor derives from its application simulation (map bytes, geometry,
/// scene, profile, prediction client, movement predictor).
pub struct AasRegeneration<'a> {
    /// Raw map bytes the asset checksum is verified against.
    pub map_bytes: &'a [u8],
    /// Decoded map geometry (donor `content.world`).
    pub geometry: &'a DecodedWorld,
    /// Shared collision queries (donor `simulation.scene`).
    pub scene: &'a dyn SceneQueries,
    /// Traversal profile (donor `botNavigationProfile(player)`).
    pub profile: &'a NavigationProfile,
    /// Client slot the predictor simulates as (donor `client.id.slot`).
    pub prediction_client: i32,
    /// Client-movement predictor (donor `predictApplicationBotMovement`).
    pub predict_client_movement: &'a dyn Fn(BotMovementPrediction) -> BotTravelPredictionResult,
}

/// Rebuild reachability, then clusters (donor `regenerate` core): reject
/// assets built against other map bytes, force a [`build_aas_reachability`]
/// rebuild, and cluster the result.
pub fn regenerate_aas(asset: &AasAsset, inputs: &AasRegeneration<'_>) -> Result<AasAsset, AasError> {
    if asset.bsp_checksum != block_checksum(inputs.map_bytes)? as i32 {
        return Err(AasError::ChecksumMismatch);
    }
    let built = build_aas_reachability(&AasReachabilityOptions {
        asset,
        geometry: inputs.geometry,
        scene: inputs.scene,
        profile: inputs.profile,
        prediction_client: inputs.prediction_client,
        predict_client_movement: inputs.predict_client_movement,
        force: true,
        debug: false,
        settings: None,
        variable: None,
        print: None,
        debug_line: None,
    })?;
    Ok(cluster_aas(&built, None)?)
}

/// Reachability rebuild supplied by the application-simulation owner:
/// the donor `regenerate` body with its simulation wiring resolved.
pub type AasRegenerator<'a> = dyn Fn(&AasAsset, &ReachabilityRequest) -> Result<AasAsset, AasError> + 'a;

/// Parsed command line (donor destructuring plus the flags/options maps).
struct AuthorPlan {
    input: String,
    output: String,
    cluster: bool,
    optimize: bool,
    reachability: Option<ReachabilityRequest>,
}

/// Parse the donor command line over positional arguments (without the
/// program name): `[input, output, ...operations]`.
fn parse_author_args(argv: &[String]) -> Result<AuthorPlan, AasError> {
    let [input, output, operations @ ..] = argv else {
        return Err(AasError::usage());
    };
    let mut cluster = false;
    let mut optimize = false;
    let mut reachability = false;
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut index = 0;
    while index < operations.len() {
        let operation = operations[index].as_str();
        if operation == "--cluster" || operation == "--optimize" || operation == "--reachability" {
            match operation {
                "--cluster" => cluster = true,
                "--optimize" => optimize = true,
                _ => reachability = true,
            }
            index += 1;
        } else if operation == "--game"
            || operation == "--map"
            || operation == "--movement"
            || operation == "--content-root"
        {
            let value = operations.get(index + 1).ok_or_else(AasError::usage)?;
            if value.starts_with("--") || pairs.iter().any(|(key, _)| key == operation) {
                return Err(AasError::usage());
            }
            pairs.push((operation.to_string(), value.clone()));
            index += 2;
        } else {
            return Err(AasError::usage());
        }
    }
    let reachability = if reachability {
        let get = |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        let (Some(game), Some(map), Some(movement)) = (get("--game"), get("--map"), get("--movement")) else {
            return Err(AasError::usage());
        };
        Some(ReachabilityRequest {
            game,
            map,
            movement,
            content_root: get("--content-root"),
            forwarded: pairs.into_iter().flat_map(|(key, value)| [key, value]).collect(),
        })
    } else if !pairs.is_empty() {
        return Err(AasError::usage());
    } else {
        None
    };
    Ok(AuthorPlan {
        input: input.clone(),
        output: output.clone(),
        cluster,
        optimize,
        reachability,
    })
}

/// Join a relative path onto the working directory and collapse `.`/`..`
/// lexically, like the donor `resolve` comparison.
fn lexical_absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Write bytes only when the output does not exist yet (donor `wx` flag).
fn write_exclusive(path: &str, bytes: &[u8]) -> Result<(), AasError> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

/// Run the donor command line with a caller-supplied reachability rebuild:
/// parse, then reachability-or-cluster, then optimize, then write with a
/// reparse verification (donor `authorAas` body).
pub fn author_aas_with(argv: &[String], regenerate: &AasRegenerator<'_>) -> Result<(), AasError> {
    let plan = parse_author_args(argv)?;
    if lexical_absolute(Path::new(&plan.input)) == lexical_absolute(Path::new(&plan.output)) {
        return Err(AasError::DistinctOutput);
    }
    let bytes = std::fs::read(&plan.input)?;
    let mut asset = parse_aas(&bytes, &plan.input, None)?;
    if let Some(request) = &plan.reachability {
        asset = regenerate(&asset, request)?;
    } else if plan.cluster {
        asset = cluster_aas(&asset, None)?;
    }
    if plan.optimize {
        asset = optimize_aas(&asset)?;
    }
    let bytes = write_aas(&asset)?;
    parse_aas(&bytes, &plan.output, Some(asset.bsp_checksum))?;
    write_exclusive(&plan.output, &bytes)
}

/// Run the donor command line (donor `authorAas`). The `--reachability`
/// path reports [`AasError::ReachabilityUnavailable`] until the
/// application-simulation lane wires a rebuild through [`author_aas_with`].
pub fn author_aas(argv: &[String]) -> Result<(), AasError> {
    author_aas_with(argv, &|_, _| Err(AasError::ReachabilityUnavailable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::movement_contract::{MovementKind, MovementProfile};
    use qa_bots::scene::{
        BodyShape, BspPlane, LeafQueryResult, PointContentsQuery, PointContentsResult, Q1WorldGeometry, TraceContact,
        TraceDetail, TraceHit, TracePolicy, TraceQuery, TraceResult, VisibilityKind,
    };
    use qa_bots::types::TravelMode;
    use qa_bots::{
        AasArea, AasAreaSettings, AasBbox, AasCluster, AasEdge, AasFace, AasNode, AasPlane, AasReachability,
    };
    use qa_core::identity::ProviderId;
    use qa_core::math::{vec3, Bounds};
    use qa_core::numeric::Q3_BINARY32_PROFILE;
    use std::cell::RefCell;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    fn push_i32(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    /// Minimal AAS v4: `area_count` empty areas with matching settings and
    /// one reachability per travel type (same layout as the inspect
    /// fixture).
    fn fixture_aas(checksum: i32, area_count: usize, travel_types: &[i32]) -> Vec<u8> {
        let areas_len = area_count * 48;
        let settings_len = area_count * 28;
        let reach_len = travel_types.len() * 44;
        let areas_offset = 124;
        let settings_offset = areas_offset + areas_len;
        let reach_offset = settings_offset + settings_len;
        let mut out = Vec::new();
        push_u32(&mut out, 0x5341_4145);
        push_i32(&mut out, 4);
        push_i32(&mut out, checksum);
        for lump in 0..14 {
            let (offset, length) = match lump {
                7 => (areas_offset, areas_len),
                8 => (settings_offset, settings_len),
                9 => (reach_offset, reach_len),
                _ => (0, 0),
            };
            push_i32(&mut out, offset as i32);
            push_i32(&mut out, length as i32);
        }
        for number in 0..area_count {
            push_i32(&mut out, number as i32);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            for _ in 0..9 {
                push_f32(&mut out, 0.0);
            }
        }
        for setting in 0..area_count {
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, if setting == 0 { travel_types.len() as i32 } else { 0 });
            push_i32(&mut out, 0);
        }
        for travel_type in travel_types {
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            push_i32(&mut out, 0);
            for _ in 0..6 {
                push_f32(&mut out, 0.0);
            }
            push_i32(&mut out, *travel_type);
            push_u16(&mut out, 0);
            push_u16(&mut out, 0);
        }
        out
    }

    /// Two-area linked asset with ground faces, edges, and planes, so the
    /// cluster pass has geometry to flood (mirrors the `qa-bots`
    /// `linked_aas` integration fixture).
    fn linked_aas(checksum: i32) -> AasAsset {
        let bounds = |min: qa_core::math::Vec3, max: qa_core::math::Vec3| Bounds { min, max };
        AasAsset {
            source: "fixture".to_string(),
            version: 5,
            bsp_checksum: checksum,
            lumps: Vec::new(),
            bboxes: vec![AasBbox {
                presence: 3,
                flags: 0,
                bounds: bounds(vec3(-16.0, -16.0, -24.0), vec3(16.0, 16.0, 32.0)),
            }],
            vertices: vec![
                vec3(0.0, 0.0, 0.0),
                vec3(64.0, 0.0, 0.0),
                vec3(64.0, 64.0, 0.0),
                vec3(0.0, 64.0, 0.0),
                vec3(0.0, -64.0, 0.0),
                vec3(64.0, -64.0, 0.0),
            ],
            planes: vec![
                AasPlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                    plane_type: 2,
                },
                AasPlane {
                    normal: vec3(0.0, 1.0, 0.0),
                    distance: 0.0,
                    plane_type: 1,
                },
            ],
            edges: vec![
                AasEdge { vertices: [0, 0] },
                AasEdge { vertices: [0, 1] },
                AasEdge { vertices: [1, 2] },
                AasEdge { vertices: [2, 3] },
                AasEdge { vertices: [3, 0] },
                AasEdge { vertices: [4, 5] },
                AasEdge { vertices: [5, 1] },
                AasEdge { vertices: [0, 4] },
            ],
            edge_indexes: vec![1, 2, 3, 4, -1, 6, 5, 7],
            faces: vec![
                AasFace {
                    plane: 0,
                    flags: 0,
                    edge_count: 0,
                    first_edge: 0,
                    front_area: 0,
                    back_area: 0,
                },
                AasFace {
                    plane: 0,
                    flags: 4,
                    edge_count: 4,
                    first_edge: 0,
                    front_area: 1,
                    back_area: 0,
                },
                AasFace {
                    plane: 0,
                    flags: 4,
                    edge_count: 4,
                    first_edge: 4,
                    front_area: 2,
                    back_area: 0,
                },
            ],
            face_indexes: vec![1, 2],
            areas: vec![
                AasArea {
                    number: 0,
                    face_count: 0,
                    first_face: 0,
                    bounds: bounds(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)),
                    center: vec3(0.0, 0.0, 0.0),
                },
                AasArea {
                    number: 1,
                    face_count: 1,
                    first_face: 0,
                    bounds: bounds(vec3(0.0, 0.0, 0.0), vec3(64.0, 64.0, 64.0)),
                    center: vec3(32.0, 32.0, 32.0),
                },
                AasArea {
                    number: 2,
                    face_count: 1,
                    first_face: 1,
                    bounds: bounds(vec3(0.0, -64.0, 0.0), vec3(64.0, 0.0, 64.0)),
                    center: vec3(32.0, -32.0, 32.0),
                },
            ],
            settings: vec![
                AasAreaSettings {
                    contents: 0,
                    flags: 0,
                    presence: 0,
                    cluster: 0,
                    cluster_area: 0,
                    reach_count: 0,
                    first_reach: 0,
                },
                AasAreaSettings {
                    contents: 0,
                    flags: 1,
                    presence: 7,
                    cluster: 1,
                    cluster_area: 0,
                    reach_count: 1,
                    first_reach: 1,
                },
                AasAreaSettings {
                    contents: 0,
                    flags: 1,
                    presence: 7,
                    cluster: 1,
                    cluster_area: 1,
                    reach_count: 1,
                    first_reach: 2,
                },
            ],
            reachability: vec![
                AasReachability {
                    area: 0,
                    face: 0,
                    edge: 0,
                    start: vec3(0.0, 0.0, 0.0),
                    end: vec3(0.0, 0.0, 0.0),
                    travel_type: 0,
                    travel_time: 0,
                    padding: 0,
                },
                AasReachability {
                    area: 2,
                    face: 0,
                    edge: 1,
                    start: vec3(32.0, 4.0, 1.0),
                    end: vec3(32.0, -4.0, 1.0),
                    travel_type: 2,
                    travel_time: 100,
                    padding: 0,
                },
                AasReachability {
                    area: 1,
                    face: 0,
                    edge: -1,
                    start: vec3(32.0, -4.0, 1.0),
                    end: vec3(32.0, 4.0, 1.0),
                    travel_type: 2,
                    travel_time: 100,
                    padding: 0,
                },
            ],
            nodes: vec![
                AasNode {
                    plane: 0,
                    children: [0, 0],
                },
                AasNode {
                    plane: 1,
                    children: [-1, -2],
                },
            ],
            portals: Vec::new(),
            portal_indexes: Vec::new(),
            clusters: vec![
                AasCluster {
                    area_count: 0,
                    reachability_area_count: 0,
                    portal_count: 0,
                    first_portal: 0,
                },
                AasCluster {
                    area_count: 2,
                    reachability_area_count: 2,
                    portal_count: 0,
                    first_portal: 0,
                },
            ],
        }
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-tools-aas-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_input(dir: &Path, bytes: &[u8]) -> (String, String) {
        let input = dir.join("input.aas");
        let output = dir.join("output.aas");
        std::fs::write(&input, bytes).unwrap();
        (
            input.to_str().unwrap().to_string(),
            output.to_str().unwrap().to_string(),
        )
    }

    #[test]
    fn usage_covers_every_argument_shape() {
        let cases: &[&[&str]] = &[
            &[],
            &["input.aas"],
            &["input.aas", "output.aas", "--bogus"],
            &["input.aas", "output.aas", "--game"],
            &["input.aas", "output.aas", "--game", "--map"],
            &["input.aas", "output.aas", "--game", "q3", "--game", "q1"],
            &["input.aas", "output.aas", "--game", "q3"],
            &["input.aas", "output.aas", "--reachability"],
            &[
                "input.aas",
                "output.aas",
                "--reachability",
                "--game",
                "q3",
                "--map",
                "q3dm1",
            ],
            &[
                "input.aas",
                "output.aas",
                "--reachability",
                "--game",
                "q3",
                "--map",
                "q3dm1",
                "--movement",
                "q3",
                "--cluster",
                "--bogus",
            ],
        ];
        for words in cases {
            let error = author_aas(&argv(words)).unwrap_err();
            assert_eq!(error.to_string(), USAGE, "argv: {words:?}");
            assert!(matches!(error, AasError::Usage(_)), "argv: {words:?}");
        }
    }

    #[test]
    fn identical_input_and_output_is_rejected_before_any_io() {
        for pair in [["same.aas", "same.aas"], ["sub/../same.aas", "./same.aas"]] {
            let error = author_aas(&argv(&pair)).unwrap_err();
            assert!(matches!(error, AasError::DistinctOutput), "{pair:?}");
            assert_eq!(error.to_string(), "AAS authoring requires a distinct output file");
        }
    }

    #[test]
    fn missing_input_is_an_io_error() {
        let dir = scratch_dir("missing-input");
        let output = dir.join("output.aas");
        let error = author_aas(&argv(&[
            dir.join("absent.aas").to_str().unwrap(),
            output.to_str().unwrap(),
        ]))
        .unwrap_err();
        assert!(matches!(error, AasError::Io(_)));
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_round_trip_preserves_the_asset() {
        let dir = scratch_dir("plain");
        let (input, output) = write_input(&dir, &fixture_aas(1234, 2, &[1, 6]));
        author_aas(&argv(&[&input, &output])).unwrap();
        let written = std::fs::read(&output).unwrap();
        let asset = parse_aas(&written, &output, Some(1234)).unwrap();
        assert_eq!(asset.bsp_checksum, 1234);
        assert_eq!(asset.areas.len(), 2);
        assert_eq!(asset.reachability.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cluster_and_optimize_round_trip() {
        for flags in [
            &["--cluster"][..],
            &["--optimize"][..],
            &["--cluster", "--optimize"][..],
        ] {
            let dir = scratch_dir(&format!("transform-{}", flags.join("-").replace("--", "")));
            let (input, output) = write_input(&dir, &write_aas(&linked_aas(4321)).unwrap());
            let mut words = vec![input.as_str(), output.as_str()];
            words.extend_from_slice(flags);
            author_aas(&argv(&words)).unwrap();
            let written = std::fs::read(&output).unwrap();
            let asset = parse_aas(&written, &output, Some(4321)).unwrap();
            assert_eq!(asset.bsp_checksum, 4321);
            assert_eq!(asset.areas.len(), 3);
            assert_eq!(asset.settings.len(), 3);
            assert_eq!(asset.reachability.len(), 3);
            if flags.contains(&"--cluster") {
                assert!(!asset.clusters.is_empty());
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn existing_output_is_never_overwritten() {
        let dir = scratch_dir("exclusive");
        let (input, output) = write_input(&dir, &fixture_aas(9, 2, &[]));
        std::fs::write(&output, b"previous bytes").unwrap();
        let error = author_aas(&argv(&[&input, &output])).unwrap_err();
        assert!(matches!(error, AasError::Io(_)));
        assert_eq!(std::fs::read(&output).unwrap(), b"previous bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reachability_forwards_options_in_donor_order() {
        let dir = scratch_dir("forwarding");
        let (input, output) = write_input(&dir, &fixture_aas(1234, 2, &[1]));
        let seen: RefCell<Option<ReachabilityRequest>> = RefCell::new(None);
        let calls = RefCell::new(0);
        author_aas_with(
            &argv(&[
                &input,
                &output,
                "--reachability",
                "--map",
                "q3dm1",
                "--content-root",
                "/content",
                "--game",
                "quake3",
                "--movement",
                "q3",
            ]),
            &|asset, request| {
                *calls.borrow_mut() += 1;
                *seen.borrow_mut() = Some(request.clone());
                let mut rebuilt = asset.clone();
                rebuilt.bsp_checksum = 777;
                Ok(rebuilt)
            },
        )
        .unwrap();
        assert_eq!(*calls.borrow(), 1);
        let request = seen.borrow().clone().unwrap();
        assert_eq!(request.game, "quake3");
        assert_eq!(request.map, "q3dm1");
        assert_eq!(request.movement, "q3");
        assert_eq!(request.content_root.as_deref(), Some("/content"));
        assert_eq!(
            request.forwarded_args(),
            &argv(&[
                "--map",
                "q3dm1",
                "--content-root",
                "/content",
                "--game",
                "quake3",
                "--movement",
                "q3"
            ])[..]
        );
        let written = std::fs::read(&output).unwrap();
        assert_eq!(parse_aas(&written, &output, Some(777)).unwrap().bsp_checksum, 777);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reachability_replaces_the_cluster_pass() {
        let run = |name: &str, extra: &[&str]| -> Vec<u8> {
            let dir = scratch_dir(name);
            let (input, output) = write_input(&dir, &fixture_aas(55, 2, &[1]));
            let mut words = vec![
                input.as_str(),
                output.as_str(),
                "--reachability",
                "--game",
                "q3",
                "--map",
                "m",
                "--movement",
                "q3",
            ];
            words.extend_from_slice(extra);
            author_aas_with(&argv(&words), &|asset, _| Ok(asset.clone())).unwrap();
            let bytes = std::fs::read(&output).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            bytes
        };
        assert_eq!(run("reach-only", &[]), run("reach-cluster", &["--cluster"]));
    }

    #[test]
    fn reachability_without_a_wired_simulation_reports_the_gap() {
        let dir = scratch_dir("unavailable");
        let (input, output) = write_input(&dir, &fixture_aas(7, 2, &[]));
        let error = author_aas(&argv(&[
            &input,
            &output,
            "--reachability",
            "--game",
            "q3",
            "--map",
            "m",
            "--movement",
            "q3",
        ]))
        .unwrap_err();
        assert!(matches!(error, AasError::ReachabilityUnavailable));
        assert!(error.to_string().contains("application simulation stack"), "{error}");
        assert!(!Path::new(&output).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Open scene: every sweep completes, nothing is solid.
    struct OpenScene;

    impl SceneQueries for OpenScene {
        fn trace(&self, query: &TraceQuery) -> TraceResult {
            TraceResult {
                fraction: 1.0,
                end: query.end,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                detail: TraceDetail::Q3 {
                    contents: 0,
                    surface_flags: 0,
                    source_plane: BspPlane {
                        normal: vec3(0.0, 0.0, 1.0),
                        distance: 0.0,
                        plane_type: 2,
                        signbits: 0,
                    },
                },
            }
        }

        fn point_contents(&self, _query: &PointContentsQuery) -> PointContentsResult {
            PointContentsResult::Q3 { contents: 0 }
        }

        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> LeafQueryResult {
            LeafQueryResult {
                leaves: Vec::new(),
                topnode: None,
                overflow: false,
            }
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }

        fn cluster_visible(&self, _from: i32, _to: i32, _kind: VisibilityKind) -> bool {
            true
        }
    }

    fn test_profile() -> NavigationProfile {
        NavigationProfile {
            movement: MovementProfile {
                kind: MovementKind::Q3,
                id: ProviderId::new("test", "movement"),
                numeric: Q3_BINARY32_PROFILE,
            },
            shape: BodyShape::Box(Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            }),
            crouched_shape: None,
            policy: TracePolicy::Q3 {
                contents_mask: -1,
                curves: true,
                player_curve_clip: true,
            },
            capabilities: [TravelMode::Walk].into_iter().collect(),
            maximum_step: 18.0,
            minimum_floor_normal: 0.7,
            maximum_drop: 64.0,
            team: None,
            monster: false,
        }
    }

    fn empty_q1_world() -> DecodedWorld {
        DecodedWorld::Q1(Q1WorldGeometry {
            entities: "{ \"classname\" \"worldspawn\" }".to_string(),
            planes: Vec::new(),
            vertices: Vec::new(),
            edges: Vec::new(),
            surface_edges: Vec::new(),
            leaves: Vec::new(),
            faces: Vec::new(),
            models: Vec::new(),
        })
    }

    fn stub_prediction(prediction: BotMovementPrediction) -> BotTravelPredictionResult {
        BotTravelPredictionResult {
            end: prediction.origin,
            velocity: vec3(0.0, 0.0, 0.0),
            frames: 0,
            stop_event: 0,
            end_area: None,
        }
    }

    #[test]
    fn regenerate_rejects_assets_from_other_map_bytes() {
        let map_bytes = b"the canonical map bytes";
        let checksum = block_checksum(map_bytes).unwrap() as i32;
        let asset = parse_aas(&fixture_aas(checksum, 2, &[]), "input.aas", None).unwrap();
        let geometry = empty_q1_world();
        let scene = OpenScene;
        let profile = test_profile();
        let inputs = AasRegeneration {
            map_bytes: b"different map bytes",
            geometry: &geometry,
            scene: &scene,
            profile: &profile,
            prediction_client: 0,
            predict_client_movement: &stub_prediction,
        };
        let error = regenerate_aas(&asset, &inputs).unwrap_err();
        assert!(matches!(error, AasError::ChecksumMismatch));
        assert_eq!(error.to_string(), "Input AAS belongs to a different BSP checksum");
    }

    #[test]
    fn regenerate_rebuilds_reachability_then_clusters() {
        let map_bytes = b"the canonical map bytes";
        let checksum = block_checksum(map_bytes).unwrap() as i32;
        let asset = parse_aas(&fixture_aas(checksum, 2, &[]), "input.aas", None).unwrap();
        let geometry = empty_q1_world();
        let scene = OpenScene;
        let profile = test_profile();
        let inputs = AasRegeneration {
            map_bytes,
            geometry: &geometry,
            scene: &scene,
            profile: &profile,
            prediction_client: 0,
            predict_client_movement: &stub_prediction,
        };
        let rebuilt = regenerate_aas(&asset, &inputs).unwrap();
        assert_eq!(rebuilt.bsp_checksum, checksum);
        assert_eq!(rebuilt.areas.len(), asset.areas.len());
    }
}
