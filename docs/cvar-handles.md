# Cached cvar consumers

Resolve a canonical `CvarHandle` or converted `View` when the consumer loads.
`value`, `integer` and `numeric` read fields cached when settings change.
They do not parse strings or search the name table. Use `generation` to notice
canonical updates, and `view_generation` for conversions with several operands.
Generation stamps come from one monotonic registry counter, so a dependency's
new stamp advances the converted view even when another operand has changed
more often. They are not content fingerprints.

Explicit settings, changed source defaults, alias detail changes and applied
latches advance the affected state. Pending writes leave active generations
unchanged. A repeated canonical setting with the same text and no alias detail
leaves its generation unchanged.

Debug builds count calls inside the name index. The `lookup-tracking` console
feature enables that counter in an optimized development build; the app's
existing `allocation-tracking` feature enables both counters. The normal release
omits the instrumentation. The app resolves `developer` once, resets the lookup
counter at each frame's start, and reports `frame_cvar_lookups` through that
cached handle when `developer` is enabled.

```sh
cargo test -p qa-console --test handles
cargo build --release -p qa-app --features qa-app/allocation-tracking
python3 tools/private_run.py --binary "$QA_DEVELOPMENT_BINARY" --owner-profile "$QA_PROFILE" --evidence "$QA_EVIDENCE/cvar-handles" -- --frames 600 --warmup 60 --startup-hold-ms 1000 +set developer 1
```

Copy the development artifact before rebuilding the normal candidate. Every
window run uses the private harness, which forces X11 with Wayland unset and
captures private audio. The positive-control test makes a real failed name
lookup and checks that the counter increments, then exercises 10,000 cached
canonical and converted reads with zero lookups. Private frame evidence is
currently scoped to the window shell; gameplay consumers must cache their own
handles as they arrive. Neither that result nor unit checks prove map play.
