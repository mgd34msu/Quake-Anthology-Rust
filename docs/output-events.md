# Shared output delivery and payload ownership

Server owns one core EventRing and its TextStore. The app loads 4,096 record
slots, one presentation consumer, up to 64 client consumers and 32 module
consumers. Text capacity includes the record slots and six independent HUD
slots per client/module, with 8,192 bytes per page. Storage is safe typed,
allocated at load and reused without frame allocation. Formatted text writes
straight into a page; raw module bytes preserve native glyphs and newlines.

Each consumer has its own cursor and fixed delivery state. Reading a record
changes neither ownership nor delivery. A bounded batch visits each pending
record at most once, so an unsent sound does not prevent a later print. Unsent
records remain pending for the next pass. A successful best-effort submission
retires that consumer's interest immediately. Reliable submission retains it
until `acknowledge` receives the matching native receipt. Several records can
share a receipt, corresponding to a native reliable message. Receipt tokens,
consumer generations and output sequences are internal and never serialized.

The protocol adapter selects the submission result using its native delivery
rules. It must validate the real ACK, including fragment/message completion,
before acknowledging the internal receipt; receipt values must not be reused
within a consumer epoch. A transmit watermark is never an ACK. In NetQuake,
unreliable datagrams have no ACK: qsrc WinQuake/net_dgrm.c:370-395 returns them
directly, while :397-423 validates reliable fragment ACKs and :427-431 sends
ACKs for reliable data. QW/Q2/Q2RR/Q3 adapters retain their original framing,
fields and widths. This storage change adds no wire field. Native channels are
still pending, so current receipt fixtures model delivery rather than prove
live legacy interoperability.

Com_Frame runs two physical input drains, before SERVER and CLIENT. Neither
output delivery nor retirement performs physical intake. Modules with an
output callback consume their own cursor at their native provider ticks.
CLIENT dispatches once before presentation, including quit paths. Audio and
particles have one presentation consumer; each bound local client independently
consumes HUD prints. The native remote callback returns Unsent by default while
its channel is unavailable. The old destructive pop/drain API and the separate
Runtime text owner are deleted.

An event slot owns one text lease. Each HUD notify, center or layout message
acquires another lease before consuming the event. Replacing, expiring or
resetting a display releases its lease; disconnect also releases every HUD
lease and unbinds the client's cursor. A page is reusable only when all these
leases release it. No payload cloning, frame reset or slow consumer can
invalidate an active HUD string. Notify/center deadlines come from the shared
cached cvars; layout leases last until replacement or reset.

When the ring fills, only consumers still holding its oldest record enter
resync. Their retained records are cancelled explicitly, their receipt state
is discarded and publication visits only registered consumers and continues for healthy consumers. Counters track
submissions, ACK operations, acknowledged records, overflow episodes, resyncs,
records cancelled by resync and records skipped during resync separately.
Resuming after a completed native resync changes the consumer generation, so
old receipts cannot retire new records. Local presentation/modules resume at
their next consumption phase. A remote peer stays in resync until its native
adapter completes the restart. Rejected text publications are counted and do
not stop the server.

```sh
python3 tools/check_rules.py
cargo test --workspace
cargo clippy --workspace --all-targets --features qa-platform/allocation-tracking -- -D warnings
cargo build --release -p qa-platform --features allocation-tracking --example output_retirement
taskset -c "$CORE" target/release/examples/output_retirement
```

The host fixture runs a 40-Hz world and 10/20/40-Hz module consumers with a
stalled reliable peer, a healthy best-effort peer and one leased local HUD.
It verifies healthy delivery and server ticks continue after bounded overflow,
with no ACK from the stalled peer. Sixty warm-up frames precede 600 pinned
measured frames and an allocation positive control. This is headless delivery
and Rust heap evidence, excluding game audio, native wire ACKs, guest modules,
SDL/driver heap and installed gameplay. Full acceptance stays open on
THE-697/890 and the three-game gate.
