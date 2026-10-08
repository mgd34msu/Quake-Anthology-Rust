# Shared output dispatch

Runtime loads one core EventRing with 4,096 entries and one TextStore with
4,096 rows of 8,192 bytes. Console prints and gameplay/module producers use
that ring. Formatted messages write directly into a reserved text row without
an intermediate String. Long formatted messages retain a valid UTF-8 prefix;
raw module bytes and native Quake glyphs stay byte data. Generations prevent
recycled text from appearing as another message.

Com_Frame calls session::dispatch_frame once after the second event/command
drain and human command construction, before presentation. Quit paths also
flush queued output once. Console/notify/chat prints reach the console sink;
center/layout/notify state updates only the addressed local client's HUD, or
all local HUDs for a broadcast. Remote wire delivery remains R11. Cached
con_notifytime and cg_centertime handles supply lifetimes, and the frame expires
old HUD messages.

Sound and effect events reach the frame consumer callbacks in ring order.
The current live shell has no loaded mixer or particle backend, so it reports
unhandled sound/effect counts. Headless checks supply consumers and prove
delivery and payload fidelity. They do not prove audible sounds or rendered
particles. THE-863 supplies module service mappings; R3/R10 supply live consumers.

The checker rejects other event vectors/deques, fixed arrays, boxed arrays and
named queues outside core, including imported payload aliases and examples.
This checks known source patterns, not arbitrary Rust type resolution.

```sh
cargo test -p qa-app --test output --all-features
python3 tools/verify_rules.py --evidence "$QA_EVIDENCE/output-rules"
cargo build --release -p qa-platform --example host_frame --features allocation-tracking
timeout 300 taskset -c "$CORE" target/release/examples/host_frame --local --binds --bots --outputs --content "$SCRIPT_PRODUCT"
python3 tools/check_sys_events.py --binary "$QA_CANDIDATE" --owner-profile "$QA_PROFILE" --evidence "$QA_EVIDENCE/output-private" --check-output-drain --commands 'echo output'
```

The probe generates fixed data directly. No input recording or replay is added.
Map/module output and a combined gameplay run remain acceptance work at R3.5.

THE-904 makes the real-input helper wait for initial command execution before
pressing W and require at least 30 forward frames. `--expected-output` checks
an explicit echo marker in a composite bind. Older private bind reports that
accepted one pre-config default-forward frame are superseded in Linear.
