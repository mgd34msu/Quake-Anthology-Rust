//! Hipnotic hiparma frames (src/content/q1/missionpacks/monsters/tables/hiparma.ts).
//!
//! quakec_hipnotic/hiparma.qc source frame order.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::base::animation::{MonsterFrame, MonsterOperation};

static OPS_ARMAGON_DIE1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_die14",
}];
static OPS_ARMAGON_DIE2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_die4",
}];
static OPS_ARMAGON_DIE5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_DIE8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_die8",
}];
static OPS_ARMAGON_DIE9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_OVERLEFT1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overleft1",
}];
static OPS_ARMAGON_OVERLEFT10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overleft5",
}];
static OPS_ARMAGON_OVERLEFT11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overleft11",
}];
static OPS_ARMAGON_OVERLEFT12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overleft12",
}];
static OPS_ARMAGON_OVERLEFT13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overleft_think",
}];
static OPS_ARMAGON_OVERLEFT14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overleft_think",
}];
static OPS_ARMAGON_OVERLEFT15: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "armagon_overleft_think",
    },
    MonsterOperation::Action {
        name: "SUB_AttackFinished(1.0)",
    },
];
static OPS_ARMAGON_OVERLEFT2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overleft_think",
}];
static OPS_ARMAGON_OVERLEFT3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overleft3",
}];
static OPS_ARMAGON_OVERLEFT4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overleft_think",
}];
static OPS_ARMAGON_OVERLEFT5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overleft5",
}];
static OPS_ARMAGON_OVERLEFT6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overleft_think",
}];
static OPS_ARMAGON_OVERLEFT7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overleft_think",
}];
static OPS_ARMAGON_OVERLEFT8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overleft3",
}];
static OPS_ARMAGON_OVERLEFT9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overleft_think",
}];
static OPS_ARMAGON_OVERRIGHT1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overright1",
}];
static OPS_ARMAGON_OVERRIGHT10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overright10",
}];
static OPS_ARMAGON_OVERRIGHT11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overright_think",
}];
static OPS_ARMAGON_OVERRIGHT12: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "armagon_overright_think",
    },
    MonsterOperation::Action {
        name: "SUB_AttackFinished(1.0)",
    },
];
static OPS_ARMAGON_OVERRIGHT2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overright_think",
}];
static OPS_ARMAGON_OVERRIGHT3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overright3",
}];
static OPS_ARMAGON_OVERRIGHT4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overright_think",
}];
static OPS_ARMAGON_OVERRIGHT5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overright5",
}];
static OPS_ARMAGON_OVERRIGHT6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overright6",
}];
static OPS_ARMAGON_OVERRIGHT7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overright_think",
}];
static OPS_ARMAGON_OVERRIGHT8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_overright3",
}];
static OPS_ARMAGON_OVERRIGHT9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "armagon_overright_think",
}];
static OPS_ARMAGON_PLANT1: &[MonsterOperation] = &[
    MonsterOperation::Action {
        name: "armagon_stand_attack",
    },
    MonsterOperation::Action { name: "armagon_think" },
];
static OPS_ARMAGON_RUN1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run1",
}];
static OPS_ARMAGON_RUN10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run1",
}];
static OPS_ARMAGON_RUN11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run5",
}];
static OPS_ARMAGON_RUN12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run12",
}];
static OPS_ARMAGON_RUN2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run1",
}];
static OPS_ARMAGON_RUN3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run3",
}];
static OPS_ARMAGON_RUN4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run1",
}];
static OPS_ARMAGON_RUN5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run5",
}];
static OPS_ARMAGON_RUN6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run1",
}];
static OPS_ARMAGON_RUN7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run1",
}];
static OPS_ARMAGON_RUN8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run1",
}];
static OPS_ARMAGON_RUN9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_run3",
}];
static OPS_ARMAGON_SATK1: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK10: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_satk9",
}];
static OPS_ARMAGON_SATK12: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_satk9",
}];
static OPS_ARMAGON_SATK14: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK15: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK16: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "armagon_think" },
    MonsterOperation::Action {
        name: "SUB_AttackFinished(0.3)",
    },
];
static OPS_ARMAGON_SATK2: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK3: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK4: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK5: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_satk6",
}];
static OPS_ARMAGON_SATK7: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK8: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SATK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_satk9",
}];
static OPS_ARMAGON_SLASER1: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER10: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER11: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "armagon_think" },
    MonsterOperation::Action {
        name: "armagon_launch_laser(40)",
    },
    MonsterOperation::Action {
        name: "armagon_launch_laser(-40)",
    },
];
static OPS_ARMAGON_SLASER12: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER13: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "armagon_think" },
    MonsterOperation::Action {
        name: "armagon_launch_laser(40)",
    },
    MonsterOperation::Action {
        name: "armagon_launch_laser(-40)",
    },
];
static OPS_ARMAGON_SLASER14: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER15: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "armagon_think" },
    MonsterOperation::Action {
        name: "armagon_launch_laser(40)",
    },
    MonsterOperation::Action {
        name: "armagon_launch_laser(-40)",
    },
];
static OPS_ARMAGON_SLASER16: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER17: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "armagon_think" },
    MonsterOperation::Action {
        name: "armagon_launch_laser(40)",
    },
    MonsterOperation::Action {
        name: "armagon_launch_laser(-40)",
    },
];
static OPS_ARMAGON_SLASER18: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER19: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER2: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER20: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "armagon_think" },
    MonsterOperation::Action {
        name: "SUB_AttackFinished(0.3)",
    },
];
static OPS_ARMAGON_SLASER3: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER4: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER5: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_satk6",
}];
static OPS_ARMAGON_SLASER7: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER8: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_SLASER9: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "armagon_think" },
    MonsterOperation::Action {
        name: "armagon_launch_laser(40)",
    },
    MonsterOperation::Action {
        name: "armagon_launch_laser(-40)",
    },
];
static OPS_ARMAGON_STAND1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand1",
}];
static OPS_ARMAGON_STAND10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand1",
}];
static OPS_ARMAGON_STAND11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND12: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND14: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND15: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND16: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND17: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND18: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND19: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND20: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand1",
}];
static OPS_ARMAGON_STAND3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STAND9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stand2",
}];
static OPS_ARMAGON_STOP1: &[MonsterOperation] = &[MonsterOperation::Action { name: "armagon_think" }];
static OPS_ARMAGON_STOP2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_stop2",
}];
static OPS_ARMAGON_WALK1: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "movetogoal(14)" },
    MonsterOperation::Action {
        name: "armagon_walkthink",
    },
];
static OPS_ARMAGON_WALK10: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "movetogoal(14)" },
    MonsterOperation::Action {
        name: "armagon_walkthink",
    },
];
static OPS_ARMAGON_WALK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_walk5",
}];
static OPS_ARMAGON_WALK12: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "movetogoal(14)" },
    MonsterOperation::Action {
        name: "armagon_walkthink",
    },
];
static OPS_ARMAGON_WALK2: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "movetogoal(14)" },
    MonsterOperation::Action {
        name: "armagon_walkthink",
    },
];
static OPS_ARMAGON_WALK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_walk3",
}];
static OPS_ARMAGON_WALK4: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "movetogoal(14)" },
    MonsterOperation::Action {
        name: "armagon_walkthink",
    },
];
static OPS_ARMAGON_WALK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_walk5",
}];
static OPS_ARMAGON_WALK6: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "movetogoal(14)" },
    MonsterOperation::Action {
        name: "armagon_walkthink",
    },
];
static OPS_ARMAGON_WALK7: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "movetogoal(14)" },
    MonsterOperation::Action {
        name: "armagon_walkthink",
    },
];
static OPS_ARMAGON_WALK8: &[MonsterOperation] = &[
    MonsterOperation::Action { name: "movetogoal(14)" },
    MonsterOperation::Action {
        name: "armagon_walkthink",
    },
];
static OPS_ARMAGON_WALK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_walk3",
}];
static OPS_ARMAGON_WATK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WATK10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk4",
}];
static OPS_ARMAGON_WATK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk11",
}];
static OPS_ARMAGON_WATK13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk13",
}];
static OPS_ARMAGON_WATK2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk2",
}];
static OPS_ARMAGON_WATK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WATK4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk4",
}];
static OPS_ARMAGON_WATK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WATK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk6",
}];
static OPS_ARMAGON_WATK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WATK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk2",
}];
static OPS_ARMAGON_WATK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WLASERATK1: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WLASERATK10: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk4",
}];
static OPS_ARMAGON_WLASERATK11: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_wlaseratk11",
}];
static OPS_ARMAGON_WLASERATK13: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk13",
}];
static OPS_ARMAGON_WLASERATK2: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk2",
}];
static OPS_ARMAGON_WLASERATK3: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WLASERATK4: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk4",
}];
static OPS_ARMAGON_WLASERATK5: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WLASERATK6: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_wlaseratk6",
}];
static OPS_ARMAGON_WLASERATK7: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];
static OPS_ARMAGON_WLASERATK8: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk2",
}];
static OPS_ARMAGON_WLASERATK9: &[MonsterOperation] = &[MonsterOperation::Action {
    name: "hiparma:armagon_watk1",
}];

pub static FRAMES: &[(&str, MonsterFrame)] = &[
    (
        "armagon_die1",
        MonsterFrame {
            frame: 84,
            next: "armagon_die2",
            operations: OPS_ARMAGON_DIE1,
        },
    ),
    (
        "armagon_die10",
        MonsterFrame {
            frame: 93,
            next: "armagon_die11",
            operations: OPS_ARMAGON_DIE10,
        },
    ),
    (
        "armagon_die11",
        MonsterFrame {
            frame: 94,
            next: "armagon_die12",
            operations: OPS_ARMAGON_DIE11,
        },
    ),
    (
        "armagon_die12",
        MonsterFrame {
            frame: 95,
            next: "armagon_die13",
            operations: OPS_ARMAGON_DIE12,
        },
    ),
    (
        "armagon_die13",
        MonsterFrame {
            frame: 96,
            next: "armagon_die14",
            operations: OPS_ARMAGON_DIE13,
        },
    ),
    (
        "armagon_die14",
        MonsterFrame {
            frame: 97,
            next: "armagon_die14",
            operations: OPS_ARMAGON_DIE14,
        },
    ),
    (
        "armagon_die2",
        MonsterFrame {
            frame: 85,
            next: "armagon_die3",
            operations: OPS_ARMAGON_DIE2,
        },
    ),
    (
        "armagon_die3",
        MonsterFrame {
            frame: 86,
            next: "armagon_die4",
            operations: OPS_ARMAGON_DIE3,
        },
    ),
    (
        "armagon_die4",
        MonsterFrame {
            frame: 87,
            next: "armagon_die5",
            operations: OPS_ARMAGON_DIE4,
        },
    ),
    (
        "armagon_die5",
        MonsterFrame {
            frame: 88,
            next: "armagon_die6",
            operations: OPS_ARMAGON_DIE5,
        },
    ),
    (
        "armagon_die6",
        MonsterFrame {
            frame: 89,
            next: "armagon_die7",
            operations: OPS_ARMAGON_DIE6,
        },
    ),
    (
        "armagon_die7",
        MonsterFrame {
            frame: 90,
            next: "armagon_die8",
            operations: OPS_ARMAGON_DIE7,
        },
    ),
    (
        "armagon_die8",
        MonsterFrame {
            frame: 91,
            next: "armagon_die9",
            operations: OPS_ARMAGON_DIE8,
        },
    ),
    (
        "armagon_die9",
        MonsterFrame {
            frame: 92,
            next: "armagon_die10",
            operations: OPS_ARMAGON_DIE9,
        },
    ),
    (
        "armagon_overleft1",
        MonsterFrame {
            frame: 25,
            next: "armagon_overleft2",
            operations: OPS_ARMAGON_OVERLEFT1,
        },
    ),
    (
        "armagon_overleft10",
        MonsterFrame {
            frame: 34,
            next: "armagon_overleft11",
            operations: OPS_ARMAGON_OVERLEFT10,
        },
    ),
    (
        "armagon_overleft11",
        MonsterFrame {
            frame: 35,
            next: "armagon_overleft12",
            operations: OPS_ARMAGON_OVERLEFT11,
        },
    ),
    (
        "armagon_overleft12",
        MonsterFrame {
            frame: 36,
            next: "armagon_overleft13",
            operations: OPS_ARMAGON_OVERLEFT12,
        },
    ),
    (
        "armagon_overleft13",
        MonsterFrame {
            frame: 37,
            next: "armagon_overleft14",
            operations: OPS_ARMAGON_OVERLEFT13,
        },
    ),
    (
        "armagon_overleft14",
        MonsterFrame {
            frame: 38,
            next: "armagon_overleft15",
            operations: OPS_ARMAGON_OVERLEFT14,
        },
    ),
    (
        "armagon_overleft15",
        MonsterFrame {
            frame: 39,
            next: "armagon_run1",
            operations: OPS_ARMAGON_OVERLEFT15,
        },
    ),
    (
        "armagon_overleft2",
        MonsterFrame {
            frame: 26,
            next: "armagon_overleft3",
            operations: OPS_ARMAGON_OVERLEFT2,
        },
    ),
    (
        "armagon_overleft3",
        MonsterFrame {
            frame: 27,
            next: "armagon_overleft4",
            operations: OPS_ARMAGON_OVERLEFT3,
        },
    ),
    (
        "armagon_overleft4",
        MonsterFrame {
            frame: 28,
            next: "armagon_overleft5",
            operations: OPS_ARMAGON_OVERLEFT4,
        },
    ),
    (
        "armagon_overleft5",
        MonsterFrame {
            frame: 29,
            next: "armagon_overleft6",
            operations: OPS_ARMAGON_OVERLEFT5,
        },
    ),
    (
        "armagon_overleft6",
        MonsterFrame {
            frame: 30,
            next: "armagon_overleft7",
            operations: OPS_ARMAGON_OVERLEFT6,
        },
    ),
    (
        "armagon_overleft7",
        MonsterFrame {
            frame: 31,
            next: "armagon_overleft8",
            operations: OPS_ARMAGON_OVERLEFT7,
        },
    ),
    (
        "armagon_overleft8",
        MonsterFrame {
            frame: 32,
            next: "armagon_overleft9",
            operations: OPS_ARMAGON_OVERLEFT8,
        },
    ),
    (
        "armagon_overleft9",
        MonsterFrame {
            frame: 33,
            next: "armagon_overleft10",
            operations: OPS_ARMAGON_OVERLEFT9,
        },
    ),
    (
        "armagon_overright1",
        MonsterFrame {
            frame: 40,
            next: "armagon_overright2",
            operations: OPS_ARMAGON_OVERRIGHT1,
        },
    ),
    (
        "armagon_overright10",
        MonsterFrame {
            frame: 49,
            next: "armagon_overright11",
            operations: OPS_ARMAGON_OVERRIGHT10,
        },
    ),
    (
        "armagon_overright11",
        MonsterFrame {
            frame: 50,
            next: "armagon_overright12",
            operations: OPS_ARMAGON_OVERRIGHT11,
        },
    ),
    (
        "armagon_overright12",
        MonsterFrame {
            frame: 51,
            next: "armagon_run1",
            operations: OPS_ARMAGON_OVERRIGHT12,
        },
    ),
    (
        "armagon_overright2",
        MonsterFrame {
            frame: 41,
            next: "armagon_overright3",
            operations: OPS_ARMAGON_OVERRIGHT2,
        },
    ),
    (
        "armagon_overright3",
        MonsterFrame {
            frame: 42,
            next: "armagon_overright4",
            operations: OPS_ARMAGON_OVERRIGHT3,
        },
    ),
    (
        "armagon_overright4",
        MonsterFrame {
            frame: 43,
            next: "armagon_overright5",
            operations: OPS_ARMAGON_OVERRIGHT4,
        },
    ),
    (
        "armagon_overright5",
        MonsterFrame {
            frame: 44,
            next: "armagon_overright6",
            operations: OPS_ARMAGON_OVERRIGHT5,
        },
    ),
    (
        "armagon_overright6",
        MonsterFrame {
            frame: 45,
            next: "armagon_overright7",
            operations: OPS_ARMAGON_OVERRIGHT6,
        },
    ),
    (
        "armagon_overright7",
        MonsterFrame {
            frame: 46,
            next: "armagon_overright8",
            operations: OPS_ARMAGON_OVERRIGHT7,
        },
    ),
    (
        "armagon_overright8",
        MonsterFrame {
            frame: 47,
            next: "armagon_overright9",
            operations: OPS_ARMAGON_OVERRIGHT8,
        },
    ),
    (
        "armagon_overright9",
        MonsterFrame {
            frame: 48,
            next: "armagon_overright10",
            operations: OPS_ARMAGON_OVERRIGHT9,
        },
    ),
    (
        "armagon_plant1",
        MonsterFrame {
            frame: 64,
            next: "armagon_plant1",
            operations: OPS_ARMAGON_PLANT1,
        },
    ),
    (
        "armagon_run1",
        MonsterFrame {
            frame: 0,
            next: "armagon_run2",
            operations: OPS_ARMAGON_RUN1,
        },
    ),
    (
        "armagon_run10",
        MonsterFrame {
            frame: 9,
            next: "armagon_run11",
            operations: OPS_ARMAGON_RUN10,
        },
    ),
    (
        "armagon_run11",
        MonsterFrame {
            frame: 10,
            next: "armagon_run12",
            operations: OPS_ARMAGON_RUN11,
        },
    ),
    (
        "armagon_run12",
        MonsterFrame {
            frame: 11,
            next: "armagon_run1",
            operations: OPS_ARMAGON_RUN12,
        },
    ),
    (
        "armagon_run2",
        MonsterFrame {
            frame: 1,
            next: "armagon_run3",
            operations: OPS_ARMAGON_RUN2,
        },
    ),
    (
        "armagon_run3",
        MonsterFrame {
            frame: 2,
            next: "armagon_run4",
            operations: OPS_ARMAGON_RUN3,
        },
    ),
    (
        "armagon_run4",
        MonsterFrame {
            frame: 3,
            next: "armagon_run5",
            operations: OPS_ARMAGON_RUN4,
        },
    ),
    (
        "armagon_run5",
        MonsterFrame {
            frame: 4,
            next: "armagon_run6",
            operations: OPS_ARMAGON_RUN5,
        },
    ),
    (
        "armagon_run6",
        MonsterFrame {
            frame: 5,
            next: "armagon_run7",
            operations: OPS_ARMAGON_RUN6,
        },
    ),
    (
        "armagon_run7",
        MonsterFrame {
            frame: 6,
            next: "armagon_run8",
            operations: OPS_ARMAGON_RUN7,
        },
    ),
    (
        "armagon_run8",
        MonsterFrame {
            frame: 7,
            next: "armagon_run9",
            operations: OPS_ARMAGON_RUN8,
        },
    ),
    (
        "armagon_run9",
        MonsterFrame {
            frame: 8,
            next: "armagon_run10",
            operations: OPS_ARMAGON_RUN9,
        },
    ),
    (
        "armagon_satk1",
        MonsterFrame {
            frame: 52,
            next: "armagon_satk2",
            operations: OPS_ARMAGON_SATK1,
        },
    ),
    (
        "armagon_satk10",
        MonsterFrame {
            frame: 61,
            next: "armagon_satk11",
            operations: OPS_ARMAGON_SATK10,
        },
    ),
    (
        "armagon_satk11",
        MonsterFrame {
            frame: 60,
            next: "armagon_satk12",
            operations: OPS_ARMAGON_SATK11,
        },
    ),
    (
        "armagon_satk12",
        MonsterFrame {
            frame: 61,
            next: "armagon_satk13",
            operations: OPS_ARMAGON_SATK12,
        },
    ),
    (
        "armagon_satk13",
        MonsterFrame {
            frame: 60,
            next: "armagon_satk14",
            operations: OPS_ARMAGON_SATK13,
        },
    ),
    (
        "armagon_satk14",
        MonsterFrame {
            frame: 61,
            next: "armagon_satk15",
            operations: OPS_ARMAGON_SATK14,
        },
    ),
    (
        "armagon_satk15",
        MonsterFrame {
            frame: 62,
            next: "armagon_satk16",
            operations: OPS_ARMAGON_SATK15,
        },
    ),
    (
        "armagon_satk16",
        MonsterFrame {
            frame: 63,
            next: "armagon_plant1",
            operations: OPS_ARMAGON_SATK16,
        },
    ),
    (
        "armagon_satk2",
        MonsterFrame {
            frame: 53,
            next: "armagon_satk3",
            operations: OPS_ARMAGON_SATK2,
        },
    ),
    (
        "armagon_satk3",
        MonsterFrame {
            frame: 54,
            next: "armagon_satk4",
            operations: OPS_ARMAGON_SATK3,
        },
    ),
    (
        "armagon_satk4",
        MonsterFrame {
            frame: 55,
            next: "armagon_satk5",
            operations: OPS_ARMAGON_SATK4,
        },
    ),
    (
        "armagon_satk5",
        MonsterFrame {
            frame: 56,
            next: "armagon_satk6",
            operations: OPS_ARMAGON_SATK5,
        },
    ),
    (
        "armagon_satk6",
        MonsterFrame {
            frame: 57,
            next: "armagon_satk7",
            operations: OPS_ARMAGON_SATK6,
        },
    ),
    (
        "armagon_satk7",
        MonsterFrame {
            frame: 58,
            next: "armagon_satk8",
            operations: OPS_ARMAGON_SATK7,
        },
    ),
    (
        "armagon_satk8",
        MonsterFrame {
            frame: 59,
            next: "armagon_satk9",
            operations: OPS_ARMAGON_SATK8,
        },
    ),
    (
        "armagon_satk9",
        MonsterFrame {
            frame: 60,
            next: "armagon_satk10",
            operations: OPS_ARMAGON_SATK9,
        },
    ),
    (
        "armagon_slaser1",
        MonsterFrame {
            frame: 52,
            next: "armagon_slaser2",
            operations: OPS_ARMAGON_SLASER1,
        },
    ),
    (
        "armagon_slaser10",
        MonsterFrame {
            frame: 61,
            next: "armagon_slaser11",
            operations: OPS_ARMAGON_SLASER10,
        },
    ),
    (
        "armagon_slaser11",
        MonsterFrame {
            frame: 60,
            next: "armagon_slaser12",
            operations: OPS_ARMAGON_SLASER11,
        },
    ),
    (
        "armagon_slaser12",
        MonsterFrame {
            frame: 61,
            next: "armagon_slaser13",
            operations: OPS_ARMAGON_SLASER12,
        },
    ),
    (
        "armagon_slaser13",
        MonsterFrame {
            frame: 60,
            next: "armagon_slaser14",
            operations: OPS_ARMAGON_SLASER13,
        },
    ),
    (
        "armagon_slaser14",
        MonsterFrame {
            frame: 61,
            next: "armagon_slaser15",
            operations: OPS_ARMAGON_SLASER14,
        },
    ),
    (
        "armagon_slaser15",
        MonsterFrame {
            frame: 60,
            next: "armagon_slaser16",
            operations: OPS_ARMAGON_SLASER15,
        },
    ),
    (
        "armagon_slaser16",
        MonsterFrame {
            frame: 61,
            next: "armagon_slaser17",
            operations: OPS_ARMAGON_SLASER16,
        },
    ),
    (
        "armagon_slaser17",
        MonsterFrame {
            frame: 60,
            next: "armagon_slaser18",
            operations: OPS_ARMAGON_SLASER17,
        },
    ),
    (
        "armagon_slaser18",
        MonsterFrame {
            frame: 61,
            next: "armagon_slaser19",
            operations: OPS_ARMAGON_SLASER18,
        },
    ),
    (
        "armagon_slaser19",
        MonsterFrame {
            frame: 62,
            next: "armagon_slaser20",
            operations: OPS_ARMAGON_SLASER19,
        },
    ),
    (
        "armagon_slaser2",
        MonsterFrame {
            frame: 53,
            next: "armagon_slaser3",
            operations: OPS_ARMAGON_SLASER2,
        },
    ),
    (
        "armagon_slaser20",
        MonsterFrame {
            frame: 63,
            next: "armagon_plant1",
            operations: OPS_ARMAGON_SLASER20,
        },
    ),
    (
        "armagon_slaser3",
        MonsterFrame {
            frame: 54,
            next: "armagon_slaser4",
            operations: OPS_ARMAGON_SLASER3,
        },
    ),
    (
        "armagon_slaser4",
        MonsterFrame {
            frame: 55,
            next: "armagon_slaser5",
            operations: OPS_ARMAGON_SLASER4,
        },
    ),
    (
        "armagon_slaser5",
        MonsterFrame {
            frame: 56,
            next: "armagon_slaser6",
            operations: OPS_ARMAGON_SLASER5,
        },
    ),
    (
        "armagon_slaser6",
        MonsterFrame {
            frame: 57,
            next: "armagon_slaser7",
            operations: OPS_ARMAGON_SLASER6,
        },
    ),
    (
        "armagon_slaser7",
        MonsterFrame {
            frame: 58,
            next: "armagon_slaser8",
            operations: OPS_ARMAGON_SLASER7,
        },
    ),
    (
        "armagon_slaser8",
        MonsterFrame {
            frame: 59,
            next: "armagon_slaser9",
            operations: OPS_ARMAGON_SLASER8,
        },
    ),
    (
        "armagon_slaser9",
        MonsterFrame {
            frame: 60,
            next: "armagon_slaser10",
            operations: OPS_ARMAGON_SLASER9,
        },
    ),
    (
        "armagon_stand1",
        MonsterFrame {
            frame: 64,
            next: "armagon_stand2",
            operations: OPS_ARMAGON_STAND1,
        },
    ),
    (
        "armagon_stand10",
        MonsterFrame {
            frame: 73,
            next: "armagon_stand11",
            operations: OPS_ARMAGON_STAND10,
        },
    ),
    (
        "armagon_stand11",
        MonsterFrame {
            frame: 74,
            next: "armagon_stand12",
            operations: OPS_ARMAGON_STAND11,
        },
    ),
    (
        "armagon_stand12",
        MonsterFrame {
            frame: 75,
            next: "armagon_stand13",
            operations: OPS_ARMAGON_STAND12,
        },
    ),
    (
        "armagon_stand13",
        MonsterFrame {
            frame: 76,
            next: "armagon_stand14",
            operations: OPS_ARMAGON_STAND13,
        },
    ),
    (
        "armagon_stand14",
        MonsterFrame {
            frame: 77,
            next: "armagon_stand15",
            operations: OPS_ARMAGON_STAND14,
        },
    ),
    (
        "armagon_stand15",
        MonsterFrame {
            frame: 78,
            next: "armagon_stand16",
            operations: OPS_ARMAGON_STAND15,
        },
    ),
    (
        "armagon_stand16",
        MonsterFrame {
            frame: 79,
            next: "armagon_stand17",
            operations: OPS_ARMAGON_STAND16,
        },
    ),
    (
        "armagon_stand17",
        MonsterFrame {
            frame: 80,
            next: "armagon_stand18",
            operations: OPS_ARMAGON_STAND17,
        },
    ),
    (
        "armagon_stand18",
        MonsterFrame {
            frame: 81,
            next: "armagon_stand19",
            operations: OPS_ARMAGON_STAND18,
        },
    ),
    (
        "armagon_stand19",
        MonsterFrame {
            frame: 82,
            next: "armagon_stand20",
            operations: OPS_ARMAGON_STAND19,
        },
    ),
    (
        "armagon_stand2",
        MonsterFrame {
            frame: 65,
            next: "armagon_stand3",
            operations: OPS_ARMAGON_STAND2,
        },
    ),
    (
        "armagon_stand20",
        MonsterFrame {
            frame: 83,
            next: "armagon_stand1",
            operations: OPS_ARMAGON_STAND20,
        },
    ),
    (
        "armagon_stand3",
        MonsterFrame {
            frame: 66,
            next: "armagon_stand4",
            operations: OPS_ARMAGON_STAND3,
        },
    ),
    (
        "armagon_stand4",
        MonsterFrame {
            frame: 67,
            next: "armagon_stand5",
            operations: OPS_ARMAGON_STAND4,
        },
    ),
    (
        "armagon_stand5",
        MonsterFrame {
            frame: 68,
            next: "armagon_stand6",
            operations: OPS_ARMAGON_STAND5,
        },
    ),
    (
        "armagon_stand6",
        MonsterFrame {
            frame: 69,
            next: "armagon_stand7",
            operations: OPS_ARMAGON_STAND6,
        },
    ),
    (
        "armagon_stand7",
        MonsterFrame {
            frame: 70,
            next: "armagon_stand8",
            operations: OPS_ARMAGON_STAND7,
        },
    ),
    (
        "armagon_stand8",
        MonsterFrame {
            frame: 71,
            next: "armagon_stand9",
            operations: OPS_ARMAGON_STAND8,
        },
    ),
    (
        "armagon_stand9",
        MonsterFrame {
            frame: 72,
            next: "armagon_stand10",
            operations: OPS_ARMAGON_STAND9,
        },
    ),
    (
        "armagon_stop1",
        MonsterFrame {
            frame: 84,
            next: "armagon_stop2",
            operations: OPS_ARMAGON_STOP1,
        },
    ),
    (
        "armagon_stop2",
        MonsterFrame {
            frame: 85,
            next: "armagon_plant1",
            operations: OPS_ARMAGON_STOP2,
        },
    ),
    (
        "armagon_walk1",
        MonsterFrame {
            frame: 0,
            next: "armagon_walk2",
            operations: OPS_ARMAGON_WALK1,
        },
    ),
    (
        "armagon_walk10",
        MonsterFrame {
            frame: 9,
            next: "armagon_walk11",
            operations: OPS_ARMAGON_WALK10,
        },
    ),
    (
        "armagon_walk11",
        MonsterFrame {
            frame: 10,
            next: "armagon_walk12",
            operations: OPS_ARMAGON_WALK11,
        },
    ),
    (
        "armagon_walk12",
        MonsterFrame {
            frame: 11,
            next: "armagon_walk1",
            operations: OPS_ARMAGON_WALK12,
        },
    ),
    (
        "armagon_walk2",
        MonsterFrame {
            frame: 1,
            next: "armagon_walk3",
            operations: OPS_ARMAGON_WALK2,
        },
    ),
    (
        "armagon_walk3",
        MonsterFrame {
            frame: 2,
            next: "armagon_walk4",
            operations: OPS_ARMAGON_WALK3,
        },
    ),
    (
        "armagon_walk4",
        MonsterFrame {
            frame: 3,
            next: "armagon_walk5",
            operations: OPS_ARMAGON_WALK4,
        },
    ),
    (
        "armagon_walk5",
        MonsterFrame {
            frame: 4,
            next: "armagon_walk6",
            operations: OPS_ARMAGON_WALK5,
        },
    ),
    (
        "armagon_walk6",
        MonsterFrame {
            frame: 5,
            next: "armagon_walk7",
            operations: OPS_ARMAGON_WALK6,
        },
    ),
    (
        "armagon_walk7",
        MonsterFrame {
            frame: 6,
            next: "armagon_walk8",
            operations: OPS_ARMAGON_WALK7,
        },
    ),
    (
        "armagon_walk8",
        MonsterFrame {
            frame: 7,
            next: "armagon_walk9",
            operations: OPS_ARMAGON_WALK8,
        },
    ),
    (
        "armagon_walk9",
        MonsterFrame {
            frame: 8,
            next: "armagon_walk10",
            operations: OPS_ARMAGON_WALK9,
        },
    ),
    (
        "armagon_watk1",
        MonsterFrame {
            frame: 12,
            next: "armagon_watk2",
            operations: OPS_ARMAGON_WATK1,
        },
    ),
    (
        "armagon_watk10",
        MonsterFrame {
            frame: 21,
            next: "armagon_watk11",
            operations: OPS_ARMAGON_WATK10,
        },
    ),
    (
        "armagon_watk11",
        MonsterFrame {
            frame: 22,
            next: "armagon_watk13",
            operations: OPS_ARMAGON_WATK11,
        },
    ),
    (
        "armagon_watk13",
        MonsterFrame {
            frame: 23,
            next: "armagon_run1",
            operations: OPS_ARMAGON_WATK13,
        },
    ),
    (
        "armagon_watk2",
        MonsterFrame {
            frame: 13,
            next: "armagon_watk3",
            operations: OPS_ARMAGON_WATK2,
        },
    ),
    (
        "armagon_watk3",
        MonsterFrame {
            frame: 14,
            next: "armagon_watk4",
            operations: OPS_ARMAGON_WATK3,
        },
    ),
    (
        "armagon_watk4",
        MonsterFrame {
            frame: 15,
            next: "armagon_watk5",
            operations: OPS_ARMAGON_WATK4,
        },
    ),
    (
        "armagon_watk5",
        MonsterFrame {
            frame: 16,
            next: "armagon_watk6",
            operations: OPS_ARMAGON_WATK5,
        },
    ),
    (
        "armagon_watk6",
        MonsterFrame {
            frame: 17,
            next: "armagon_watk7",
            operations: OPS_ARMAGON_WATK6,
        },
    ),
    (
        "armagon_watk7",
        MonsterFrame {
            frame: 18,
            next: "armagon_watk8",
            operations: OPS_ARMAGON_WATK7,
        },
    ),
    (
        "armagon_watk8",
        MonsterFrame {
            frame: 19,
            next: "armagon_watk9",
            operations: OPS_ARMAGON_WATK8,
        },
    ),
    (
        "armagon_watk9",
        MonsterFrame {
            frame: 20,
            next: "armagon_watk10",
            operations: OPS_ARMAGON_WATK9,
        },
    ),
    (
        "armagon_wlaseratk1",
        MonsterFrame {
            frame: 12,
            next: "armagon_wlaseratk2",
            operations: OPS_ARMAGON_WLASERATK1,
        },
    ),
    (
        "armagon_wlaseratk10",
        MonsterFrame {
            frame: 21,
            next: "armagon_wlaseratk11",
            operations: OPS_ARMAGON_WLASERATK10,
        },
    ),
    (
        "armagon_wlaseratk11",
        MonsterFrame {
            frame: 22,
            next: "armagon_wlaseratk13",
            operations: OPS_ARMAGON_WLASERATK11,
        },
    ),
    (
        "armagon_wlaseratk13",
        MonsterFrame {
            frame: 23,
            next: "armagon_run1",
            operations: OPS_ARMAGON_WLASERATK13,
        },
    ),
    (
        "armagon_wlaseratk2",
        MonsterFrame {
            frame: 13,
            next: "armagon_wlaseratk3",
            operations: OPS_ARMAGON_WLASERATK2,
        },
    ),
    (
        "armagon_wlaseratk3",
        MonsterFrame {
            frame: 14,
            next: "armagon_wlaseratk4",
            operations: OPS_ARMAGON_WLASERATK3,
        },
    ),
    (
        "armagon_wlaseratk4",
        MonsterFrame {
            frame: 15,
            next: "armagon_wlaseratk5",
            operations: OPS_ARMAGON_WLASERATK4,
        },
    ),
    (
        "armagon_wlaseratk5",
        MonsterFrame {
            frame: 16,
            next: "armagon_wlaseratk6",
            operations: OPS_ARMAGON_WLASERATK5,
        },
    ),
    (
        "armagon_wlaseratk6",
        MonsterFrame {
            frame: 17,
            next: "armagon_wlaseratk7",
            operations: OPS_ARMAGON_WLASERATK6,
        },
    ),
    (
        "armagon_wlaseratk7",
        MonsterFrame {
            frame: 18,
            next: "armagon_wlaseratk8",
            operations: OPS_ARMAGON_WLASERATK7,
        },
    ),
    (
        "armagon_wlaseratk8",
        MonsterFrame {
            frame: 19,
            next: "armagon_wlaseratk9",
            operations: OPS_ARMAGON_WLASERATK8,
        },
    ),
    (
        "armagon_wlaseratk9",
        MonsterFrame {
            frame: 20,
            next: "armagon_wlaseratk10",
            operations: OPS_ARMAGON_WLASERATK9,
        },
    ),
];

/// Look up a hiparma frame by name.
#[must_use]
pub fn frame(name: &str) -> Option<&'static MonsterFrame> {
    FRAMES
        .binary_search_by(|(candidate, _)| candidate.cmp(&name))
        .ok()
        .map(|index| &FRAMES[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_sorted_with_expected_count() {
        assert_eq!(FRAMES.len(), 148);
        assert!(FRAMES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn frame_resolves_first_last_and_unknown() {
        let head = frame("armagon_die1").expect("first frame");
        assert_eq!((head.frame, head.next), (84, "armagon_die2"));
        let tail = frame("armagon_wlaseratk9").expect("last frame");
        assert_eq!((tail.frame, tail.next), (20, "armagon_wlaseratk10"));
        assert!(frame("no_such_frame").is_none());
    }
}
