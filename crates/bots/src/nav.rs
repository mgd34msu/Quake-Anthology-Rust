//! NAV2 layout from `quake-1-re-ts/src/lib/nav.ts` (retail-derived
//! v12-v18) and NAV3 layout from q2repro `src/server/nav.c`,
//! `inc/server/nav.h`, ported from `src/bots/navigation/nav.ts`.
//! Copyright (C) 2003-2006 Andrey Nazarov.

use qa_core::binary::{BinaryError, BinaryReader};
use qa_core::math::{Bounds, Vec3};

use crate::error::BotsError;
use crate::types::{KexGeneration, TraversalHint};

/// Kex navigation node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KexNode {
    /// Node flags.
    pub flags: i32,
    /// First link index.
    pub first_link: i32,
    /// Link count.
    pub link_count: i32,
    /// Node radius.
    pub radius: i32,
    /// Node origin.
    pub origin: Vec3,
}

/// Kex navigation link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KexLink {
    /// Target node.
    pub target: i32,
    /// Link type.
    pub link_type: i32,
    /// Effective flags.
    pub flags: i32,
    /// Stored flags.
    pub stored_flags: i32,
    /// Traversal index.
    pub traversal: Option<i32>,
}

/// Kex link entity binding.
#[derive(Debug, Clone, PartialEq)]
pub struct KexEntity {
    /// Source link index.
    pub link: i32,
    /// Source model (NAV3 only).
    pub model: Option<i32>,
    /// Entity bounds.
    pub bounds: Bounds,
    /// Trailing source words.
    pub tail: Vec<i32>,
}

/// Parsed Kex NAV2/NAV3 asset.
#[derive(Debug, Clone, PartialEq)]
pub struct KexNavigationAsset {
    /// Generation.
    pub kind: KexGeneration,
    /// Source name.
    pub source: String,
    /// Format version.
    pub version: i32,
    /// Cost multiplier.
    pub heuristic: f64,
    /// Nodes.
    pub nodes: Vec<KexNode>,
    /// Links.
    pub links: Vec<KexLink>,
    /// Traversal hints.
    pub traversals: Vec<TraversalHint>,
    /// Link entities.
    pub entities: Vec<KexEntity>,
}

fn read_vector(reader: &mut BinaryReader<'_>) -> Result<Vec3, BinaryError> {
    Ok(Vec3 {
        x: reader.finite_f32()?,
        y: reader.finite_f32()?,
        z: reader.finite_f32()?,
    })
}

fn read_count(reader: &mut BinaryReader<'_>, source: &str, stride: usize) -> Result<usize, BotsError> {
    let offset = reader.offset();
    let value = reader.i32()?;
    if value < 0 || value as usize > reader.length() / stride {
        return Err(BinaryError::custom(source, offset, "invalid navigation record count").into());
    }
    Ok(value as usize)
}

/// Parse Kex NAV2/NAV3 bytes.
pub fn parse_kex_navigation(bytes: &[u8], source: &str) -> Result<KexNavigationAsset, BotsError> {
    let mut reader = BinaryReader::new(bytes, source);
    let magic = reader.fixed_string(4)?;
    let kind = match magic.as_str() {
        "NAV2" => KexGeneration::Nav2,
        "NAV3" => KexGeneration::Nav3,
        _ => return Err(BinaryError::custom(source, 0, "expected NAV2 or NAV3").into()),
    };
    let version = reader.i32()?;
    let supported = match kind {
        KexGeneration::Nav2 => (12..=18).contains(&version),
        KexGeneration::Nav3 => (1..=6).contains(&version),
    };
    if !supported {
        return Err(BinaryError::custom(source, 4, format!("unsupported {magic} version {version}")).into());
    }
    let node_count = read_count(&mut reader, source, 20)?;
    let link_count = read_count(&mut reader, source, 6)?;
    let traversal_count = read_count(&mut reader, source, 36)?;
    let heuristic = if kind == KexGeneration::Nav3 || version >= 16 {
        f64::from(reader.finite_f32()?)
    } else {
        1.0
    };
    if heuristic <= 0.0 {
        return Err(BinaryError::custom(
            source,
            reader.offset().saturating_sub(4),
            "navigation cost multiplier must be positive",
        )
        .into());
    }
    let mut headers = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let flags = i32::from(reader.u16()?);
        let links = reader.u16()?;
        let first_link = reader.u16()?;
        let radius = i32::from(reader.u16()?);
        if usize::from(first_link) + usize::from(links) > link_count {
            return Err(BinaryError::custom(
                source,
                reader.offset().saturating_sub(8),
                "node link range exceeds table",
            )
            .into());
        }
        headers.push((flags, links, first_link, radius));
    }
    let mut nodes = Vec::with_capacity(node_count);
    for (flags, links, first_link, radius) in headers {
        nodes.push(KexNode {
            flags,
            first_link: i32::from(first_link),
            link_count: i32::from(links),
            radius,
            origin: read_vector(&mut reader)?,
        });
    }
    let mut links = Vec::with_capacity(link_count);
    for _ in 0..link_count {
        let target = reader.u16()?;
        let link_type = reader.u8()?;
        let stored_flags = reader.u8()?;
        let traversal = reader.u16()?;
        if usize::from(target) >= node_count || traversal != 0xffff && usize::from(traversal) >= traversal_count {
            return Err(BinaryError::custom(
                source,
                reader.offset().saturating_sub(6),
                "navigation link references missing record",
            )
            .into());
        }
        // NAV2 v18 flags have unresolved semantics; retain them without
        // applying NAV3's Disabled bit.
        let flags = if kind == KexGeneration::Nav3 && version < 3 {
            3
        } else if kind == KexGeneration::Nav3 && version < 6 {
            i32::from(stored_flags) & !12
        } else {
            i32::from(stored_flags)
        };
        links.push(KexLink {
            target: i32::from(target),
            link_type: i32::from(link_type),
            flags,
            stored_flags: i32::from(stored_flags),
            traversal: if traversal == 0xffff {
                None
            } else {
                Some(i32::from(traversal))
            },
        });
    }
    let mut traversals = Vec::with_capacity(traversal_count);
    for _ in 0..traversal_count {
        traversals.push(TraversalHint {
            funnel: read_vector(&mut reader)?,
            start: read_vector(&mut reader)?,
            end: read_vector(&mut reader)?,
            ladder_plane: if kind == KexGeneration::Nav3 && version >= 4 {
                Some(read_vector(&mut reader)?)
            } else {
                None
            },
        });
    }
    let entity_count = read_count(&mut reader, source, 26)?;
    let mut entities = Vec::with_capacity(entity_count);
    for _ in 0..entity_count {
        let link = reader.u16()?;
        let model = match kind {
            KexGeneration::Nav3 if version >= 2 => Some(reader.i32()?),
            KexGeneration::Nav3 => Some(0),
            KexGeneration::Nav2 => None,
        };
        let bounds = Bounds {
            min: read_vector(&mut reader)?,
            max: read_vector(&mut reader)?,
        };
        let mut tail = Vec::new();
        if kind == KexGeneration::Nav2 {
            let words = if version <= 12 {
                0
            } else if version <= 14 {
                2
            } else {
                1
            };
            for _ in 0..words {
                tail.push(reader.i32()?);
            }
        }
        if usize::from(link) >= link_count {
            return Err(BinaryError::custom(source, reader.offset(), "entity references missing link").into());
        }
        if bounds.min.x > bounds.max.x || bounds.min.y > bounds.max.y || bounds.min.z > bounds.max.z {
            return Err(BinaryError::custom(source, reader.offset(), "inverted entity bounds").into());
        }
        entities.push(KexEntity {
            link: i32::from(link),
            model,
            bounds,
            tail,
        });
    }
    if reader.remaining() != 0 {
        return Err(BinaryError::custom(source, reader.offset(), "unconsumed navigation bytes").into());
    }
    Ok(KexNavigationAsset {
        kind,
        source: source.to_string(),
        version,
        heuristic,
        nodes,
        links,
        traversals,
        entities,
    })
}
