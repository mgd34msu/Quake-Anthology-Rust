//! Q2 rerelease native weapon declaration parsing and admission.
//!
//! Donor: `src/compat/q2/rerelease/native-weapon-declaration.ts` — bridges
//! evidence-backed weapon declarations into validated trajectory profiles.

use std::collections::HashMap;

use thiserror::Error;

use super::layouts::{edict_layout, field_offset};

/// Declaration failure with a read path.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{path}: {message}")]
pub struct DeclarationError {
    /// Read path.
    pub path: String,
    /// Message.
    pub message: String,
}

/// Parsed declaration value.
#[derive(Debug, Clone, PartialEq)]
pub enum DeclValue {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// String.
    Str(String),
    /// List.
    List(Vec<DeclValue>),
    /// Record.
    Map(HashMap<String, DeclValue>),
}

/// Reader over a parsed declaration value.
#[derive(Debug, Clone)]
pub struct DeclReader<'a> {
    value: &'a DeclValue,
    path: String,
}

impl<'a> DeclReader<'a> {
    /// Create a reader.
    #[must_use]
    pub fn new(value: &'a DeclValue, path: &str) -> Self {
        Self {
            value,
            path: path.to_string(),
        }
    }

    /// Raw value.
    #[must_use]
    pub const fn value(&self) -> &'a DeclValue {
        self.value
    }

    fn fail(&self, message: &str) -> DeclarationError {
        DeclarationError {
            path: self.path.clone(),
            message: message.to_string(),
        }
    }

    /// Read a record field.
    pub fn field(&self, name: &str) -> Result<DeclReader<'a>, DeclarationError> {
        match self.value {
            DeclValue::Map(map) => map.get(name).map_or_else(
                || Err(self.fail(&format!("missing field {name}"))),
                |value| {
                    Ok(DeclReader {
                        value,
                        path: format!("{}.{name}", self.path),
                    })
                },
            ),
            _ => Err(self.fail("expected a record")),
        }
    }

    /// Reject unknown record fields.
    pub fn keys(&self, allowed: &[&str]) -> Result<(), DeclarationError> {
        match self.value {
            DeclValue::Map(map) => {
                for name in map.keys() {
                    if !allowed.contains(&name.as_str()) {
                        return Err(self.fail("unsupported declaration field"));
                    }
                }
                Ok(())
            }
            _ => Err(self.fail("expected a declaration record")),
        }
    }

    /// Read a string.
    pub fn string(&self) -> Result<String, DeclarationError> {
        match self.value {
            DeclValue::Str(value) => Ok(value.clone()),
            _ => Err(self.fail("expected a string")),
        }
    }

    /// Read an integer with a minimum.
    pub fn integer(&self, minimum: i64) -> Result<i64, DeclarationError> {
        match self.value {
            DeclValue::Int(value) if *value >= minimum => Ok(*value),
            _ => Err(self.fail("expected an integer in range")),
        }
    }

    /// Read a list.
    pub fn list<T>(
        &self,
        read: impl Fn(&DeclReader<'a>) -> Result<T, DeclarationError>,
    ) -> Result<Vec<T>, DeclarationError> {
        match self.value {
            DeclValue::List(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    read(&DeclReader {
                        value,
                        path: format!("{}[{index}]", self.path),
                    })
                })
                .collect(),
            _ => Err(self.fail("expected an array")),
        }
    }

    /// Read a nullable value.
    pub fn nullable<T>(
        &self,
        read: impl Fn(&DeclReader<'a>) -> Result<T, DeclarationError>,
    ) -> Result<Option<T>, DeclarationError> {
        match self.value {
            DeclValue::Null => Ok(None),
            _ => read(self).map(Some),
        }
    }

    /// Read an expected literal.
    pub fn literal_str(&self, expected: &str) -> Result<String, DeclarationError> {
        let value = self.string()?;
        if value == expected {
            Ok(value)
        } else {
            Err(self.fail(&format!("expected {expected}")))
        }
    }

    /// Read an expected integer literal.
    pub fn literal_int(&self, expected: i64) -> Result<i64, DeclarationError> {
        let value = self.integer(i64::MIN)?;
        if value == expected {
            Ok(value)
        } else {
            Err(self.fail(&format!("expected {expected}")))
        }
    }

    /// Read a choice of strings.
    pub fn choice(&self, choices: &[&str]) -> Result<String, DeclarationError> {
        let value = self.string()?;
        if choices.contains(&value.as_str()) {
            Ok(value)
        } else {
            Err(self.fail(&format!("expected {}", choices.join(" or "))))
        }
    }

    /// Read a namespaced identity.
    pub fn namespaced(&self) -> Result<String, DeclarationError> {
        let value = self.string()?;
        match value.find(':') {
            Some(colon) if colon > 0 && colon < value.len() - 1 => Ok(value),
            _ => Err(self.fail("expected a namespaced identity")),
        }
    }
}

fn uint(reader: &DeclReader, minimum: i64) -> Result<u64, DeclarationError> {
    let value = reader.integer(minimum)?;
    if value > 0xffff_ffff {
        return Err(DeclarationError {
            path: reader.path.clone(),
            message: "expected a uint32 byte offset or value".to_string(),
        });
    }
    Ok(value as u64)
}

fn text(reader: &DeclReader, allow_empty: bool) -> Result<String, DeclarationError> {
    let value = reader.string()?;
    if (!allow_empty && value.is_empty()) || value.contains('\0') {
        return Err(DeclarationError {
            path: reader.path.clone(),
            message: "expected non-NUL text".to_string(),
        });
    }
    Ok(value)
}

/// Normalize a relative resource path.
fn normalize_resource_path(path: &str) -> Result<String, DeclarationError> {
    let normalized = path.replace('\\', "/");
    let bad = normalized.is_empty()
        || normalized.contains('\0')
        || normalized.len() >= 2 && normalized.as_bytes()[1] == b':' && normalized.as_bytes()[0].is_ascii_alphabetic()
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if bad {
        return Err(DeclarationError {
            path: "artifactPath".to_string(),
            message: format!("Invalid relative resource path: {path}"),
        });
    }
    Ok(normalized)
}

/// Registration record layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclRegistrationLayout {
    /// Record byte length.
    pub byte_length: usize,
    /// Name offset.
    pub name: usize,
    /// Tag offset.
    pub tag: usize,
    /// Callback offset.
    pub callback: usize,
}

/// Typed registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclRegistration {
    /// Record RVA.
    pub rva: u64,
    /// Callback name.
    pub name: String,
    /// Save tag.
    pub tag: u32,
    /// Record layout.
    pub layout: DeclRegistrationLayout,
}

/// Native entry with optional registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclEntry {
    /// Entry RVA.
    pub rva: u64,
    /// Typed registration.
    pub registration: Option<DeclRegistration>,
}

/// Native weapon command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclCommand {
    /// Arguments.
    pub arguments: Vec<String>,
    /// Tail text.
    pub tail: String,
}

/// Native weapon cvar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclCvar {
    /// Cvar name.
    pub name: String,
    /// Value.
    pub value: String,
}

/// Entity field contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclEntityFields {
    /// Record byte length.
    pub byte_length: usize,
    /// Origin offset.
    pub origin: usize,
    /// Angles offset.
    pub angles: usize,
    /// Velocity offset.
    pub velocity: usize,
    /// Client offset.
    pub client: usize,
    /// Owner offset.
    pub owner: usize,
    /// View height offset.
    pub view_height: usize,
    /// Generation offset.
    pub generation: usize,
    /// Next think offset.
    pub next_think: usize,
    /// Think callback offset.
    pub think_callback: usize,
    /// Think registration offset.
    pub think_registration: usize,
    /// Touch callback offset.
    pub touch_callback: usize,
}

/// Client field contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclClientFields {
    /// Record byte length.
    pub byte_length: usize,
    /// Weapon offset.
    pub weapon: usize,
    /// View angles offset.
    pub view_angles: usize,
    /// Forward offset.
    pub forward: usize,
}

/// Equipped weapon contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclEquippedWeapon {
    /// Record byte length.
    pub byte_length: usize,
    /// Callback offset.
    pub callback: usize,
    /// Expected entry.
    pub expected: DeclEntry,
}

/// Call chain with a signature tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclCalls {
    /// Signature tag.
    pub signature: String,
    /// Calls.
    pub calls: Vec<DeclEntry>,
}

/// Native weapon behavior declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeWeaponBehaviorDeclaration {
    /// Version.
    pub version: u32,
    /// Kind tag.
    pub kind: String,
    /// ABI tag.
    pub abi: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub artifact_digest: String,
    /// Behavior id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Weapon role.
    pub role: String,
    /// Aspect.
    pub aspect: String,
    /// Entity fields.
    pub entity: DeclEntityFields,
    /// Client fields.
    pub client: DeclClientFields,
    /// Equipped weapon fields.
    pub equipped_weapon: DeclEquippedWeapon,
    /// Time storage tag.
    pub time_storage: String,
    /// Time RVA.
    pub time_rva: u64,
    /// Think signature tag.
    pub think_signature: String,
    /// Think save tag.
    pub think_tag: u32,
    /// Think registration layout.
    pub think_registration: DeclRegistrationLayout,
    /// Allocate signature tag.
    pub allocate_signature: String,
    /// Allocate entry.
    pub allocate: DeclEntry,
    /// Free signature tag.
    pub free_signature: String,
    /// Free entry.
    pub free: DeclEntry,
    /// Projectile touch entry.
    pub projectile_touch: DeclEntry,
    /// Equip chain.
    pub equip: DeclCalls,
    /// Launch chain.
    pub launch: DeclCalls,
    /// Activation RVA.
    pub activate_rva: Option<u64>,
    /// Fire RVA.
    pub fire_rva: u64,
    /// Initialization classes.
    pub initialization_classes: Vec<String>,
    /// Equipment commands.
    pub equipment: Vec<DeclCommand>,
    /// Ammunition command.
    pub ammunition: DeclCommand,
    /// Initial cvars.
    pub initial_cvars: Vec<DeclCvar>,
    /// Provisioning cvars.
    pub provisioning_cvars: Vec<DeclCvar>,
}

struct FieldRecord {
    byte_length: usize,
    used: Vec<(usize, usize)>,
}

impl FieldRecord {
    fn field(&mut self, reader: &DeclReader, name: &str, width: usize) -> Result<usize, DeclarationError> {
        let field = reader.field(name)?;
        let start = uint(&field, 0)? as usize;
        let end = start + width;
        let align = if width == 12 { 4 } else { width.min(8) };
        if end > self.byte_length || start % align != 0 {
            return Err(DeclarationError {
                path: field.path.clone(),
                message: "field exceeds record or violates its storage alignment".to_string(),
            });
        }
        if self
            .used
            .iter()
            .any(|(other_start, other_end)| start < *other_end && *other_start < end)
        {
            return Err(DeclarationError {
                path: field.path.clone(),
                message: "overlapping declared fields".to_string(),
            });
        }
        self.used.push((start, end));
        Ok(start)
    }
}

fn record_fields(reader: &DeclReader) -> Result<FieldRecord, DeclarationError> {
    Ok(FieldRecord {
        byte_length: uint(&reader.field("byteLength")?, 1)? as usize,
        used: Vec::new(),
    })
}

fn read_registration_layout(reader: &DeclReader) -> Result<DeclRegistrationLayout, DeclarationError> {
    reader.keys(&["byteLength", "name", "tag", "callback"])?;
    let mut record = record_fields(reader)?;
    Ok(DeclRegistrationLayout {
        byte_length: record.byte_length,
        name: record.field(reader, "name", 8)?,
        tag: record.field(reader, "tag", 4)?,
        callback: record.field(reader, "callback", 8)?,
    })
}

fn read_entry(reader: &DeclReader) -> Result<DeclEntry, DeclarationError> {
    reader.keys(&["rva", "registration"])?;
    let registered = reader.field("registration")?;
    if !matches!(registered.value(), DeclValue::Null) {
        registered.keys(&["rva", "name", "tag", "layout"])?;
    }
    Ok(DeclEntry {
        rva: uint(&reader.field("rva")?, 1)?,
        registration: registered.nullable(|value| {
            Ok(DeclRegistration {
                rva: uint(&value.field("rva")?, 1)?,
                name: text(&value.field("name")?, false)?,
                tag: uint(&value.field("tag")?, 0)? as u32,
                layout: read_registration_layout(&value.field("layout")?)?,
            })
        })?,
    })
}

fn read_command(reader: &DeclReader) -> Result<DeclCommand, DeclarationError> {
    reader.keys(&["arguments", "tail"])?;
    let arguments = reader.field("arguments")?.list(|value| text(value, true))?;
    if arguments.is_empty() || arguments[0].is_empty() {
        return Err(reader.fail("command requires a name"));
    }
    Ok(DeclCommand {
        arguments,
        tail: text(&reader.field("tail")?, true)?,
    })
}

fn read_cvars(reader: &DeclReader) -> Result<Vec<DeclCvar>, DeclarationError> {
    let mut seen = HashSet::new();
    reader.list(|value| {
        value.keys(&["name", "value"])?;
        let name = text(&value.field("name")?, false)?;
        let valid = name
            .bytes()
            .next()
            .is_some_and(|first| first == b'_' || first.is_ascii_alphabetic())
            && name.bytes().all(|byte| byte == b'_' || byte.is_ascii_alphanumeric());
        let lowered = name.to_lowercase();
        if !valid || !seen.insert(lowered) {
            return Err(value.fail("invalid or duplicate cvar name"));
        }
        Ok(DeclCvar {
            name,
            value: text(&value.field("value")?, true)?,
        })
    })
}

fn read_calls(reader: &DeclReader) -> Result<DeclCalls, DeclarationError> {
    reader.keys(&["signature", "calls"])?;
    let signature = reader.field("signature")?.literal_str("entity-void")?;
    let calls = reader.field("calls")?.list(read_entry)?;
    if calls.is_empty() || calls.len() > 64 {
        return Err(reader.fail("expected 1-64 source calls"));
    }
    Ok(DeclCalls { signature, calls })
}

/// Parse explicit evidence-backed capabilities; this never discovers
/// private layouts or executable offsets.
pub fn read_native_weapon_declaration(
    value: &DeclValue,
    module: Option<(&str, &str)>,
) -> Result<NativeWeaponBehaviorDeclaration, DeclarationError> {
    let reader = DeclReader::new(value, "native-weapon-profile");
    reader.keys(&[
        "version",
        "kind",
        "abi",
        "artifactPath",
        "artifactDigest",
        "id",
        "title",
        "role",
        "aspect",
        "entity",
        "client",
        "equippedWeapon",
        "time",
        "think",
        "allocate",
        "free",
        "projectileTouch",
        "equip",
        "launch",
        "activateRva",
        "fireRva",
        "initializationClasses",
        "equipment",
        "ammunition",
        "initialCvars",
        "provisioningCvars",
    ])?;
    reader.field("entity")?.keys(&[
        "byteLength",
        "origin",
        "angles",
        "velocity",
        "client",
        "owner",
        "viewHeight",
        "generation",
        "nextThink",
        "thinkCallback",
        "thinkRegistration",
        "touchCallback",
    ])?;
    reader
        .field("client")?
        .keys(&["byteLength", "weapon", "viewAngles", "forward"])?;
    reader
        .field("equippedWeapon")?
        .keys(&["byteLength", "callback", "expected"])?;
    reader.field("time")?.keys(&["storage", "rva"])?;
    reader.field("think")?.keys(&["signature", "tag", "registration"])?;
    reader.field("allocate")?.keys(&["signature", "entry"])?;
    reader.field("free")?.keys(&["signature", "entry"])?;
    let artifact_path = normalize_resource_path(&text(&reader.field("artifactPath")?, false)?)?;
    let digest = reader.field("artifactDigest")?.string()?;
    if digest.len() != 71 || !digest.starts_with("sha256:") || !digest[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(reader
            .field("artifactDigest")?
            .fail("expected canonical SHA256 identity"));
    }
    if let Some((module_digest, module_path)) = module {
        if module_digest != digest || module_path != artifact_path {
            return Err(reader.fail("native profile artifact identity differs"));
        }
    }
    let entity_reader = reader.field("entity")?;
    let mut entity = record_fields(&entity_reader)?;
    let client_reader = reader.field("client")?;
    let mut client = record_fields(&client_reader)?;
    let weapon_reader = reader.field("equippedWeapon")?;
    let mut weapon = record_fields(&weapon_reader)?;
    let equip = read_calls(&reader.field("equip")?)?;
    let launch = read_calls(&reader.field("launch")?)?;
    let activate_rva = reader.field("activateRva")?.nullable(|value| uint(value, 1))?;
    let fire_rva = uint(&reader.field("fireRva")?, 1)?;
    if activate_rva.is_some_and(|rva| !equip.calls.iter().any(|call| call.rva == rva)) {
        return Err(reader.fail("activation must identify a declared equip call"));
    }
    if !launch.calls.iter().any(|call| call.rva == fire_rva) {
        return Err(reader.fail("fire must identify a declared launch call"));
    }
    let initialization_classes = reader
        .field("initializationClasses")?
        .list(|value| text(value, false))?;
    let worldspawn = initialization_classes
        .iter()
        .filter(|name| *name == "worldspawn")
        .count();
    let unique: HashSet<&str> = initialization_classes.iter().map(String::as_str).collect();
    if worldspawn != 1 || unique.len() != initialization_classes.len() {
        return Err(reader.fail("initialization classes require one worldspawn and unique classes"));
    }
    let think_reader = reader.field("think")?;
    let allocate_reader = reader.field("allocate")?;
    let free_reader = reader.field("free")?;
    let time_reader = reader.field("time")?;
    let profile = NativeWeaponBehaviorDeclaration {
        version: reader.field("version")?.literal_int(1)? as u32,
        kind: reader.field("kind")?.literal_str("q2-api2023-trajectory")?,
        abi: reader.field("abi")?.literal_str("windows-x86-64")?,
        artifact_path,
        artifact_digest: digest,
        id: reader.field("id")?.namespaced()?,
        title: text(&reader.field("title")?, false)?,
        role: reader
            .field("role")?
            .choice(&["rocket", "grenade", "nail", "bolt", "plasma", "energy", "grapple"])?,
        aspect: reader.field("aspect")?.literal_str("trajectory")?,
        entity: DeclEntityFields {
            byte_length: entity.byte_length,
            origin: entity.field(&entity_reader, "origin", 12)?,
            angles: entity.field(&entity_reader, "angles", 12)?,
            velocity: entity.field(&entity_reader, "velocity", 12)?,
            client: entity.field(&entity_reader, "client", 8)?,
            owner: entity.field(&entity_reader, "owner", 8)?,
            view_height: entity.field(&entity_reader, "viewHeight", 4)?,
            generation: entity.field(&entity_reader, "generation", 4)?,
            next_think: entity.field(&entity_reader, "nextThink", 8)?,
            think_callback: entity.field(&entity_reader, "thinkCallback", 8)?,
            think_registration: entity.field(&entity_reader, "thinkRegistration", 8)?,
            touch_callback: entity.field(&entity_reader, "touchCallback", 8)?,
        },
        client: DeclClientFields {
            byte_length: client.byte_length,
            weapon: client.field(&client_reader, "weapon", 8)?,
            view_angles: client.field(&client_reader, "viewAngles", 12)?,
            forward: client.field(&client_reader, "forward", 12)?,
        },
        equipped_weapon: DeclEquippedWeapon {
            byte_length: weapon.byte_length,
            callback: weapon.field(&weapon_reader, "callback", 8)?,
            expected: read_entry(&weapon_reader.field("expected")?)?,
        },
        time_storage: time_reader.field("storage")?.literal_str("int64-milliseconds")?,
        time_rva: uint(&time_reader.field("rva")?, 1)?,
        think_signature: think_reader.field("signature")?.literal_str("entity-void")?,
        think_tag: uint(&think_reader.field("tag")?, 0)? as u32,
        think_registration: read_registration_layout(&think_reader.field("registration")?)?,
        allocate_signature: allocate_reader.field("signature")?.literal_str("void-pointer")?,
        allocate: read_entry(&allocate_reader.field("entry")?)?,
        free_signature: free_reader.field("signature")?.literal_str("entity-void")?,
        free: read_entry(&free_reader.field("entry")?)?,
        projectile_touch: read_entry(&reader.field("projectileTouch")?)?,
        equip,
        launch,
        activate_rva,
        fire_rva,
        initialization_classes,
        equipment: reader.field("equipment")?.list(read_command)?,
        ammunition: read_command(&reader.field("ammunition")?)?,
        initial_cvars: read_cvars(&reader.field("initialCvars")?)?,
        provisioning_cvars: read_cvars(&reader.field("provisioningCvars")?)?,
    };
    let edict = edict_layout();
    let at = |name: &str| field_offset(&edict, name).unwrap_or(usize::MAX);
    if profile.entity.byte_length < edict.byte_length
        || profile.entity.origin != at("s.origin")
        || profile.entity.angles != at("s.angles")
        || profile.entity.client != at("client")
        || profile.entity.owner != at("owner")
    {
        return Err(reader
            .field("entity")?
            .fail("declared fields differ from the public API2023 edict prefix"));
    }
    Ok(profile)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for byte in text.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\u{byte:04x}")),
        }
    }
    out.push('"');
    out
}

fn entry_json(entry: &DeclEntry) -> String {
    let registration = match &entry.registration {
        None => "null".to_string(),
        Some(registration) => format!(
            "{{\"rva\":{},\"name\":{},\"tag\":{},\"layout\":{{\"byteLength\":{},\"name\":{},\"tag\":{},\"callback\":{}}}}}",
            registration.rva,
            escape(&registration.name),
            registration.tag,
            registration.layout.byte_length,
            registration.layout.name,
            registration.layout.tag,
            registration.layout.callback,
        ),
    };
    format!("{{\"rva\":{},\"registration\":{registration}}}", entry.rva)
}

/// Serialize a declaration canonically through validation.
pub fn serialize_native_weapon_declaration(value: &NativeWeaponBehaviorDeclaration) -> String {
    let commands = |commands: &[DeclCommand]| {
        commands
            .iter()
            .map(|command| {
                format!(
                    "{{\"arguments\":[{}],\"tail\":{}}}",
                    command
                        .arguments
                        .iter()
                        .map(|argument| escape(argument))
                        .collect::<Vec<_>>()
                        .join(","),
                    escape(&command.tail)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    let cvars = |cvars: &[DeclCvar]| {
        cvars
            .iter()
            .map(|cvar| format!("{{\"name\":{},\"value\":{}}}", escape(&cvar.name), escape(&cvar.value)))
            .collect::<Vec<_>>()
            .join(",")
    };
    let calls = |calls: &DeclCalls| {
        format!(
            "{{\"signature\":{},\"calls\":[{}]}}",
            escape(&calls.signature),
            calls.calls.iter().map(entry_json).collect::<Vec<_>>().join(",")
        )
    };
    let entity = &value.entity;
    let client = &value.client;
    format!(
        "{{\"version\":{},\"kind\":{},\"abi\":{},\"artifactPath\":{},\"artifactDigest\":{},\"id\":{},\"title\":{},\"role\":{},\"aspect\":{},\"entity\":{{\"byteLength\":{},\"origin\":{},\"angles\":{},\"velocity\":{},\"client\":{},\"owner\":{},\"viewHeight\":{},\"generation\":{},\"nextThink\":{},\"thinkCallback\":{},\"thinkRegistration\":{},\"touchCallback\":{}}},\"client\":{{\"byteLength\":{},\"weapon\":{},\"viewAngles\":{},\"forward\":{}}},\"equippedWeapon\":{{\"byteLength\":{},\"callback\":{},\"expected\":{}}},\"time\":{{\"storage\":{},\"rva\":{}}},\"think\":{{\"signature\":{},\"tag\":{},\"registration\":{{\"byteLength\":{},\"name\":{},\"tag\":{},\"callback\":{}}}}},\"allocate\":{{\"signature\":{},\"entry\":{}}},\"free\":{{\"signature\":{},\"entry\":{}}},\"projectileTouch\":{},\"equip\":{},\"launch\":{},\"activateRva\":{},\"fireRva\":{},\"initializationClasses\":[{}],\"equipment\":[{}],\"ammunition\":{},\"initialCvars\":[{}],\"provisioningCvars\":[{}]}}",
        value.version,
        escape(&value.kind),
        escape(&value.abi),
        escape(&value.artifact_path),
        escape(&value.artifact_digest),
        escape(&value.id),
        escape(&value.title),
        escape(&value.role),
        escape(&value.aspect),
        entity.byte_length,
        entity.origin,
        entity.angles,
        entity.velocity,
        entity.client,
        entity.owner,
        entity.view_height,
        entity.generation,
        entity.next_think,
        entity.think_callback,
        entity.think_registration,
        entity.touch_callback,
        client.byte_length,
        client.weapon,
        client.view_angles,
        client.forward,
        value.equipped_weapon.byte_length,
        value.equipped_weapon.callback,
        entry_json(&value.equipped_weapon.expected),
        escape(&value.time_storage),
        value.time_rva,
        escape(&value.think_signature),
        value.think_tag,
        value.think_registration.byte_length,
        value.think_registration.name,
        value.think_registration.tag,
        value.think_registration.callback,
        escape(&value.allocate_signature),
        entry_json(&value.allocate),
        escape(&value.free_signature),
        entry_json(&value.free),
        entry_json(&value.projectile_touch),
        calls(&value.equip),
        calls(&value.launch),
        value.activate_rva.map_or("null".to_string(), |rva| rva.to_string()),
        value.fire_rva,
        value.initialization_classes.iter().map(|name| escape(name)).collect::<Vec<_>>().join(","),
        commands(&value.equipment),
        format!(
            "{{\"arguments\":[{}],\"tail\":{}}}",
            value.ammunition.arguments.iter().map(|argument| escape(argument)).collect::<Vec<_>>().join(","),
            escape(&value.ammunition.tail)
        ),
        cvars(&value.initial_cvars),
        cvars(&value.provisioning_cvars),
    )
}

/// Compare two declarations by canonical serialization.
#[must_use]
pub fn same_native_weapon_declaration(
    left: &NativeWeaponBehaviorDeclaration,
    right: &NativeWeaponBehaviorDeclaration,
) -> bool {
    serialize_native_weapon_declaration(left) == serialize_native_weapon_declaration(right)
}

/// PE section mirror for admission checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeSectionMirror {
    /// Section RVA.
    pub rva: u64,
    /// Mapped size.
    pub mapped_size: u64,
    /// Permissions: read, write, execute.
    pub permissions: Vec<String>,
}

/// PE image mirror for admission checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeImageMirror {
    /// ABI tag.
    pub abi: String,
    /// Image size.
    pub image_size: u64,
    /// Sections.
    pub sections: Vec<PeSectionMirror>,
}

/// Catalog admission checks declarations against the actual selected
/// image, before native execution.
pub fn validate_native_weapon_image(
    profile: &NativeWeaponBehaviorDeclaration,
    image: &PeImageMirror,
) -> Result<(), DeclarationError> {
    if image.abi != profile.abi {
        return Err(DeclarationError {
            path: "abi".to_string(),
            message: "Native weapon declaration ABI differs from its PE image".to_string(),
        });
    }
    let range = |rva: u64, size: u64, access: &str| {
        if rva + size > image.image_size
            || !image.sections.iter().any(|section| {
                rva >= section.rva
                    && rva + size <= section.rva + section.mapped_size
                    && section.permissions.iter().any(|permission| permission == access)
            })
        {
            return Err(DeclarationError {
                path: "image".to_string(),
                message: format!("Native weapon {access} range is outside its PE section: {rva}+{size}"),
            });
        }
        Ok(())
    };
    let check = |entry: &DeclEntry| {
        range(entry.rva, 1, "execute")?;
        if let Some(registration) = &entry.registration {
            range(registration.rva, registration.layout.byte_length as u64, "read")?;
        }
        Ok(())
    };
    for entry in profile.equip.calls.iter().chain(profile.launch.calls.iter()).chain([
        &profile.allocate,
        &profile.free,
        &profile.projectile_touch,
        &profile.equipped_weapon.expected,
    ]) {
        check(entry)?;
    }
    range(profile.time_rva, 8, "write")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn str_value(text: &str) -> DeclValue {
        DeclValue::Str(text.to_string())
    }

    fn entry_value(rva: i64) -> DeclValue {
        let mut map = HashMap::new();
        map.insert("rva".to_string(), DeclValue::Int(rva));
        map.insert("registration".to_string(), DeclValue::Null);
        DeclValue::Map(map)
    }

    fn calls_value(rvas: &[i64]) -> DeclValue {
        let mut map = HashMap::new();
        map.insert("signature".to_string(), str_value("entity-void"));
        map.insert(
            "calls".to_string(),
            DeclValue::List(rvas.iter().map(|rva| entry_value(*rva)).collect()),
        );
        DeclValue::Map(map)
    }

    fn declaration_value() -> DeclValue {
        let edict = edict_layout();
        let at = |name: &str| field_offset(&edict, name).expect("offset") as i64;
        let mut entity = HashMap::new();
        entity.insert("byteLength".to_string(), DeclValue::Int(0x7a8));
        entity.insert("origin".to_string(), DeclValue::Int(at("s.origin")));
        entity.insert("angles".to_string(), DeclValue::Int(at("s.angles")));
        entity.insert("velocity".to_string(), DeclValue::Int(0x694));
        entity.insert("client".to_string(), DeclValue::Int(at("client")));
        entity.insert("owner".to_string(), DeclValue::Int(at("owner")));
        entity.insert("viewHeight".to_string(), DeclValue::Int(0x7a0));
        entity.insert("generation".to_string(), DeclValue::Int(0x5c0));
        entity.insert("nextThink".to_string(), DeclValue::Int(0x6d8));
        entity.insert("thinkCallback".to_string(), DeclValue::Int(0x700));
        entity.insert("thinkRegistration".to_string(), DeclValue::Int(0x708));
        entity.insert("touchCallback".to_string(), DeclValue::Int(0x710));
        let mut client = HashMap::new();
        client.insert("byteLength".to_string(), DeclValue::Int(0x19b0));
        client.insert("weapon".to_string(), DeclValue::Int(0xbe8));
        client.insert("viewAngles".to_string(), DeclValue::Int(0x1998));
        client.insert("forward".to_string(), DeclValue::Int(0x19a4));
        let mut equipped = HashMap::new();
        equipped.insert("byteLength".to_string(), DeclValue::Int(0x30));
        equipped.insert("callback".to_string(), DeclValue::Int(0x28));
        equipped.insert("expected".to_string(), entry_value(0xefaf0));
        let mut time = HashMap::new();
        time.insert("storage".to_string(), str_value("int64-milliseconds"));
        time.insert("rva".to_string(), DeclValue::Int(0x2999c8));
        let mut registration = HashMap::new();
        registration.insert("byteLength".to_string(), DeclValue::Int(24));
        registration.insert("name".to_string(), DeclValue::Int(0));
        registration.insert("tag".to_string(), DeclValue::Int(8));
        registration.insert("callback".to_string(), DeclValue::Int(16));
        let mut think = HashMap::new();
        think.insert("signature".to_string(), str_value("entity-void"));
        think.insert("tag".to_string(), DeclValue::Int(20));
        think.insert("registration".to_string(), DeclValue::Map(registration));
        let mut allocate = HashMap::new();
        allocate.insert("signature".to_string(), str_value("void-pointer"));
        allocate.insert("entry".to_string(), entry_value(0x95010));
        let mut free = HashMap::new();
        free.insert("signature".to_string(), str_value("entity-void"));
        free.insert("entry".to_string(), entry_value(0x95140));
        let mut command = HashMap::new();
        command.insert(
            "arguments".to_string(),
            DeclValue::List(vec![str_value("give"), str_value("Rockets")]),
        );
        command.insert("tail".to_string(), str_value("Rockets"));
        let mut map = HashMap::new();
        map.insert("version".to_string(), DeclValue::Int(1));
        map.insert("kind".to_string(), str_value("q2-api2023-trajectory"));
        map.insert("abi".to_string(), str_value("windows-x86-64"));
        map.insert("artifactPath".to_string(), str_value("q2eaks/game.dll"));
        map.insert(
            "artifactDigest".to_string(),
            str_value("sha256:b60b79f7fb6f115218681a9cbab8765267e34f72466975526df05ad288925dde"),
        );
        map.insert("id".to_string(), str_value("native:rocket-trajectory"));
        map.insert("title".to_string(), str_value("Faster rockets"));
        map.insert("role".to_string(), str_value("rocket"));
        map.insert("aspect".to_string(), str_value("trajectory"));
        map.insert("entity".to_string(), DeclValue::Map(entity));
        map.insert("client".to_string(), DeclValue::Map(client));
        map.insert("equippedWeapon".to_string(), DeclValue::Map(equipped));
        map.insert("time".to_string(), DeclValue::Map(time));
        map.insert("think".to_string(), DeclValue::Map(think));
        map.insert("allocate".to_string(), DeclValue::Map(allocate));
        map.insert("free".to_string(), DeclValue::Map(free));
        map.insert("projectileTouch".to_string(), entry_value(0x98060));
        map.insert("equip".to_string(), calls_value(&[0xed4d0]));
        map.insert("launch".to_string(), calls_value(&[0xed420, 0xef900]));
        map.insert("activateRva".to_string(), DeclValue::Int(0xed4d0));
        map.insert("fireRva".to_string(), DeclValue::Int(0xef900));
        map.insert(
            "initializationClasses".to_string(),
            DeclValue::List(vec![str_value("worldspawn")]),
        );
        map.insert(
            "equipment".to_string(),
            DeclValue::List(vec![DeclValue::Map(command.clone())]),
        );
        map.insert("ammunition".to_string(), DeclValue::Map(command));
        map.insert("initialCvars".to_string(), DeclValue::List(vec![]));
        map.insert("provisioningCvars".to_string(), DeclValue::List(vec![]));
        DeclValue::Map(map)
    }

    #[test]
    fn declaration_parses_serializes_and_compares() {
        let value = declaration_value();
        let profile = read_native_weapon_declaration(&value, None).expect("parse");
        assert_eq!(profile.version, 1);
        assert_eq!(profile.entity.byte_length, 0x7a8);
        assert_eq!(profile.fire_rva, 0xef900);
        assert_eq!(profile.activate_rva, Some(0xed4d0));
        let serialized = serialize_native_weapon_declaration(&profile);
        assert!(serialized.contains("\"fireRva\":981248"));
        assert!(same_native_weapon_declaration(&profile, &profile));
        let mut other = profile.clone();
        other.fire_rva = 1;
        assert!(!same_native_weapon_declaration(&profile, &other));
        let bound = read_native_weapon_declaration(
            &value,
            Some((
                "sha256:b60b79f7fb6f115218681a9cbab8765267e34f72466975526df05ad288925dde",
                "q2eaks/game.dll",
            )),
        );
        assert!(bound.is_ok());
        assert!(read_native_weapon_declaration(&value, Some(("sha256:other", "q2eaks/game.dll"))).is_err());
    }

    #[test]
    fn declaration_rejects_bad_fields_and_images() {
        let value = declaration_value();
        if let DeclValue::Map(map) = &value {
            let mut tampered = map.clone();
            tampered.insert("role".to_string(), str_value("nuke"));
            assert!(read_native_weapon_declaration(&DeclValue::Map(tampered), None).is_err());
        }
        if let DeclValue::Map(map) = &value {
            let mut tampered = map.clone();
            tampered.insert("fireRva".to_string(), DeclValue::Int(1));
            assert!(read_native_weapon_declaration(&DeclValue::Map(tampered), None).is_err());
        }
        let profile = read_native_weapon_declaration(&value, None).expect("parse");
        let image = PeImageMirror {
            abi: "windows-x86-64".to_string(),
            image_size: 0x300000,
            sections: vec![
                PeSectionMirror {
                    rva: 0,
                    mapped_size: 0x200000,
                    permissions: vec!["read".to_string(), "execute".to_string()],
                },
                PeSectionMirror {
                    rva: 0x200000,
                    mapped_size: 0x100000,
                    permissions: vec!["read".to_string(), "write".to_string()],
                },
            ],
        };
        validate_native_weapon_image(&profile, &image).expect("image");
        let wrong_abi = PeImageMirror {
            abi: "linux-x86-64".to_string(),
            ..image.clone()
        };
        assert!(validate_native_weapon_image(&profile, &wrong_abi).is_err());
        let tiny = PeImageMirror {
            image_size: 0x1000,
            ..image
        };
        assert!(validate_native_weapon_image(&profile, &tiny).is_err());
    }
}
