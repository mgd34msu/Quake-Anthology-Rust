use qa_core::events::{EventRing, OutputConsumerId, OutputRecord, OutputSubmission, TextStore};
/// Consumers report actual successful submission, an actual native reliable
/// receipt, or no submission. Merely reading a record never retires it.
pub trait EventConsumer {
    fn submit(&mut self, record: OutputRecord, texts: &mut TextStore) -> OutputSubmission;
}
pub fn dispatch_consumer(
    events: &mut EventRing,
    id: OutputConsumerId,
    consumer: &mut impl EventConsumer,
) {
    let Some(mut batch) = events.batch(id) else {
        return;
    };
    while let Some(record) = events.next(&mut batch) {
        let submission = consumer.submit(record, &mut events.texts);
        events.submit(id, record.sequence, submission);
    }
}
