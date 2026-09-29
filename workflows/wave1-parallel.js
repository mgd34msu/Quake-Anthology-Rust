export default async function workflow(host) {
  const schema = {
    type: "object",
    required: ["complete", "files_changed", "tests_added", "tests_passing", "gates_green", "unresolved"],
    properties: {
      complete: { type: "boolean" },
      files_changed: { type: "array", items: { type: "string" } },
      tests_added: { type: "number" },
      tests_passing: { type: "number" },
      gates_green: { type: "boolean" },
      unresolved: { type: "array", items: { type: "string" } }
    }
  };
  const common = "RULES (all mandatory): (1) ZERO DEFERRALS - port every file in your scope fully; no stubs, no unimplemented!/todo!, no future work, nothing left for later. (2) SOURCE BOUNDARY - donor is /home/buzzkill/Projects/quake-typescript; read ONLY its src/, tests/, tools/, docs/, verification/ subtrees. NEVER scan .artifacts/ (270G), dist/, node_modules/. NEVER read qsrc or qfiles trees. (3) Idiomatic Rust: DRY, reuse qa-core, zero-cost abstractions, no TS-isms. (4) TESTS - repo-layout tests with synthetic fixtures mirroring donor behavior for every module. (5) GATES - before finishing, ALL green in your worktree, every command wrapped in timeout 300: cargo fmt --all; cargo clippy --all-targets -- -D warnings; cargo test --workspace. (6) Do NOT modify crates/client/** (owned by another agent in parent). Avoid new dependencies unless essential; prefer std. (7) GIT - start with: git checkout -b lane/ID (your lane id; if branch exists, check it out); commit ALL work at end; leave worktree clean with branch containing everything. Return schema fields honestly.";
  const lanes = [
    { label: "bots-nav", input: "TASK impl-bots-nav. Port donor src/bots/navigation (24 files, AAS reachability/routing/movement) into NEW crate qa-bots at crates/bots (register in workspace Cargo.toml). Read donor imports first to choose deps; must not create a dependency cycle (depend only on lower crates; verify graph direction first). Synthetic-AAS-fixture tests. " + common },
    { label: "undefer-llm", input: "TASK impl-undefer-llm. (a) Port donor src/llm/* (9 files: api, auth, codex, errors, models, request, responses, settings, sse) into NEW crates/app/src/llm/ with injectable fetcher trait (no network in tests); cover SSE parsing, errors, Codex auth, settings. (b) Port src/console/draw.ts, llm.ts, llm-batch.ts into crates/app/src/console/draw.rs, llm.rs, llm_batch.rs wired into the console registry; draw.rs uses qa-client types read-only (do not edit qa-client). Remove the out-of-scope note in console/mod.rs. " + common },
    { label: "platform", input: "TASK impl-platform. Port donor src/platform/* (sdl, sdl-render-context, gl, gl-programs, gl-framebuffers, audio, controller, freetype, freetype-layout, vorbis, theora, ipx, ipx-native, native-libraries, runtime, files/contained, files/writable) into NEW crate qa-platform at crates/platform (register in workspace Cargo.toml). Donor uses bun:ffi dlopen - faithful Rust port is dynamic loading via libloading for SDL/GL/FreeType/Vorbis/Theora with honest Error::Unavailable naming the library when absent; files/ipx-native via std. Unit-test pure logic plus fallback paths; live-lib tests must pass without system libs installed. " + common },
    { label: "net-rest", input: "TASK impl-net-rest. Extend crates/net with donor src/network NOT already ported: (a) codec variants: q1 FitzQuake/RMQ wide plus qw29, q2 R1Q2/Q2Pro/rerelease/KEX; (b) channels/sessions/handshakes/connectionless q1+q2+q3; (c) q2 gtv/mvd/server-write, q3 client/server/netchan/transport/snapshot-store/reliable/rcon/download, q1 discovery/prediction/recording; (d) network/common (transport, reliability, fragments, scheduling, session, socks, ipx, loopback, endpoint); (e) network/services; (f) network/unified. Sockets via std::net only, no tokio. Tests: byte-exact codec fixtures plus loopback-socket sessions. " + common },
    { label: "guest-vm", input: "TASK impl-guest-vm. Extend crates/guest (keep existing modules) with donor src/guest: abi, core (callbacks/contracts/memory/registers), elf, floating-point (binary/sse/x87/trigonometric), pe, runtime, x64 (cpu/decoder/integer-kernel/plan), x86 (cpu/decoder/arithmetic). Then replace NullLogic: read ServerLogic in crates/world/src/server.rs, implement it for your VM guest runner in qa-guest; minimal wiring edits in crates/world plus crates/app startup allowed. Tests: CPU fixtures with exact register/memory assertions, ELF/PE fixtures, ABI round-trips. " + common }
  ];
  const reports = await host.parallel(lanes.map(function (l) {
    return { input: l.input, isolation: true, label: l.label, schema: schema };
  }));
  const summary = reports.map(function (r, i) {
    return { lane: lanes[i].label, ref: r ? r.ref : null, error_kind: r ? r.error_kind : "null-result", data: r ? r.data : null };
  });
  const synthesis = await host.agent({
    input: "Merge-check the 5 lane reports (JSON below). For each lane state complete/incomplete plus branch lane/<id> plus test counts plus gate status, then give the parent a merge order (lanes touching shared files Cargo.toml/Cargo.lock last with conflict notes). Keep data under 3000 bytes. Reports: " + JSON.stringify(summary).slice(0, 6000),
    label: "merge-plan",
    schema: schema
  });
  return { status: "ok", ref: synthesis.ref, text: synthesis.text };
}
