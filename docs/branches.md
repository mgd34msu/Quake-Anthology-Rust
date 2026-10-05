# Stale branches

Branches below are superseded snapshots from the file-for-file donor-port phase
(Sep 30–Oct 2). Their content already lives on `main` (merged via other
branches, often restructured); the remaining per-branch diffs are stale and
must not be merged — doing so would reintroduce duplicate or outdated
transliterated code that the native verticals replace. Kept as archaeological
reference only. Active work happens on `lane/q1-vertical-succ-*`,
`lane/fix-*`, and `lane/docs-*` branches.

| Branch | Tip | Holds | Still matters? |
|---|---|---|---|
| wip/sim-resume-rest | 232b668 | Simulation-resume leftovers, header neutralization | No — fully merged into main |
| wip/sim-guest | 89ac8c8 | Guest-world union ports (classic/rerelease guest sources, Q2 native-world identity, save.rs) | No — equivalent guest modules on main |
| wip/sim-actor | c0f2ad0 | Actor/monster/player port batch (execution, checkpoints, placement, equipment, grapple, input) | No — content exists on main |
| wip/sim-q3finish | 5b2522e | Q3 guest-runtime + movement-projection ports | No — Q3 runtime modules on main |
| wip/sim-net | d232379 | Networked-simulation ports (Q1 QC, Q2 guest, Q2 rerelease native, weapon behavior) | No — network modules on main |
| wip/sim-qc | 49f9b43 | QuakeC/QVM mod ports (sources, weapon behaviors, messages, QW cvars) | No — QC/QVM modules on main |
| wip/sim-ars | 0434b25 | Arsenal + prediction-runtime ports (commands, ammo regen, step/sequence/state) | No — prediction modules on main |
| wip/sim-q3 | 2d7c3a0 | Early Q3 sim port (command policy, host, player/server state) | No — superseded by later Q3 ports on main |
| lane/impl-content-q3-core | 6c5f850 | Q3 content batch incl. single-file q3_team_arena.rs (15k lines) | No — Team Arena content on main as q3/team_arena/ modules |
| lane/impl-content-q1-resume | 37dd1eb | Q1 integration snapshot (addons + missionpacks barrels, 155 files) | No — Q1 content trees on main |
| lane/impl-content-q1-addons-resume | bf79b07 | Q1 addons batch (CTF, horde, items, barrels) | No — addons modules on main |
| lane/impl-content-q1-addons | 613449b | Earlier Q1 addons batch (monsters, frames, impulses) | No — superseded by the resume branch + main |
| lane/impl-content-q1-missionpacks | 79edda3 | Q1 mission-pack batch (monsters + world, 73 files) | No — mission-pack modules on main |
| lane/impl-content-q1-base-wt2 | c17f29f | Q1 base single-batch snapshot (animation/frames/species/travel) | No — base modules on main |
| lane/impl-content-q1-base | eb3400c | Q1 base batch (rules, creatures, monsters, actions, player) | No — base modules on main |
