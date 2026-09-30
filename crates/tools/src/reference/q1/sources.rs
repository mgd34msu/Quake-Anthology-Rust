//! Q1 source pins (donor `tools/reference/q1/sources.ts`).
//!
//! Pinned source excerpts backing the Q1 oracle equations, plus the oracle
//! scope limits recorded on every capture.

use crate::json::Json;

/// A pinned source excerpt.
#[derive(Debug, Clone)]
pub struct SourceExcerpt {
    /// First line (1-based, inclusive).
    pub first_line: u32,
    /// Last line (1-based, inclusive).
    pub last_line: u32,
    /// Why the excerpt is pinned.
    pub purpose: String,
}

impl SourceExcerpt {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("firstLine".to_owned(), Json::uint(u64::from(self.first_line))),
            ("lastLine".to_owned(), Json::uint(u64::from(self.last_line))),
            ("purpose".to_owned(), Json::string(&self.purpose)),
        ])
    }
}

/// A pinned source file.
#[derive(Debug, Clone)]
pub struct SourcePin {
    /// Stable pin identifier.
    pub id: String,
    /// Repository-relative path.
    pub path: String,
    /// SHA-256 hex of the pinned bytes.
    pub sha256: String,
    /// Pinned excerpts.
    pub excerpts: Vec<SourceExcerpt>,
}

impl SourcePin {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("path".to_owned(), Json::string(&self.path)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            (
                "excerpts".to_owned(),
                Json::array(self.excerpts.iter().map(SourceExcerpt::to_json).collect()),
            ),
        ])
    }
}

fn pin(id: &str, path: &str, sha256: &str, excerpts: &[Excerpt]) -> SourcePin {
    SourcePin {
        id: id.to_owned(),
        path: path.to_owned(),
        sha256: sha256.to_owned(),
        excerpts: excerpts
            .iter()
            .map(|excerpt| SourceExcerpt {
                first_line: excerpt.first_line,
                last_line: excerpt.last_line,
                purpose: excerpt.purpose.to_owned(),
            })
            .collect(),
    }
}

struct Excerpt {
    first_line: u32,
    last_line: u32,
    purpose: &'static str,
}

/// Pinned Q1 oracle sources.
#[must_use]
pub fn q1_source_pins() -> Vec<SourcePin> {
    vec![
        pin(
            "quake-sv-phys",
            "quake/WinQuake/sv_phys.c",
            "4c65e087e2a86fa6fb1883e0ec050ce0469cb31242f4c2243ac28d974826952e",
            &[Excerpt {
                first_line: 126,
                last_line: 144,
                purpose: "SV_RunThink scheduling, callback entry and removal return",
            }],
        ),
        pin(
            "quake-remove-builtin",
            "quake/WinQuake/pr_cmds.c",
            "24be89ddb3a27ba2bcc90868f6bb884b069490d97d2c84c631844d63bd38b075",
            &[Excerpt {
                first_line: 965,
                last_line: 971,
                purpose: "PF_Remove delegates entity removal to ED_Free",
            }],
        ),
        pin(
            "quake-free-edict",
            "quake/WinQuake/pr_edict.c",
            "90467fdbc99ffb557afae0a828e82c6d2e3d38e7509707fe4a6842578798f28c",
            &[Excerpt {
                first_line: 122,
                last_line: 139,
                purpose: "ED_Free marks free, clears entity fields and stores nextthink=-1",
            }],
        ),
        pin(
            "quake-pr-exec",
            "quake/WinQuake/pr_exec.c",
            "43cc7181ee0a74f43e32045fb77f05767041fa1d94ef69f4cc8532d933340b8b",
            &[
                Excerpt {
                    first_line: 410,
                    last_line: 412,
                    purpose: "OP_ADD_F stores a float after addition",
                },
                Excerpt {
                    first_line: 419,
                    last_line: 421,
                    purpose: "OP_SUB_F stores a float after subtraction",
                },
                Excerpt {
                    first_line: 428,
                    last_line: 430,
                    purpose: "OP_MUL_F stores a float after multiplication",
                },
                Excerpt {
                    first_line: 447,
                    last_line: 457,
                    purpose: "OP_DIV_F and signed int conversions for OP_BITAND and OP_BITOR",
                },
            ],
        ),
        pin(
            "quake-eval-type",
            "quake/WinQuake/progs.h",
            "2ba22bcae0bf4915970876e902064448c4191653968b3a2a2c5bdfade8005596",
            &[Excerpt {
                first_line: 24,
                last_line: 33,
                purpose: "eval_t float storage",
            }],
        ),
        pin(
            "quake-prog-fields",
            "quake/WinQuake/progdefs.q1",
            "e81793ac2f78dc2a461d5de78c65937060ebb272c876bdf12250aec2b673d30c",
            &[
                Excerpt {
                    first_line: 3,
                    last_line: 11,
                    purpose: "QC globals and time storage",
                },
                Excerpt {
                    first_line: 85,
                    last_line: 89,
                    purpose: "think callback and nextthink float field",
                },
            ],
        ),
        pin(
            "quake-host-clock",
            "quake/WinQuake/host.c",
            "5f16220fdd2b34e4d6c1d60a9542a951c9a379b53e030af4a19811533498b5f1",
            &[Excerpt {
                first_line: 40,
                last_line: 43,
                purpose: "host_frametime is double",
            }],
        ),
        pin(
            "quake-server-clock",
            "quake/WinQuake/server.h",
            "a408184fe7a6261953d3b40f53df7020e43702abece1746206d736bc1ca59a5d",
            &[Excerpt {
                first_line: 38,
                last_line: 44,
                purpose: "sv.time is double",
            }],
        ),
        pin(
            "mg1-hub",
            "quake-rerelease-qc/quakec_mg1/map_specific/hub.qc",
            "d105dbb02675b2216ff3bc5d68f04667a73a6832b7ccd13f7f6e58e5640be24a",
            &[Excerpt {
                first_line: 21,
                last_line: 30,
                purpose: "hub exit removes itself unless the complete mask is present",
            }],
        ),
        pin(
            "mg1-sigils",
            "quake-rerelease-qc/quakec_mg1/items_runes.qc",
            "c53b931a1b787cc546f51e15052e0c20aca2991f58e8a5444c724e053ee4d177",
            &[Excerpt {
                first_line: 28,
                last_line: 37,
                purpose: "SIGIL_ALL includes five sigils, value 31; sixth sigil is excluded",
            }],
        ),
        pin(
            "mg3-counter",
            "quake-rerelease-qc/quakec_mg3/mg3_triggers.qc",
            "9be9cf40aabd8fc7e4995462f35b7d647531d0ac765e58022d035951543ebf63",
            &[
                Excerpt {
                    first_line: 66,
                    last_line: 81,
                    purpose: "Count only E1 through E4 and invoke SUB_UseTargets at threshold",
                },
                Excerpt {
                    first_line: 125,
                    last_line: 134,
                    purpose: "Cooperative inhibition, zero-count default and use callback installation",
                },
            ],
        ),
        pin(
            "mg3-defs",
            "quake-rerelease-qc/quakec_mg3/defs.qc",
            "6e6571276449d31a1cf62624e3ba9b11b8341034d9d1c8db20843f1d698c4632",
            &[
                Excerpt {
                    first_line: 838,
                    last_line: 842,
                    purpose: "COOP_ONLY and NOT_IN_COOP masks, inhibition macro",
                },
                Excerpt {
                    first_line: 860,
                    last_line: 864,
                    purpose: "Sigil bit definitions",
                },
            ],
        ),
        pin(
            "mg3-subs",
            "quake-rerelease-qc/quakec_mg3/subs.qc",
            "9bf015c15ce74eff887e5f2546bc3bc055d9c868589dedbac26aa6f6b1fd854d",
            &[
                Excerpt {
                    first_line: 61,
                    last_line: 69,
                    purpose: "RemovedOutsideCoop",
                },
                Excerpt {
                    first_line: 311,
                    last_line: 335,
                    purpose: "SUB_UseTargets can defer actual target callbacks; oracle stops at its invocation",
                },
            ],
        ),
    ]
}

/// Q1 oracle scope limits recorded on every capture.
#[must_use]
pub fn q1_oracle_limits() -> Vec<String> {
    [
        "These are evaluations of source-derived equations in strict TypeScript under Bun, not measured native or retail traces.",
        "No original C engine, retail executable, QuakeC compiler, VM or commercial asset is executed by this capture.",
        "Scalar stores assume IEEE-754 binary32 round-to-nearest ties-to-even and signed 32-bit int conversions within range. Native compiler excess precision, undefined conversions, NaN, infinity, overflow, division by zero, vector reductions and FMA are outside this oracle.",
        "SV_RunThink models one invocation on a live entity. Entity identifiers are symbolic and zero is world. Callback effects are prescribed retain, remove or reschedule operations. Remove follows PF_Remove and ED_Free for free and nextthink; other cleared edict fields, arbitrary QuakeC execution and movement are outside scope.",
        "The mg1 oracle stops at remove(self) or trigger_changelevel() invocation. It does not evaluate trigger initialization or complete a retail hub.",
        "The mg3 oracle models trigger_rune_counter spawn filtering followed by one use call. It records SUB_UseTargets invocation with self and activator; target resolution, delay, messages, killtargets and downstream callbacks remain outside scope.",
        "Rerelease mission code is pinned to the locally available source file hashes. Historical retail builds may differ, including the mg1 fifth-sigil definition.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
