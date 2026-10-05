//! Navigation-visible content references projected from
//! `src/contracts/content.ts` and `src/content/mounts/index.ts`: content
//! and value identities, resource provenance, and the resource-open seam
//! that navigation loading reads through. Full mount ownership lives with
//! the content provider.

use qa_content::contract::ResourceIdentity;

/// Content identity (`family:group:name:variant`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentId {
    /// Opaque identity text.
    pub text: String,
}

impl ContentId {
    /// Wrap identity text.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string() }
    }
}

/// Provenance of an opened resource: navigation checks only the owning
/// mount's content identity for checksumless NAV assets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceProvenance {
    /// Owning mount's content identity.
    pub mount_content: ContentId,
}

/// Resolved reference to opened bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceReference {
    /// Requested resource path.
    pub requested_path: String,
    /// Resource provenance.
    pub provenance: ResourceProvenance,
    /// Byte value identity.
    pub identity: ResourceIdentity,
    /// Byte length.
    pub byte_length: usize,
}

/// Opened resource bytes with their resolved reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedResource {
    /// Resolved reference.
    pub reference: ResourceReference,
    /// Resource bytes.
    pub bytes: Vec<u8>,
}

/// Mounted-geometry resource store. Navigation opens `maps/*.aas` and
/// `bots/navigation/*.nav` through this seam.
pub trait NavigationResources {
    /// Open a resource path, or return `None` when absent.
    fn open(&self, path: &str) -> Option<OpenedResource>;
}
