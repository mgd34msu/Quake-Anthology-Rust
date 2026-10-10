use super::brushes::GeometryError;
use qa_core::{
    names::NameTable,
    primitives::{GeometryId, NameId, SurfaceFlags, SurfaceId},
};

/// File rows are converted once, retaining native texinfo/shader indices.
#[derive(Clone, Copy)]
pub struct SurfaceInput<'a> {
    pub name: &'a [u8],
    pub material: &'a [u8],
    pub flags: SurfaceFlags,
    pub value: i32,
    pub source_index: u32,
}
impl SurfaceInput<'_> {
    pub fn unnamed(flags: SurfaceFlags) -> Self {
        Self {
            name: b"",
            material: b"",
            flags,
            value: 0,
            source_index: 0,
        }
    }
}
pub struct SurfaceTable<'a> {
    pub records: Vec<SurfaceInput<'a>>,
    pub sides: Vec<u32>,
}
impl SurfaceTable<'static> {
    /// Analytic geometry has flags without a file surface namespace.
    pub fn flags(flags: Vec<SurfaceFlags>) -> Self {
        Self {
            sides: (0..flags.len()).map(|index| index as u32).collect(),
            records: flags.into_iter().map(SurfaceInput::unnamed).collect(),
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct SurfaceRecord {
    pub name: NameId,
    pub material: NameId,
    pub flags: SurfaceFlags,
    pub value: i32,
    pub source_index: u32,
}
impl SurfaceRecord {
    pub const EMPTY: Self = Self {
        name: NameId(0),
        material: NameId(0),
        flags: SurfaceFlags(0),
        value: 0,
        source_index: 0,
    };
}
#[derive(Clone, Copy)]
pub struct SurfaceView<'a> {
    pub name: &'a [u8],
    pub material: &'a [u8],
    pub flags: SurfaceFlags,
    pub value: i32,
    pub source_index: u32,
}
pub(super) struct SurfaceStorage {
    names: NameTable,
    records: Box<[SurfaceRecord]>,
    sides: Box<[u32]>,
    pub geometry: Option<GeometryId>,
}
impl SurfaceStorage {
    pub fn load(table: SurfaceTable<'_>, sides: usize) -> Result<Self, GeometryError> {
        if table.records.len() > u32::MAX as usize
            || table.sides.len() != sides
            || table
                .sides
                .iter()
                .any(|&index| index as usize >= table.records.len())
        {
            return Err(GeometryError::Surface);
        }
        let names = NameTable::load(
            table
                .records
                .iter()
                .flat_map(|row| [row.name, row.material]),
        )
        .map_err(|_| GeometryError::Capacity)?;
        let records = table
            .records
            .into_iter()
            .map(|row| {
                Ok(SurfaceRecord {
                    name: names.find(row.name).ok_or(GeometryError::Surface)?,
                    material: names.find(row.material).ok_or(GeometryError::Surface)?,
                    flags: row.flags,
                    value: row.value,
                    source_index: row.source_index,
                })
            })
            .collect::<Result<Box<[_]>, GeometryError>>()?;
        Ok(Self {
            names,
            records,
            sides: table.sides.into_boxed_slice(),
            geometry: None,
        })
    }
    pub fn view(&self, index: u32) -> Option<SurfaceView<'_>> {
        let row = self.records.get(index as usize)?;
        Some(SurfaceView {
            name: self.names.get(row.name)?,
            material: self.names.get(row.material)?,
            flags: row.flags,
            value: row.value,
            source_index: row.source_index,
        })
    }
    pub fn count(&self) -> u32 {
        self.records.len() as u32
    }
    pub fn borrow(&self) -> SurfaceRows<'_> {
        SurfaceRows {
            records: &self.records,
            sides: &self.sides,
            geometry: self.geometry,
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct SurfaceRows<'a> {
    pub records: &'a [SurfaceRecord],
    pub sides: &'a [u32],
    pub geometry: Option<GeometryId>,
}
impl SurfaceRows<'_> {
    pub fn contact(self, side: usize) -> (SurfaceFlags, Option<SurfaceId>) {
        let index = self.sides[side];
        (
            self.records[index as usize].flags,
            self.geometry.map(|geometry| SurfaceId { geometry, index }),
        )
    }
}
