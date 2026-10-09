# Body attachments

THE-650 uses one `BodyAttachment` value in the entity SoA, with an internal
generation handle for its anchor. `EntityTable::attach` validates both lifetimes,
finite offsets and cycle freedom. Updating a binding retains insertion order;
detaching and reattaching moves it to the end. Entity release, client reset and
QuakeWorld displacement remove the retiring body's binding and detach its direct
children. A child's own children continue following that still-live child.

`AreaGrid::transport_attachments` walks bindings in insertion order and resolves
each chain parent first. Its traversal vector and the shared core `StampSet`
are allocated at load. No recursion, raw arena or separate body registry is used.
The three follow modes match the proven C port's `src/world/body.c`:

| Mode | Local anchor offset |
| --- | --- |
| Translation | Supplied offset |
| Center | `(mins + maxs) * 0.5`; ignores supplied offset |
| BoundsMin | `mins + supplied offset` |

Only the child's position changes. Velocity, angles and local bounds survive.
Signed zero changes count as a changed position. A moved, already-linked child
uses the area grid's existing flags and head/tail insertion rule for an explicit
relink. An unlinked child stays unlinked. Unchanged positions do not relink.
Nonfinite computed positions reject only that transport and increment its counter;
other bindings continue. A mismatched load capacity rejects that batch without
growing scratch storage.

The shared SERVER movement commit transports the graph after authoritative body
updates, then synchronizes attached local, remote and bot client body state before
snapshot copying. Native providers can use the same commit entry at their own
body-commit phase. Attachment handles never become protocol entity numbers:
legacy adapters publish the resulting native pose using their existing fields.
Native module binding, touch callbacks, live mover integration and prediction of
an actively followed client remain open. This slice supplies no client attachment
prediction algorithm and does not establish a universal native physics phase.

Developer comparison:

```sh
cargo build --release -p qa-platform --example body_attachments \
  --features allocation-tracking
python3 tools/compare_attachments.py --c-port "$C_REFERENCE" \
  --binary target/release/examples/body_attachments --evidence "$EVIDENCE"
timeout 300 taskset -c "$PINNED_CPU" \
  target/release/examples/body_attachments 8192
```

The comparison extracts unchanged attachment and transport functions from the
C reference into a developer helper. Body access and link counting are stubs;
the comparison covers final state bits and redundant-link behavior, not native
field access or touch callbacks. The release probe uses a reverse-inserted chain,
all three modes, mixed native insertion rules and 8,192 link/unlink cycles per
frame. Its allocation gate includes stable second transports, detach/reattach and
fidelity checks; its reported stage times cover transport and link cycles only.
See [frame times](frame-times.md) for the measured scope and limits.
