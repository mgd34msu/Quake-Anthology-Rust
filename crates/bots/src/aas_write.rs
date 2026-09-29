//! `AAS_WriteAASFile` from id Software, ported from
//! `src/bots/navigation/aas-write.ts`. Shared immutable asset output:
//! version 5 with the source header obfuscation, validated by reparse.
//! Copyright (C) 1999-2005 Id Software, Inc.

use qa_core::binary::BinaryWriter;
use qa_core::math::Vec3;

use crate::aas::{parse_aas, AasAsset};
use crate::error::BotsError;

fn write_vector(writer: &mut BinaryWriter, value: Vec3) -> Result<(), BotsError> {
    writer.f32(value.x)?;
    writer.f32(value.y)?;
    writer.f32(value.z)?;
    Ok(())
}

/// Serialize an AAS asset to version-5 bytes.
pub fn write_aas(world: &AasAsset) -> Result<Vec<u8>, BotsError> {
    let mut lumps: Vec<Vec<u8>> = Vec::new();
    let mut header = BinaryWriter::new(124);
    header.i32(0x5341_4145)?;
    header.i32(5)?;
    header.i32(world.bsp_checksum)?;
    let mut offset = 124usize;
    let mut write_lump = |bytes: Vec<u8>| -> Result<(), BotsError> {
        header.i32(offset as i32)?;
        header.i32(bytes.len() as i32)?;
        offset += bytes.len();
        lumps.push(bytes);
        Ok(())
    };
    let mut writer = BinaryWriter::new(world.bboxes.len() * 32);
    for bbox in &world.bboxes {
        writer.i32(bbox.presence)?;
        writer.i32(bbox.flags)?;
        write_vector(&mut writer, bbox.bounds.min)?;
        write_vector(&mut writer, bbox.bounds.max)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.vertices.len() * 12);
    for vertex in &world.vertices {
        write_vector(&mut writer, *vertex)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.planes.len() * 20);
    for plane in &world.planes {
        write_vector(&mut writer, plane.normal)?;
        writer.f32(plane.distance)?;
        writer.i32(plane.plane_type)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.edges.len() * 8);
    for edge in &world.edges {
        writer.i32(edge.vertices[0])?;
        writer.i32(edge.vertices[1])?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.edge_indexes.len() * 4);
    for value in &world.edge_indexes {
        writer.i32(*value)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.faces.len() * 24);
    for face in &world.faces {
        writer.i32(face.plane)?;
        writer.i32(face.flags)?;
        writer.i32(face.edge_count)?;
        writer.i32(face.first_edge)?;
        writer.i32(face.front_area)?;
        writer.i32(face.back_area)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.face_indexes.len() * 4);
    for value in &world.face_indexes {
        writer.i32(*value)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.areas.len() * 48);
    for area in &world.areas {
        writer.i32(area.number)?;
        writer.i32(area.face_count)?;
        writer.i32(area.first_face)?;
        write_vector(&mut writer, area.bounds.min)?;
        write_vector(&mut writer, area.bounds.max)?;
        write_vector(&mut writer, area.center)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.settings.len() * 28);
    for setting in &world.settings {
        writer.i32(setting.contents)?;
        writer.i32(setting.flags)?;
        writer.i32(setting.presence)?;
        writer.i32(setting.cluster)?;
        writer.i32(setting.cluster_area)?;
        writer.i32(setting.reach_count)?;
        writer.i32(setting.first_reach)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.reachability.len() * 44);
    for reach in &world.reachability {
        writer.i32(reach.area)?;
        writer.i32(reach.face)?;
        writer.i32(reach.edge)?;
        write_vector(&mut writer, reach.start)?;
        write_vector(&mut writer, reach.end)?;
        writer.i32(reach.travel_type)?;
        writer.u16(reach.travel_time)?;
        writer.u16(reach.padding)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.nodes.len() * 12);
    for node in &world.nodes {
        writer.i32(node.plane)?;
        writer.i32(node.children[0])?;
        writer.i32(node.children[1])?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.portals.len() * 20);
    for portal in &world.portals {
        writer.i32(portal.area)?;
        writer.i32(portal.front_cluster)?;
        writer.i32(portal.back_cluster)?;
        writer.i32(portal.cluster_areas[0])?;
        writer.i32(portal.cluster_areas[1])?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.portal_indexes.len() * 4);
    for value in &world.portal_indexes {
        writer.i32(*value)?;
    }
    write_lump(writer.finish())?;
    let mut writer = BinaryWriter::new(world.clusters.len() * 16);
    for cluster in &world.clusters {
        writer.i32(cluster.area_count)?;
        writer.i32(cluster.reachability_area_count)?;
        writer.i32(cluster.portal_count)?;
        writer.i32(cluster.first_portal)?;
    }
    write_lump(writer.finish())?;
    let mut bytes = header.finish();
    for (index, byte) in bytes.iter_mut().enumerate().skip(8) {
        *byte ^= (((index - 8) * 119) & 255) as u8;
    }
    let mut result = BinaryWriter::new(offset);
    result.bytes(&bytes)?;
    for lump in &lumps {
        result.bytes(lump)?;
    }
    let output = result.finish();
    parse_aas(&output, &world.source, Some(world.bsp_checksum))?;
    Ok(output)
}
