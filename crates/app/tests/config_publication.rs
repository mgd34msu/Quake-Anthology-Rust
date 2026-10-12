use qa_app::client_resources::{ConfigPublication, ResourceReadiness};
use qa_core::primitives::ModuleId;

#[test]
fn resource_readiness_distinguishes_pending_ack_from_an_unsupported_row() {
    let mut publication = ConfigPublication::load();
    assert!(publication.begin(ModuleId(1), 1));
    assert_eq!(
        publication.readiness(ModuleId(1), 1),
        ResourceReadiness::Pending
    );
    publication.published(1, 1, Some(7));
    publication.acknowledged(ModuleId(1), 1, 6);
    assert_eq!(
        publication.readiness(ModuleId(1), 1),
        ResourceReadiness::Pending
    );
    publication.acknowledged(ModuleId(1), 1, 7);
    assert_eq!(
        publication.readiness(ModuleId(1), 1),
        ResourceReadiness::Ready
    );
    publication.published(1, 2, Some(8));
    publication.acknowledged(ModuleId(1), 1, 7);
    assert_eq!(
        publication.readiness(ModuleId(1), 1),
        ResourceReadiness::Pending
    );
    publication.published(1, 3, None);
    assert_eq!(
        publication.readiness(ModuleId(1), 1),
        ResourceReadiness::Unsupported
    );
    publication.acknowledged(ModuleId(1), 1, 8);
    assert_eq!(
        publication.readiness(ModuleId(1), 1),
        ResourceReadiness::Unsupported
    );
    publication.begin(ModuleId(2), 1);
    publication.published(1, 1, Some(9));
    publication.acknowledged(ModuleId(1), 1, 9);
    assert_eq!(
        publication.readiness(ModuleId(2), 1),
        ResourceReadiness::Pending
    );
    assert_eq!(
        publication.readiness(ModuleId(1), 1),
        ResourceReadiness::Unsupported
    );
}
