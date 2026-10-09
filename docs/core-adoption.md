# Core adoption and native acceptance

Checked after d79e5660 on 2026-10-09. The following slices are committed and
pushed, with their matched timings and bounded proof in [frame-times.md](frame-times.md):

| Slice | Change | Removed paths |
| --- | --- | --- |
| THE-656/702 ee5cb1ff | App chooses its client capacity at load; all loader callers moved | One implicit-64 app loader |
| THE-617 a696f10c | Converted cvar names use load-interned NameIds | Two cvar-name string comparisons |
| THE-3165/THE-711 d79e5660 | Renderer uses core vector operators/math | Eleven copied helpers |

The latest workspace has 582 passing all-target tests; the unchanged checker,
tracked Clippy, original-C comparisons and allocation gates pass. This is
bounded evidence for those slices, not a claim that every bypass is absent.
No new primitive-use checker was added.

Current player/HUD ownership is in `session::Server::clients`: one load-sized
array, each row holding core PlayerState and HudState. The host's CLIENT phase
projects every connected client through the shared HudBindings; output dispatch
uses the same HUD and text leases. No stock HUD drawer is implemented yet.
THE-702 still needs its Q1 sbar consumer, and THE-656 still needs installed play.

THE-859's retained candidate receipts cover six CPU/GL three-map split-seat
walks plus two base1 Q1+Q3 runs, with real X repeat and independent keyboard and
pointer phases. They belong to clean normal candidate3b85f115, not current main
or an installed qa-rust. Native gameplay qualification remains open.

The remaining original acceptance gates have these concrete dependencies:

* `compat/src/lib.rs` contains no module host or EngineServices implementation
  (THE-863 and its ABI-host issues). Main constructs FrameHost with an empty
  provider list and explicitly reports gameplay=false and native entity count0.
  THE-884 cannot produce original game phase logs or combined native providers.
* `app/src/host.rs` still calls Server::submit_command directly for local seats.
  `network::PacketReceiver` counts incoming payloads; native channel framing is
  absent (THE-860). THE-885/2869 therefore cannot prove that every local native
  message takes the shared loopback/event/channel route. A second invented local
  wire format would not satisfy legacy protocol requirements.
* `tools/install_qualified_build.py` requires actual gameplay before replacing
  qa-rust. Current candidates cannot meet that gate, so THE-859/887/888 cannot
  complete their installed acceptance. The preview exception does not qualify
  those gates. No install or gate waiver has occurred.

THE-656/702/617/859/884/885/887/888 remain In Progress for their native/installed
criteria; THE-3165 is In Review for its recorded math-adoption criteria. No issue
was set Done. Native host/HUD/channel integration must precede the outstanding
installed proofs; its placement in the core-first order is pending owner direction.

Evidence: `~/.cache/qa-rust/core-order-20261009/adoption-and-gates.json` and
`historical-walk-receipts.json`, plus the per-slice evidence recorded in
frame-times.md. The four current math CPU runs' twelve recorded owned PIDs are
confirmed absent in THE-3165's `cleanup-confirmation.json`. Working-tree state,
remote main and issue/comment writes are checked separately after reporting.
