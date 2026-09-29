//! `AAS_Optimize` from id Software `be_aas_optimize.c`, ported from
//! `src/bots/navigation/aas-optimize.ts`. Source ladder-only geometry
//! compaction; source arrays and routing metadata remain immutable.
//! Copyright (C) 1999-2005 Id Software, Inc.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3};

use crate::aas::{AasArea, AasAsset, AasEdge, AasFace, AasReachability};
use crate::error::{indexed, BotsError};

struct Optimizer<'a> {
    asset: &'a AasAsset,
    vertices: Vec<Vec3>,
    edges: Vec<AasEdge>,
    faces: Vec<AasFace>,
    edge_indexes: Vec<i32>,
    face_indexes: Vec<i32>,
    vertex_map: HashMap<i32, i32>,
    edge_map: HashMap<i32, i32>,
    face_map: HashMap<i32, i32>,
}

impl<'a> Optimizer<'a> {
    fn new(asset: &'a AasAsset) -> Self {
        Self {
            asset,
            vertices: Vec::new(),
            edges: vec![AasEdge { vertices: [0, 0] }],
            faces: vec![AasFace {
                plane: 0,
                flags: 0,
                edge_count: 0,
                first_edge: 0,
                front_area: 0,
                back_area: 0,
            }],
            edge_indexes: Vec::new(),
            face_indexes: Vec::new(),
            vertex_map: HashMap::new(),
            edge_map: HashMap::new(),
            face_map: HashMap::new(),
        }
    }

    fn at<T>(values: &[T], index: i32) -> Result<&T, BotsError> {
        indexed(values, i64::from(index), "AAS optimization index")
    }

    fn vertex(&mut self, number: i32) -> Result<i32, BotsError> {
        let mapped = self.vertex_map.get(&number).copied().unwrap_or(0);
        // Preserve source zero sentinel behavior, including duplication
        // of output vertex zero.
        if mapped != 0 {
            return Ok(mapped);
        }
        let result = self.vertices.len() as i32;
        self.vertices.push(*Self::at(&self.asset.vertices, number)?);
        self.vertex_map.insert(number, result);
        Ok(result)
    }

    fn edge(&mut self, number: i32) -> Result<i32, BotsError> {
        let key = number.unsigned_abs() as i32;
        let source = *Self::at(&self.asset.edges, key)?;
        let result = match self.edge_map.get(&key) {
            Some(result) => *result,
            None => {
                let first = self.vertex(source.vertices[0])?;
                let second = self.vertex(source.vertices[1])?;
                let result = self.edges.len() as i32;
                self.edges.push(AasEdge {
                    vertices: [first, second],
                });
                self.edge_map.insert(key, result);
                result
            }
        };
        Ok(if number > 0 { result } else { -result })
    }

    fn face(&mut self, number: i32) -> Result<i32, BotsError> {
        let key = number.unsigned_abs() as i32;
        let source = *Self::at(&self.asset.faces, key)?;
        if (source.flags & 2) == 0 {
            return Ok(0);
        }
        let result = match self.face_map.get(&key) {
            Some(result) => *result,
            None => {
                let first_edge = self.edge_indexes.len() as i32;
                for offset in 0..source.edge_count {
                    let signed = *Self::at(&self.asset.edge_indexes, source.first_edge + offset)?;
                    let mapped = self.edge(signed)?;
                    self.edge_indexes.push(mapped);
                }
                let result = self.faces.len() as i32;
                self.faces.push(AasFace {
                    first_edge,
                    edge_count: self.edge_indexes.len() as i32 - first_edge,
                    ..source
                });
                self.face_map.insert(key, result);
                result
            }
        };
        Ok(if number > 0 { result } else { -result })
    }

    fn remap(number: i32, map: &HashMap<i32, i32>) -> i32 {
        (if number < 0 { -1 } else { 1 }) * map.get(&(number.unsigned_abs() as i32)).copied().unwrap_or(0)
    }
}

/// Compact ladder geometry, remapping faces and edges in source
/// first-reference order.
pub fn optimize_aas(asset: &AasAsset) -> Result<AasAsset, BotsError> {
    let mut optimizer = Optimizer::new(asset);
    let mut areas: Vec<AasArea> = Vec::with_capacity(asset.areas.len());
    for (number, area) in asset.areas.iter().enumerate() {
        if number == 0 {
            areas.push(AasArea {
                number: 0,
                face_count: 0,
                first_face: 0,
                bounds: Bounds {
                    min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                },
                center: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            });
            continue;
        }
        let first_face = optimizer.face_indexes.len() as i32;
        for offset in 0..area.face_count {
            let signed = *Optimizer::at(&asset.face_indexes, area.first_face + offset)?;
            let mapped = optimizer.face(signed)?;
            if mapped != 0 {
                optimizer.face_indexes.push(mapped);
            }
        }
        areas.push(AasArea {
            first_face,
            face_count: optimizer.face_indexes.len() as i32 - first_face,
            ..*area
        });
    }
    let reachability: Vec<AasReachability> = asset
        .reachability
        .iter()
        .map(|reach| {
            let travel = reach.travel_type & 0x00ff_ffff;
            if travel == 11 || travel == 18 || travel == 19 {
                *reach
            } else {
                AasReachability {
                    face: Optimizer::remap(reach.face, &optimizer.face_map),
                    edge: Optimizer::remap(reach.edge, &optimizer.edge_map),
                    ..*reach
                }
            }
        })
        .collect();
    Ok(AasAsset {
        lumps: Vec::new(),
        vertices: optimizer.vertices,
        edges: optimizer.edges,
        edge_indexes: optimizer.edge_indexes,
        faces: optimizer.faces,
        face_indexes: optimizer.face_indexes,
        areas,
        reachability,
        ..asset.clone()
    })
}
