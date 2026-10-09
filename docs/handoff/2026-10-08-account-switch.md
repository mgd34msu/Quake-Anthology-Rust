Resume from this file after the owner's 22:55 account-switch pause. The engine
slice is committed and pushed as `5862373da1e7b685efce748b4773f5a7604c5f08`
(THE-2872/THE-2865). The following documentation commit records this handoff and
its verified measurements; it changes no engine code. The working tree was clean
before these documentation changes. WIP branch `wip/the-2852-2026-10-08` at
`170e7fba142b846884765698662d6f2ccacc5292` preserves two older, unverified
HUD comparison drafts (`tools/probes/hud_value_fixture.rs` and `hud_values.c`)
from the parked stash, byte-for-byte. They were not applied to main and need
reconciliation with current HUD APIs before use. No THE-889 code was changed.
The older StampSet stash is a historical backup of work already integrated on
main; both original local stashes were left intact.

The one platform dispatcher now uses bounded atomic job claims, caller
participation and per-worker wakes. Its barrier keeps borrowed job storage alive
through completion and unwind. The app's separate serial execution loop is
removed. CPU bands use caller plus bands-minus-one background workers. Automatic
selection uses affinity-allowed physical cores and can reduce bands for rows or
mandatory cache size; all bands share 32 MiB. Physical topology is implemented
for this Linux host; unavailable topology uses logical, affinity-aware fallback.
`r_cpuBands` is one generated engine extension, archived and renderer-latched,
with zero meaning auto and 1/2/4/8 fixed. The owner CSV remains unchanged.

Verified locally: checker, all-target workspace tests, warning-denied Clippy,
27 developer-tool tests, 103 checker fixtures (85 rejected, 18 allowed), scoped
worker/caller allocation positive controls, and a 600-frame zero-allocation
dispatch probe. The compiled catalog matches all 1,260 owner rows/26,460 cells
and the 21 extension cells. Original C helpers match 100,080 numeric and 344,475
conversion records. The private fixture copier now includes engine-cvars.csv;
its initial missing-file failure remains in the evidence, followed by a passing
complete-fixture run.

Portable release builds: baseline `39aaea41`, 37.55 seconds; candidate
`5862373d`, 38.05 seconds. Neither enables proof input. Eight pinned physical
cores (4 through 11), no debugger, 60 warm-up and 600 measured frames were used.
The dispatcher has identical job outputs and zero calling/worker Rust heap
counts at 1/2/4/8 execution lanes. The private fixed-scene CPU benchmark has
bit-identical RGBA and inverse depth for e1m1/base1/q3dm1 at all four band counts,
including q3dm1 auto and the baseline rows. Its 17 private runs consumed fresh
copied owner settings, quit normally, preserved candidates/original profiles,
and left none of their 51 recorded owned PIDs running.

q3dm1 auto selected eight bands: draw median 6.250 ms, p99 7.250 ms. Fixed eight
band medians were 6.733 and 6.559 ms; the baseline eight-band medians were 7.036
and 7.221 ms. Mean median decreased 6.77%. Auto is within 10% of the best fixed
row. The matched-eight-band sequence is A/B/A/B, with other workloads between;
it is not an ABBA experiment. This is fixed-scene CPU preparation/raster/
dispatch/barrier work, excluding host simulation, presentation and native heap.
The under-4-ms target is still unmet and q3dm1 speed work remains paused.

Evidence is under the task cache directory
`~/.cache/qa-rust/THE-2872-2865/`: `before/` and `after/` hold binaries, source
archives, build metadata/logs; `dispatch-comparison.json` and raw dispatch rows;
`retail-comparison.json`, raw frame-times/pixels/depth and 17 private receipts;
`cvar-cells/comparison.json`, `cvar-native/comparison.json`;
`rule-fixtures-complete/result.json`; `pause-cleanup.json`; and copied developer
helpers in `scripts/`. The workspace test log is
`~/.cache/qa-rust/THE-2872-workspace-tests.log`. All raw failed/initial attempts
are retained rather than replaced by passing summaries.

Exact next step: run the one pending finished-slice normal-app private
qualification for e1m1/base1/q3dm1, CPU automatic bands and GL, 640x400, 60 warm-up
and 600 measured frames, using `after/qa-rust` and fresh owner-profile copies.
Check normal quit, actual CPU band selection and all-instrumented-thread
allocation gate. Record GL identity and label Mesa llvmpipe software rows.
No such full suite was started after the pause directive. Do not repeat the
fixed-band matrix unless a change/failure warrants it. Update the existing
THE-2872/THE-2865 evidence comments; keep live/installation gates explicit.
Then continue THE-889 per its description and design-refinement comment: one
human/bot intent-to-usercmd builder, scales from rule data and native widths at
wire boundaries. Source inspection found the current table has `duck` but no
`crouch` alias or `holster`, and no centerview registration. Earlier THE-904
configured-hold overclaims remain withdrawn. No THE-889 implementation was
started in this slice.

Installed artifacts are unchanged: qa-rust still has its gameplay-qualified
installation gate; the separate qa-rust-preview remains the earlier `08ad0eb4`
render preview, with its adjacent limitations/run notes. There is no new install
or gameplay/GL-performance claim here. Native Q2 rerelease TGA sky loading
(THE-2890), clean-CI SDL3 provisioning (THE-2893), native guest modules and legacy
network interoperability, installed independent-seat/combined movement and
inline movers, output-page retirement (THE-859/697/890), THE-893 input-player
release blocker and THE-2879 format parity remain open. Preserve the core order;
no releases/version tags, content fingerprints or extra intake points.

The owner's budget rule remains: checker/workspace/allocation checks between
commits; full private three-map CPU/GL qualification only at a finished slice or
preview install. Normal private launches force X11, unset Wayland and use private
captured/dummy audio; stop only recorded owned PIDs. Linear is the source of
truth, issues remain In Progress while live gates are open, and never mark Done.
The account-switch directive explicitly authorizes a resume notice in #quake-rust.
Discussion stays top-level in #quake-discussion with [QA-RUST].
