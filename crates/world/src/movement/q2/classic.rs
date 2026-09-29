//! Quake II classic movement.
//!
//! Donor provenance: `src/movement/q2/classic.ts` (ported from Quake II
//! `pmove` sources via quake-2-re-ts and checked against the originals).

use qa_core::math::Bounds;
use qa_core::numeric::{float_to_wrapped_i32, Arithmetic, NumericOps};

use super::super::swept_body::SweepStop;
use super::dimensions::{accept_body_bounds, character_height};
use super::math::{Q2Math, Q2MathEdition};
use super::swept::sweep_q2_body;
use super::types::{
    pm_flags, pm_type, CPlane, CSurface, ClassicPmove, SrcVec3, TraceT, AXES, CONTENTS_CURRENT_0, CONTENTS_CURRENT_180,
    CONTENTS_CURRENT_270, CONTENTS_CURRENT_90, CONTENTS_CURRENT_DOWN, CONTENTS_CURRENT_UP, CONTENTS_LADDER,
    CONTENTS_SLIME, CONTENTS_SOLID, CONTENTS_WATER, MASK_CURRENT, MASK_WATER, MAXTOUCH, PITCH, STEPSIZE,
};
use super::view::classic_view_angles;

fn to_short(x: f64) -> i32 {
    float_to_wrapped_i32(x).wrapping_shl(16) >> 16
}

struct Pml {
    origin: SrcVec3,
    velocity: SrcVec3,
    forward: SrcVec3,
    right: SrcVec3,
    up: SrcVec3,
    frametime: f64,
    groundsurface: Option<CSurface>,
    groundplane: CPlane,
    groundcontents: i32,
    previous_origin: SrcVec3,
    ladder: bool,
}

struct ClassicRunner<'a, 'cb> {
    pm: &'a mut ClassicPmove<'cb>,
    pml: Pml,
    n: NumericOps,
    math: Q2Math,
    air_accelerate: f64,
    strafejump_hack: bool,
    flight: bool,
    speed_multiplier: f64,
    pm_stopspeed: f64,
    pm_maxspeed: f64,
    pm_duckspeed: f64,
    pm_accelerate: f64,
    pm_wateraccelerate: f64,
    pm_friction: f64,
    pm_waterfriction: f64,
    pm_waterspeed: f64,
}

impl ClassicRunner<'_, '_> {
    fn equipment_speed(&self, value: f64) -> f64 {
        if self.speed_multiplier == 1.0 {
            value
        } else {
            self.n.mul(value, self.speed_multiplier)
        }
    }

    fn copy_plane(math: &Q2Math, dst: &mut CPlane, src: &CPlane) {
        math.copy(src.normal, &mut dst.normal);
        dst.dist = src.dist;
        dst.plane_type = src.plane_type;
        dst.signbits = src.signbits;
    }

    fn step_slide_move_(&mut self) {
        let mut primal = self.math.vec3(0.0, 0.0, 0.0);
        self.math.copy(self.pml.velocity, &mut primal);
        let mut body = ClassicSweepBody {
            origin: self.pml.origin,
            velocity: self.pml.velocity,
            mins: self.pm.mins,
            maxs: self.pm.maxs,
            trace: &mut self.pm.trace,
            numtouch: &mut self.pm.numtouch,
            touchents: &mut self.pm.touchents,
            touchtraces: &mut self.pm.touchtraces,
            n: self.n,
            math: self.math,
        };
        let stop = sweep_q2_body(&mut body, self.n, self.pml.frametime);
        self.pml.origin = body.origin;
        self.pml.velocity = body.velocity;
        if stop != SweepStop::Solid && self.pm.s.pm_time != 0 {
            let primal_copy = primal;
            self.math.copy(primal_copy, &mut self.pml.velocity);
        }
    }

    fn step_slide_move(&mut self) {
        let mut start_o = self.math.vec3(0.0, 0.0, 0.0);
        let mut start_v = self.math.vec3(0.0, 0.0, 0.0);
        let mut down_o = self.math.vec3(0.0, 0.0, 0.0);
        let mut down_v = self.math.vec3(0.0, 0.0, 0.0);
        let mut up = self.math.vec3(0.0, 0.0, 0.0);
        let mut down = self.math.vec3(0.0, 0.0, 0.0);
        self.math.copy(self.pml.origin, &mut start_o);
        self.math.copy(self.pml.velocity, &mut start_v);
        self.step_slide_move_();
        self.math.copy(self.pml.origin, &mut down_o);
        self.math.copy(self.pml.velocity, &mut down_v);
        self.math.copy(start_o, &mut up);
        up[2] = f64::from(self.n.store(self.n.add(up[2], f64::from(STEPSIZE))));
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        let mut trace = (self.pm.trace)(up, mins, maxs, up);
        if trace.allsolid {
            return;
        }
        let up_copy = up;
        self.math.copy(up_copy, &mut self.pml.origin);
        let start_v_copy = start_v;
        self.math.copy(start_v_copy, &mut self.pml.velocity);
        self.step_slide_move_();
        self.math.copy(self.pml.origin, &mut down);
        down[2] = f64::from(self.n.store(self.n.sub(down[2], f64::from(STEPSIZE))));
        let origin = self.pml.origin;
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        trace = (self.pm.trace)(origin, mins, maxs, down);
        if !trace.allsolid {
            let endpos = trace.endpos;
            self.math.copy(endpos, &mut self.pml.origin);
        }
        let origin = self.pml.origin;
        self.math.copy(origin, &mut up);
        let down_dist = self.n.add(
            self.n
                .mul(self.n.sub(down_o[0], start_o[0]), self.n.sub(down_o[0], start_o[0])),
            self.n
                .mul(self.n.sub(down_o[1], start_o[1]), self.n.sub(down_o[1], start_o[1])),
        );
        let up_dist = self.n.add(
            self.n.mul(self.n.sub(up[0], start_o[0]), self.n.sub(up[0], start_o[0])),
            self.n.mul(self.n.sub(up[1], start_o[1]), self.n.sub(up[1], start_o[1])),
        );
        if down_dist > up_dist || trace.plane.normal[2] < super::types::MIN_STEP_NORMAL {
            let down_o_copy = down_o;
            let down_v_copy = down_v;
            self.math.copy(down_o_copy, &mut self.pml.origin);
            self.math.copy(down_v_copy, &mut self.pml.velocity);
            return;
        }
        self.pml.velocity[2] = f64::from(self.n.store(down_v[2]));
    }

    fn friction(&mut self) {
        let vel = self.pml.velocity;
        let speed = self.n.sqrt(self.n.add(
            self.n.add(self.n.mul(vel[0], vel[0]), self.n.mul(vel[1], vel[1])),
            self.n.mul(vel[2], vel[2]),
        ));
        if speed < 1.0 {
            self.pml.velocity[0] = f64::from(self.n.store(0.0));
            self.pml.velocity[1] = f64::from(self.n.store(0.0));
            return;
        }
        let mut drop = 0.0;
        let slick = self
            .pml
            .groundsurface
            .as_ref()
            .is_some_and(|surface| surface.flags & super::types::SURF_SLICK == 0);
        if (self.pm.groundentity.is_some() && slick) || self.pml.ladder {
            let control = if speed < self.pm_stopspeed {
                self.pm_stopspeed
            } else {
                speed
            };
            drop = self.n.add(
                drop,
                self.n.mul(self.n.mul(control, self.pm_friction), self.pml.frametime),
            );
        }
        if self.pm.waterlevel != 0 && !self.pml.ladder {
            drop = self.n.add(
                drop,
                self.n.mul(
                    self.n
                        .mul(self.n.mul(speed, self.pm_waterfriction), self.pm.waterlevel as f64),
                    self.pml.frametime,
                ),
            );
        }
        let mut newspeed = self.n.sub(speed, drop);
        if newspeed < 0.0 {
            newspeed = 0.0;
        }
        newspeed = self.n.div(newspeed, speed);
        self.pml.velocity[0] = f64::from(self.n.store(self.n.mul(vel[0], newspeed)));
        self.pml.velocity[1] = f64::from(self.n.store(self.n.mul(vel[1], newspeed)));
        self.pml.velocity[2] = f64::from(self.n.store(self.n.mul(vel[2], newspeed)));
    }

    fn accelerate(&mut self, wishdir: SrcVec3, wishspeed: f64, accel: f64) {
        let currentspeed = self.math.dot(self.pml.velocity, wishdir);
        let addspeed = self.n.sub(wishspeed, currentspeed);
        if addspeed <= 0.0 {
            return;
        }
        let mut accelspeed = self.n.mul(self.n.mul(accel, self.pml.frametime), wishspeed);
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }
        for i in AXES {
            self.pml.velocity[i] = f64::from(
                self.n
                    .store(self.n.add(self.pml.velocity[i], self.n.mul(accelspeed, wishdir[i]))),
            );
        }
    }

    fn air_accelerate(&mut self, wishdir: SrcVec3, wishspeed: f64, accel: f64) {
        let mut wishspd = wishspeed;
        if wishspd > 30.0 {
            wishspd = 30.0;
        }
        let currentspeed = self.math.dot(self.pml.velocity, wishdir);
        let addspeed = self.n.sub(wishspd, currentspeed);
        if addspeed <= 0.0 {
            return;
        }
        let mut accelspeed = self.n.mul(self.n.mul(accel, wishspeed), self.pml.frametime);
        if accelspeed > addspeed {
            accelspeed = addspeed;
        }
        for i in AXES {
            self.pml.velocity[i] = f64::from(
                self.n
                    .store(self.n.add(self.pml.velocity[i], self.n.mul(accelspeed, wishdir[i]))),
            );
        }
    }

    fn add_currents(&mut self, wishvel: &mut SrcVec3) {
        if self.pml.ladder && self.pml.velocity[2].abs() <= 200.0 {
            if self.pm.viewangles[PITCH] <= -15.0 && self.pm.cmd.forwardmove > 0.0 {
                wishvel[2] = f64::from(self.n.store(200.0));
            } else if self.pm.viewangles[PITCH] >= 15.0 && self.pm.cmd.forwardmove > 0.0 {
                wishvel[2] = f64::from(self.n.store(-200.0));
            } else if self.pm.cmd.upmove > 0.0 {
                wishvel[2] = f64::from(self.n.store(200.0));
            } else if self.pm.cmd.upmove < 0.0 {
                wishvel[2] = f64::from(self.n.store(-200.0));
            } else {
                wishvel[2] = f64::from(self.n.store(0.0));
            }
            wishvel[0] = f64::from(self.n.store(wishvel[0].clamp(-25.0, 25.0)));
            wishvel[1] = f64::from(self.n.store(wishvel[1].clamp(-25.0, 25.0)));
        }
        if self.pm.watertype & MASK_CURRENT != 0 {
            let mut v = self.math.vec3(0.0, 0.0, 0.0);
            if self.pm.watertype & CONTENTS_CURRENT_0 != 0 {
                v[0] = f64::from(self.n.store(self.n.add(v[0], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_90 != 0 {
                v[1] = f64::from(self.n.store(self.n.add(v[1], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_180 != 0 {
                v[0] = f64::from(self.n.store(self.n.sub(v[0], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_270 != 0 {
                v[1] = f64::from(self.n.store(self.n.sub(v[1], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_UP != 0 {
                v[2] = f64::from(self.n.store(self.n.add(v[2], 1.0)));
            }
            if self.pm.watertype & CONTENTS_CURRENT_DOWN != 0 {
                v[2] = f64::from(self.n.store(self.n.sub(v[2], 1.0)));
            }
            let mut s = self.pm_waterspeed;
            if self.pm.waterlevel == 1 && self.pm.groundentity.is_some() {
                s = self.n.div(s, 2.0);
            }
            let copy = *wishvel;
            let mut out = *wishvel;
            self.math.ma(copy, s, v, &mut out);
            *wishvel = out;
        }
        if self.pm.groundentity.is_some() {
            let mut v = self.math.vec3(0.0, 0.0, 0.0);
            if self.pml.groundcontents & CONTENTS_CURRENT_0 != 0 {
                v[0] = f64::from(self.n.store(self.n.add(v[0], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_90 != 0 {
                v[1] = f64::from(self.n.store(self.n.add(v[1], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_180 != 0 {
                v[0] = f64::from(self.n.store(self.n.sub(v[0], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_270 != 0 {
                v[1] = f64::from(self.n.store(self.n.sub(v[1], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_UP != 0 {
                v[2] = f64::from(self.n.store(self.n.add(v[2], 1.0)));
            }
            if self.pml.groundcontents & CONTENTS_CURRENT_DOWN != 0 {
                v[2] = f64::from(self.n.store(self.n.sub(v[2], 1.0)));
            }
            let copy = *wishvel;
            let mut out = *wishvel;
            self.math.ma(copy, 100.0, v, &mut out);
            *wishvel = out;
        }
    }

    fn water_move(&mut self) {
        let mut wishvel = self.math.vec3(0.0, 0.0, 0.0);
        let fmove = self.equipment_speed(self.pm.cmd.forwardmove);
        let smove = self.equipment_speed(self.pm.cmd.sidemove);
        for i in AXES {
            wishvel[i] = f64::from(self.n.store(self.n.add(
                self.n.mul(self.pml.forward[i], fmove),
                self.n.mul(self.pml.right[i], smove),
            )));
        }
        if self.pm.cmd.forwardmove == 0.0 && self.pm.cmd.sidemove == 0.0 && self.pm.cmd.upmove == 0.0 {
            wishvel[2] = f64::from(self.n.store(self.n.sub(wishvel[2], 60.0)));
        } else {
            wishvel[2] = f64::from(
                self.n
                    .store(self.n.add(wishvel[2], self.equipment_speed(self.pm.cmd.upmove))),
            );
        }
        self.add_currents(&mut wishvel);
        let mut wishdir = self.math.vec3(0.0, 0.0, 0.0);
        self.math.copy(wishvel, &mut wishdir);
        let mut wishspeed = self.math.normalize(&mut wishdir);
        if wishspeed > self.pm_maxspeed {
            let scale = self.n.div(self.pm_maxspeed, wishspeed);
            self.math.mul_eq(&mut wishvel, scale);
            wishspeed = self.pm_maxspeed;
        }
        wishspeed = self.n.mul(wishspeed, 0.5);
        let accelerate = self.pm_wateraccelerate;
        self.accelerate(wishdir, wishspeed, accelerate);
        self.step_slide_move();
    }

    fn air_move(&mut self) {
        let mut wishvel = self.math.vec3(0.0, 0.0, 0.0);
        let fmove = self.equipment_speed(self.pm.cmd.forwardmove);
        let smove = self.equipment_speed(self.pm.cmd.sidemove);
        for i in 0..2 {
            wishvel[i] = f64::from(self.n.store(self.n.add(
                self.n.mul(self.pml.forward[i], fmove),
                self.n.mul(self.pml.right[i], smove),
            )));
        }
        wishvel[2] = f64::from(self.n.store(0.0));
        self.add_currents(&mut wishvel);
        let mut wishdir = self.math.vec3(0.0, 0.0, 0.0);
        self.math.copy(wishvel, &mut wishdir);
        let mut wishspeed = self.math.normalize(&mut wishdir);
        let maxspeed = if self.pm.s.pm_flags & pm_flags::DUCKED != 0 {
            self.pm_duckspeed
        } else {
            self.pm_maxspeed
        };
        if wishspeed > maxspeed {
            let mut out = wishvel;
            self.math.scale_into(wishvel, self.n.div(maxspeed, wishspeed), &mut out);
            wishvel = out;
            wishspeed = maxspeed;
        }
        if self.pml.ladder {
            let accelerate = self.pm_accelerate;
            self.accelerate(wishdir, wishspeed, accelerate);
            if wishvel[2] == 0.0 {
                if self.pml.velocity[2] > 0.0 {
                    self.pml.velocity[2] = f64::from(
                        self.n.store(
                            self.n
                                .sub(self.pml.velocity[2], self.n.mul(self.pm.s.gravity, self.pml.frametime)),
                        ),
                    );
                    if self.pml.velocity[2] < 0.0 {
                        self.pml.velocity[2] = f64::from(self.n.store(0.0));
                    }
                } else {
                    self.pml.velocity[2] = f64::from(
                        self.n.store(
                            self.n
                                .add(self.pml.velocity[2], self.n.mul(self.pm.s.gravity, self.pml.frametime)),
                        ),
                    );
                    if self.pml.velocity[2] > 0.0 {
                        self.pml.velocity[2] = f64::from(self.n.store(0.0));
                    }
                }
            }
            self.step_slide_move();
        } else if self.pm.groundentity.is_some() {
            self.pml.velocity[2] = f64::from(self.n.store(0.0));
            let accelerate = self.pm_accelerate;
            self.accelerate(wishdir, wishspeed, accelerate);
            if self.pm.s.gravity > 0.0 {
                self.pml.velocity[2] = f64::from(self.n.store(0.0));
            } else {
                self.pml.velocity[2] = f64::from(
                    self.n.store(
                        self.n
                            .sub(self.pml.velocity[2], self.n.mul(self.pm.s.gravity, self.pml.frametime)),
                    ),
                );
            }
            if self.pml.velocity[0] == 0.0 && self.pml.velocity[1] == 0.0 {
                return;
            }
            self.step_slide_move();
        } else {
            let air = self.air_accelerate;
            if air != 0.0 {
                self.air_accelerate(wishdir, wishspeed, self.pm_accelerate);
            } else {
                self.accelerate(wishdir, wishspeed, 1.0);
            }
            self.pml.velocity[2] = f64::from(
                self.n.store(
                    self.n
                        .sub(self.pml.velocity[2], self.n.mul(self.pm.s.gravity, self.pml.frametime)),
                ),
            );
            self.step_slide_move();
        }
    }

    fn categorize_position(&mut self) {
        let mut point = self.math.vec3(0.0, 0.0, 0.0);
        point[0] = f64::from(self.n.store(self.pml.origin[0]));
        point[1] = f64::from(self.n.store(self.pml.origin[1]));
        point[2] = f64::from(self.n.store(self.n.sub(self.pml.origin[2], 0.25)));
        if self.pml.velocity[2] > 180.0 {
            self.pm.s.pm_flags &= !pm_flags::ON_GROUND;
            self.pm.groundentity = None;
        } else {
            let origin = self.pml.origin;
            let mins = self.pm.mins;
            let maxs = self.pm.maxs;
            let trace = (self.pm.trace)(origin, mins, maxs, point);
            let plane = trace.plane.clone();
            let math = self.math;
            Self::copy_plane(&math, &mut self.pml.groundplane, &plane);
            self.pml.groundsurface = trace.surface.clone();
            self.pml.groundcontents = trace.contents;
            if trace.ent.is_none() || (trace.plane.normal[2] < 0.7 && !trace.startsolid) {
                self.pm.groundentity = None;
                self.pm.s.pm_flags &= !pm_flags::ON_GROUND;
            } else {
                self.pm.groundentity = trace.ent.clone();
                if self.pm.s.pm_flags & pm_flags::TIME_WATERJUMP != 0 {
                    self.pm.s.pm_flags &= !(pm_flags::TIME_WATERJUMP | pm_flags::TIME_LAND | pm_flags::TIME_TELEPORT);
                    self.pm.s.pm_time = 0;
                }
                if self.pm.s.pm_flags & pm_flags::ON_GROUND == 0 {
                    self.pm.s.pm_flags |= pm_flags::ON_GROUND;
                    if self.pml.velocity[2] < -200.0 && !self.strafejump_hack {
                        self.pm.s.pm_flags |= pm_flags::TIME_LAND;
                        self.pm.s.pm_time = if self.pml.velocity[2] < -400.0 { 25 } else { 18 };
                    }
                }
            }
            if self.pm.numtouch < MAXTOUCH {
                if let Some(ent) = trace.ent.clone() {
                    if self.pm.numtouch >= self.pm.touchents.len() {
                        self.pm.touchents.resize(self.pm.numtouch + 1, ent.clone());
                        self.pm.touchtraces.resize(self.pm.numtouch + 1, trace.clone());
                    }
                    self.pm.touchents[self.pm.numtouch] = ent;
                    self.pm.touchtraces[self.pm.numtouch] = trace;
                    self.pm.numtouch += 1;
                }
            }
        }
        self.pm.waterlevel = 0;
        self.pm.watertype = 0;
        let sample2 = self.n.sub(self.pm.viewheight, self.pm.mins[2]).trunc() as i32;
        let sample1 = (self.n.div(sample2 as f64, 2.0)) as i32;
        point[2] = f64::from(
            self.n
                .store(self.n.add(self.n.add(self.pml.origin[2], self.pm.mins[2]), 1.0)),
        );
        let point_copy = point;
        let mut cont = (self.pm.pointcontents)(point_copy);
        if cont & MASK_WATER != 0 {
            self.pm.watertype = cont;
            self.pm.waterlevel = 1;
            point[2] = f64::from(
                self.n.store(
                    self.n
                        .add(self.n.add(self.pml.origin[2], self.pm.mins[2]), sample1 as f64),
                ),
            );
            let point_copy = point;
            cont = (self.pm.pointcontents)(point_copy);
            if cont & MASK_WATER != 0 {
                self.pm.waterlevel = 2;
                point[2] = f64::from(
                    self.n.store(
                        self.n
                            .add(self.n.add(self.pml.origin[2], self.pm.mins[2]), sample2 as f64),
                    ),
                );
                let point_copy = point;
                cont = (self.pm.pointcontents)(point_copy);
                if cont & MASK_WATER != 0 {
                    self.pm.waterlevel = 3;
                }
            }
        }
    }

    fn check_jump(&mut self) {
        if self.pm.s.pm_flags & pm_flags::TIME_LAND != 0 {
            return;
        }
        if self.pm.cmd.upmove < 10.0 {
            self.pm.s.pm_flags &= !pm_flags::JUMP_HELD;
            return;
        }
        if self.pm.s.pm_flags & pm_flags::JUMP_HELD != 0 {
            return;
        }
        if self.pm.s.pm_type == pm_type::DEAD {
            return;
        }
        if self.pm.waterlevel >= 2 {
            self.pm.groundentity = None;
            if self.pml.velocity[2] <= -300.0 {
                return;
            }
            self.pml.velocity[2] = f64::from(self.n.store(if self.pm.watertype == CONTENTS_WATER {
                100.0
            } else if self.pm.watertype == CONTENTS_SLIME {
                80.0
            } else {
                50.0
            }));
            return;
        }
        if self.pm.groundentity.is_none() {
            return;
        }
        self.pm.s.pm_flags |= pm_flags::JUMP_HELD;
        self.pm.groundentity = None;
        self.pml.velocity[2] = f64::from(self.n.store(self.n.add(self.pml.velocity[2], 270.0)));
        if self.pml.velocity[2] < 270.0 {
            self.pml.velocity[2] = f64::from(self.n.store(270.0));
        }
    }

    fn check_special_movement(&mut self) {
        let mut spot = self.math.vec3(0.0, 0.0, 0.0);
        let mut flatforward = self.math.vec3(0.0, 0.0, 0.0);
        if self.pm.s.pm_time != 0 {
            return;
        }
        self.pml.ladder = false;
        flatforward[0] = f64::from(self.n.store(self.pml.forward[0]));
        flatforward[1] = f64::from(self.n.store(self.pml.forward[1]));
        flatforward[2] = f64::from(self.n.store(0.0));
        self.math.normalize(&mut flatforward);
        let origin = self.pml.origin;
        self.math.ma(origin, 1.0, flatforward, &mut spot);
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        let trace = (self.pm.trace)(origin, mins, maxs, spot);
        if trace.fraction < 1.0 && trace.contents & CONTENTS_LADDER != 0 {
            self.pml.ladder = true;
        }
        if self.pm.waterlevel != 2 {
            return;
        }
        let origin = self.pml.origin;
        self.math.ma(origin, 30.0, flatforward, &mut spot);
        spot[2] = f64::from(self.n.store(self.n.add(spot[2], 4.0)));
        let spot_copy = spot;
        let cont = (self.pm.pointcontents)(spot_copy);
        if cont & CONTENTS_SOLID == 0 {
            return;
        }
        spot[2] = f64::from(self.n.store(self.n.add(spot[2], 16.0)));
        let spot_copy = spot;
        let cont = (self.pm.pointcontents)(spot_copy);
        if cont != 0 {
            return;
        }
        let mut out = self.pml.velocity;
        self.math.scale_into(flatforward, 50.0, &mut out);
        self.pml.velocity = out;
        self.pml.velocity[2] = f64::from(self.n.store(350.0));
        self.pm.s.pm_flags |= pm_flags::TIME_WATERJUMP;
        self.pm.s.pm_time = 255;
    }

    fn fly_move(&mut self, doclip: bool) {
        self.pm.viewheight = character_height(&self.pm.character_bounds, 22.0, &self.n);
        let speed = self.math.length(self.pml.velocity);
        if speed < 1.0 {
            let mut velocity = self.pml.velocity;
            self.math.clear(&mut velocity);
            self.pml.velocity = velocity;
        } else {
            let friction = self.n.mul(self.pm_friction, 1.5);
            let control = if speed < self.pm_stopspeed {
                self.pm_stopspeed
            } else {
                speed
            };
            let drop = self.n.mul(self.n.mul(control, friction), self.pml.frametime);
            let mut newspeed = self.n.sub(speed, drop);
            if newspeed < 0.0 {
                newspeed = 0.0;
            }
            newspeed = self.n.div(newspeed, speed);
            let mut velocity = self.pml.velocity;
            self.math.scale_into(self.pml.velocity, newspeed, &mut velocity);
            self.pml.velocity = velocity;
        }
        let fmove = self.equipment_speed(self.pm.cmd.forwardmove);
        let smove = self.equipment_speed(self.pm.cmd.sidemove);
        let mut forward = self.pml.forward;
        self.math.normalize(&mut forward);
        self.pml.forward = forward;
        let mut right = self.pml.right;
        self.math.normalize(&mut right);
        self.pml.right = right;
        let mut wishvel = self.math.vec3(0.0, 0.0, 0.0);
        for i in AXES {
            wishvel[i] = f64::from(self.n.store(self.n.add(
                self.n.mul(self.pml.forward[i], fmove),
                self.n.mul(self.pml.right[i], smove),
            )));
        }
        wishvel[2] = f64::from(
            self.n
                .store(self.n.add(wishvel[2], self.equipment_speed(self.pm.cmd.upmove))),
        );
        let mut wishdir = self.math.vec3(0.0, 0.0, 0.0);
        self.math.copy(wishvel, &mut wishdir);
        let mut wishspeed = self.math.normalize(&mut wishdir);
        if wishspeed > self.pm_maxspeed {
            let scale = self.n.div(self.pm_maxspeed, wishspeed);
            self.math.mul_eq(&mut wishvel, scale);
            wishspeed = self.pm_maxspeed;
        }
        let currentspeed = self.math.dot(self.pml.velocity, wishdir);
        let addspeed = self.n.sub(wishspeed, currentspeed);
        if addspeed <= 0.0 && !doclip {
            return;
        }
        let accelspeed = self
            .n
            .mul(self.n.mul(self.pm_accelerate, self.pml.frametime), wishspeed)
            .clamp(0.0, addspeed.max(0.0));
        for i in AXES {
            self.pml.velocity[i] = f64::from(
                self.n
                    .store(self.n.add(self.pml.velocity[i], self.n.mul(accelspeed, wishdir[i]))),
            );
        }
        if doclip {
            self.step_slide_move();
        } else {
            let origin = self.pml.origin;
            let velocity = self.pml.velocity;
            let frametime = self.pml.frametime;
            let mut out = origin;
            self.math.ma(origin, frametime, velocity, &mut out);
            self.pml.origin = out;
        }
    }

    fn check_duck(&mut self) {
        let previous_duck = self.pm.s.pm_flags & pm_flags::DUCKED;
        self.pm.mins[0] = f64::from(self.n.store(f64::from(self.pm.character_bounds.min.x)));
        self.pm.mins[1] = f64::from(self.n.store(f64::from(self.pm.character_bounds.min.y)));
        self.pm.maxs[0] = f64::from(self.n.store(f64::from(self.pm.character_bounds.max.x)));
        self.pm.maxs[1] = f64::from(self.n.store(f64::from(self.pm.character_bounds.max.y)));
        if self.pm.s.pm_type == pm_type::GIB {
            self.pm.mins[2] = f64::from(self.n.store(character_height(&self.pm.character_bounds, 0.0, &self.n)));
            self.pm.maxs[2] = f64::from(self.n.store(character_height(&self.pm.character_bounds, 16.0, &self.n)));
            self.pm.viewheight = character_height(&self.pm.character_bounds, 8.0, &self.n);
            return;
        }
        self.pm.mins[2] = f64::from(
            self.n
                .store(character_height(&self.pm.character_bounds, -24.0, &self.n)),
        );
        if self.pm.s.pm_type == pm_type::DEAD {
            self.pm.s.pm_flags |= pm_flags::DUCKED;
        } else if self.pm.cmd.upmove < 0.0 && self.pm.s.pm_flags & pm_flags::ON_GROUND != 0 {
            self.pm.s.pm_flags |= pm_flags::DUCKED;
        } else if self.pm.s.pm_flags & pm_flags::DUCKED != 0 {
            self.pm.maxs[2] = f64::from(self.n.store(character_height(&self.pm.character_bounds, 32.0, &self.n)));
            let origin = self.pml.origin;
            let (mins, maxs) = match self.pm.body_bounds {
                Some(requested) => (
                    [
                        f64::from(requested.min.x),
                        f64::from(requested.min.y),
                        f64::from(requested.min.z),
                    ],
                    [
                        f64::from(requested.max.x),
                        f64::from(requested.max.y),
                        f64::from(requested.max.z),
                    ],
                ),
                None => (self.pm.mins, self.pm.maxs),
            };
            let trace = (self.pm.trace)(origin, mins, maxs, origin);
            if !trace.allsolid {
                self.pm.s.pm_flags &= !pm_flags::DUCKED;
            }
        }
        if self.pm.s.pm_flags & pm_flags::DUCKED != 0 {
            self.pm.maxs[2] = f64::from(self.n.store(character_height(&self.pm.character_bounds, 4.0, &self.n)));
            self.pm.viewheight = character_height(&self.pm.character_bounds, -2.0, &self.n);
        } else {
            self.pm.maxs[2] = f64::from(self.n.store(character_height(&self.pm.character_bounds, 32.0, &self.n)));
            self.pm.viewheight = character_height(&self.pm.character_bounds, 22.0, &self.n);
        }
        let requested = self.pm.body_bounds.unwrap_or(Bounds {
            min: qa_core::math::vec3(self.pm.mins[0] as f32, self.pm.mins[1] as f32, self.pm.mins[2] as f32),
            max: qa_core::math::vec3(self.pm.maxs[0] as f32, self.pm.maxs[1] as f32, self.pm.maxs[2] as f32),
        });
        let previous = self.pm.previous_bounds.unwrap_or(requested);
        let origin = self.pml.origin;
        let trace_fn = &mut self.pm.trace;
        let bounds = accept_body_bounds(&previous, &requested, |bounds| {
            let trace = trace_fn(
                origin,
                [
                    f64::from(bounds.min.x),
                    f64::from(bounds.min.y),
                    f64::from(bounds.min.z),
                ],
                [
                    f64::from(bounds.max.x),
                    f64::from(bounds.max.y),
                    f64::from(bounds.max.z),
                ],
                origin,
            );
            !trace.allsolid
        });
        if bounds != requested {
            self.pm.s.pm_flags = (self.pm.s.pm_flags & !pm_flags::DUCKED) | previous_duck;
            self.pm.viewheight = character_height(
                &self.pm.character_bounds,
                if previous_duck != 0 { -2.0 } else { 22.0 },
                &self.n,
            );
        }
        self.pm.mins = [
            f64::from(bounds.min.x),
            f64::from(bounds.min.y),
            f64::from(bounds.min.z),
        ];
        self.pm.maxs = [
            f64::from(bounds.max.x),
            f64::from(bounds.max.y),
            f64::from(bounds.max.z),
        ];
    }

    fn dead_move(&mut self) {
        if self.pm.groundentity.is_none() {
            return;
        }
        let mut forward = self.math.length(self.pml.velocity);
        forward = self.n.sub(forward, 20.0);
        if forward <= 0.0 {
            let mut velocity = self.pml.velocity;
            self.math.clear(&mut velocity);
            self.pml.velocity = velocity;
        } else {
            let mut velocity = self.pml.velocity;
            self.math.normalize(&mut velocity);
            let mut out = velocity;
            self.math.scale_into(velocity, forward, &mut out);
            self.pml.velocity = out;
        }
    }

    fn good_position(&mut self) -> bool {
        if self.pm.s.pm_type == pm_type::SPECTATOR {
            return true;
        }
        let mut origin = self.math.vec3(0.0, 0.0, 0.0);
        let mut end = self.math.vec3(0.0, 0.0, 0.0);
        for i in AXES {
            origin[i] = f64::from(self.n.store(self.n.mul(self.pm.s.origin[i] as f64, 0.125)));
            end[i] = origin[i];
        }
        let mins = self.pm.mins;
        let maxs = self.pm.maxs;
        let trace = (self.pm.trace)(origin, mins, maxs, end);
        !trace.allsolid
    }

    fn snap_position(&mut self) {
        const JITTERBITS: [i32; 8] = [0, 4, 1, 2, 3, 5, 6, 7];
        let mut sign = [0.0, 0.0, 0.0];
        let mut base = [0i16; 3];
        for i in AXES {
            self.pm.s.velocity[i] = to_short(self.n.mul(self.pml.velocity[i], 8.0));
        }
        for i in AXES {
            sign[i] = f64::from(self.n.store(if self.pml.origin[i] >= 0.0 { 1.0 } else { -1.0 }));
            self.pm.s.origin[i] = to_short(self.n.mul(self.pml.origin[i], 8.0));
            if self.n.mul(self.pm.s.origin[i] as f64, 0.125) == self.pml.origin[i] {
                sign[i] = f64::from(self.n.store(0.0));
            }
        }
        for i in AXES {
            base[i] = self.pm.s.origin[i] as i16;
        }
        for j in 0..8 {
            let bits = JITTERBITS[j];
            for i in AXES {
                self.pm.s.origin[i] = to_short(base[i] as f64);
            }
            for i in AXES {
                if bits & (1 << i) != 0 {
                    self.pm.s.origin[i] = to_short(self.n.add(self.pm.s.origin[i] as f64, sign[i]));
                }
            }
            if self.good_position() {
                return;
            }
        }
        for i in AXES {
            self.pm.s.origin[i] = to_short(self.pml.previous_origin[i]);
        }
    }

    fn initial_snap_position(&mut self) {
        const OFFSET: [f64; 3] = [0.0, -1.0, 1.0];
        let mut base = [0i16; 3];
        for i in AXES {
            base[i] = f64::from(self.n.store(self.pm.s.origin[i] as f64)) as i16;
        }
        for z in AXES {
            self.pm.s.origin[2] = to_short(self.n.add(base[2] as f64, OFFSET[z]));
            for y in AXES {
                self.pm.s.origin[1] = to_short(self.n.add(base[1] as f64, OFFSET[y]));
                for x in AXES {
                    self.pm.s.origin[0] = to_short(self.n.add(base[0] as f64, OFFSET[x]));
                    if self.good_position() {
                        self.pml.origin[0] = f64::from(self.n.store(self.n.mul(self.pm.s.origin[0] as f64, 0.125)));
                        self.pml.origin[1] = f64::from(self.n.store(self.n.mul(self.pm.s.origin[1] as f64, 0.125)));
                        self.pml.origin[2] = f64::from(self.n.store(self.n.mul(self.pm.s.origin[2] as f64, 0.125)));
                        for i in AXES {
                            self.pml.previous_origin[i] = f64::from(self.n.store(self.pm.s.origin[i] as f64));
                        }
                        return;
                    }
                }
            }
        }
    }

    fn clamp_angles(&mut self) {
        let angles = self.pm.cmd.angles;
        let delta = self.pm.s.delta_angles;
        let flags = self.pm.s.pm_flags;
        let n = self.n;
        classic_view_angles(&mut self.pm.viewangles, angles, delta, flags, &n);
        let viewangles = self.pm.viewangles;
        let mut forward = self.pml.forward;
        let mut right = self.pml.right;
        let mut up = self.pml.up;
        self.math.angle_vectors(viewangles, &mut forward, &mut right, &mut up);
        self.pml.forward = forward;
        self.pml.right = right;
        self.pml.up = up;
    }

    fn run(&mut self) {
        self.pm.numtouch = 0;
        let mut viewangles = self.pm.viewangles;
        self.math.clear(&mut viewangles);
        self.pm.viewangles = viewangles;
        self.pm.viewheight = 0.0;
        self.pm.groundentity = None;
        self.pm.watertype = 0;
        self.pm.waterlevel = 0;
        let fresh = Pml {
            origin: self.math.vec3(0.0, 0.0, 0.0),
            velocity: self.math.vec3(0.0, 0.0, 0.0),
            forward: self.math.vec3(0.0, 0.0, 0.0),
            right: self.math.vec3(0.0, 0.0, 0.0),
            up: self.math.vec3(0.0, 0.0, 0.0),
            frametime: 0.0,
            groundsurface: None,
            groundplane: super::types::plane(),
            groundcontents: 0,
            previous_origin: self.math.vec3(0.0, 0.0, 0.0),
            ladder: false,
        };
        self.pml = fresh;
        self.pml.origin[0] = f64::from(self.n.store(self.n.mul(self.pm.s.origin[0] as f64, 0.125)));
        self.pml.origin[1] = f64::from(self.n.store(self.n.mul(self.pm.s.origin[1] as f64, 0.125)));
        self.pml.origin[2] = f64::from(self.n.store(self.n.mul(self.pm.s.origin[2] as f64, 0.125)));
        self.pml.velocity[0] = f64::from(self.n.store(self.n.mul(self.pm.s.velocity[0] as f64, 0.125)));
        self.pml.velocity[1] = f64::from(self.n.store(self.n.mul(self.pm.s.velocity[1] as f64, 0.125)));
        self.pml.velocity[2] = f64::from(self.n.store(self.n.mul(self.pm.s.velocity[2] as f64, 0.125)));
        for i in AXES {
            self.pml.previous_origin[i] = f64::from(self.n.store(self.pm.s.origin[i] as f64));
        }
        self.pml.frametime = self.n.mul(self.pm.cmd.msec as f64, 0.001);
        self.clamp_angles();
        if self.flight && self.pm.s.pm_type == pm_type::NORMAL {
            self.pm.s.pm_flags &= !(pm_flags::ON_GROUND | pm_flags::DUCKED | pm_flags::TIME_WATERJUMP);
            self.pm.s.pm_time = 0;
            self.fly_move(true);
            self.snap_position();
            return;
        }
        if self.pm.s.pm_type == pm_type::SPECTATOR {
            self.fly_move(false);
            self.snap_position();
            return;
        }
        if self.pm.s.pm_type >= pm_type::DEAD {
            self.pm.cmd.forwardmove = 0.0;
            self.pm.cmd.sidemove = 0.0;
            self.pm.cmd.upmove = 0.0;
        }
        if self.pm.s.pm_type == pm_type::FREEZE {
            return;
        }
        self.check_duck();
        if self.pm.snapinitial {
            self.initial_snap_position();
        }
        self.categorize_position();
        if self.pm.s.pm_type == pm_type::DEAD {
            self.dead_move();
        }
        self.check_special_movement();
        if self.pm.s.pm_time != 0 {
            let mut msec = self.pm.cmd.msec >> 3;
            if msec == 0 {
                msec = 1;
            }
            if msec >= self.pm.s.pm_time {
                self.pm.s.pm_flags &= !(pm_flags::TIME_WATERJUMP | pm_flags::TIME_LAND | pm_flags::TIME_TELEPORT);
                self.pm.s.pm_time = 0;
            } else {
                self.pm.s.pm_time = self.n.sub(self.pm.s.pm_time as f64, msec as f64) as i32;
            }
        }
        if self.pm.s.pm_flags & pm_flags::TIME_TELEPORT != 0 {
        } else if self.pm.s.pm_flags & pm_flags::TIME_WATERJUMP != 0 {
            self.pml.velocity[2] = f64::from(
                self.n.store(
                    self.n
                        .sub(self.pml.velocity[2], self.n.mul(self.pm.s.gravity, self.pml.frametime)),
                ),
            );
            if self.pml.velocity[2] < 0.0 {
                self.pm.s.pm_flags &= !(pm_flags::TIME_WATERJUMP | pm_flags::TIME_LAND | pm_flags::TIME_TELEPORT);
                self.pm.s.pm_time = 0;
            }
            self.step_slide_move();
        } else {
            self.check_jump();
            self.friction();
            if self.pm.waterlevel >= 2 {
                self.water_move();
            } else {
                let mut angles = self.pm.viewangles;
                if angles[PITCH] > 180.0 {
                    angles[PITCH] = f64::from(self.n.store(self.n.sub(angles[PITCH], 360.0)));
                }
                angles[PITCH] = f64::from(self.n.store(self.n.div(angles[PITCH], 3.0)));
                let mut forward = self.pml.forward;
                let mut right = self.pml.right;
                let mut up = self.pml.up;
                self.math.angle_vectors(angles, &mut forward, &mut right, &mut up);
                self.pml.forward = forward;
                self.pml.right = right;
                self.pml.up = up;
                self.air_move();
            }
        }
        self.categorize_position();
        self.snap_position();
    }
}

struct ClassicSweepBody<'a, 'cb> {
    origin: SrcVec3,
    velocity: SrcVec3,
    mins: SrcVec3,
    maxs: SrcVec3,
    trace: &'a mut Box<dyn FnMut(SrcVec3, SrcVec3, SrcVec3, SrcVec3) -> TraceT + 'cb>,
    numtouch: &'a mut usize,
    touchents: &'a mut Vec<super::types::MovementEntity>,
    touchtraces: &'a mut Vec<TraceT>,
    n: NumericOps,
    math: Q2Math,
}

impl super::swept::Q2SweepBody for ClassicSweepBody<'_, '_> {
    fn origin(&self) -> SrcVec3 {
        self.origin
    }
    fn velocity(&self) -> SrcVec3 {
        self.velocity
    }
    fn write_origin(&mut self, origin: SrcVec3) {
        self.origin = origin;
    }
    fn write_velocity(&mut self, velocity: SrcVec3) {
        self.velocity = velocity;
    }
    fn write_velocity_zero_z(&mut self) {
        self.velocity[2] = f64::from(self.n.store(0.0));
    }
    fn trace(&mut self, start: SrcVec3, end: SrcVec3) -> TraceT {
        (self.trace)(start, self.mins, self.maxs, end)
    }
    fn clip(&mut self, input: SrcVec3, normal: SrcVec3) -> SrcVec3 {
        let mut overbounce = 1.01;
        if !matches!(self.n.profile.arithmetic, Arithmetic::DonorBinary64(_)) {
            overbounce = f64::from(self.n.store(overbounce));
        }
        let mut out = self.math.vec3(0.0, 0.0, 0.0);
        let backoff = self.n.mul(self.math.dot(input, normal), overbounce);
        for i in AXES {
            let change = self.n.mul(normal[i], backoff);
            out[i] = f64::from(self.n.store(self.n.sub(input[i], change)));
            if out[i] > -super::types::STOP_EPSILON && out[i] < super::types::STOP_EPSILON {
                out[i] = f64::from(self.n.store(0.0));
            }
        }
        out
    }
    fn touch(&mut self, trace: &TraceT) {
        if *self.numtouch < MAXTOUCH {
            if let Some(ent) = &trace.ent {
                if *self.numtouch >= self.touchents.len() {
                    self.touchents.resize(*self.numtouch + 1, ent.clone());
                    self.touchtraces.resize(*self.numtouch + 1, trace.clone());
                }
                self.touchents[*self.numtouch] = ent.clone();
                self.touchtraces[*self.numtouch] = trace.clone();
                *self.numtouch += 1;
            }
        }
    }
}

/// Run classic Quake II pmove over one command.
pub fn pmove_classic(
    pm: &mut ClassicPmove<'_>,
    numeric_ops: NumericOps,
    air_accelerate: f64,
    strafejump_hack: bool,
    flight: bool,
    speed_multiplier: f64,
) {
    let math = Q2Math::new(numeric_ops, Q2MathEdition::Classic);
    let equipment = |value: f64| {
        if speed_multiplier == 1.0 {
            value
        } else {
            numeric_ops.mul(value, speed_multiplier)
        }
    };
    let mut runner = ClassicRunner {
        pm,
        pml: Pml {
            origin: [0.0, 0.0, 0.0],
            velocity: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, 0.0],
            right: [0.0, 0.0, 0.0],
            up: [0.0, 0.0, 0.0],
            frametime: 0.0,
            groundsurface: None,
            groundplane: super::types::plane(),
            groundcontents: 0,
            previous_origin: [0.0, 0.0, 0.0],
            ladder: false,
        },
        n: numeric_ops,
        math,
        air_accelerate,
        strafejump_hack,
        flight,
        speed_multiplier,
        pm_stopspeed: 100.0,
        pm_maxspeed: equipment(300.0),
        pm_duckspeed: equipment(100.0),
        pm_accelerate: 10.0,
        pm_wateraccelerate: 10.0,
        pm_friction: 6.0,
        pm_waterfriction: 1.0,
        pm_waterspeed: 400.0,
    };
    runner.run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};

    use super::super::types::{ClassicPmoveCmd, ClassicPmoveState};

    fn open_trace(end: SrcVec3) -> TraceT {
        TraceT {
            allsolid: false,
            startsolid: false,
            fraction: 1.0,
            endpos: end,
            plane: super::super::types::plane(),
            surface: None,
            contents: 0,
            ent: None,
            plane2: super::super::types::plane(),
            surface2: None,
            native: None,
        }
    }

    fn fixture() -> ClassicPmove<'static> {
        ClassicPmove {
            s: ClassicPmoveState {
                pm_type: pm_type::NORMAL,
                origin: [0, 0, 800],
                velocity: [0, 0, 0],
                pm_flags: pm_flags::ON_GROUND,
                pm_time: 0,
                gravity: 800.0,
                delta_angles: [0, 0, 0],
            },
            cmd: ClassicPmoveCmd {
                msec: 50,
                angles: [0, 8192, 0],
                forwardmove: 400.0,
                sidemove: 0.0,
                upmove: 0.0,
                buttons: 0,
                impulse: 0,
                lightlevel: 0,
            },
            snapinitial: false,
            numtouch: 0,
            touchents: Vec::new(),
            touchtraces: Vec::new(),
            viewangles: [0.0, 0.0, 0.0],
            viewheight: 0.0,
            mins: [-16.0, -16.0, -24.0],
            maxs: [16.0, 16.0, 32.0],
            groundentity: None,
            watertype: 0,
            waterlevel: 0,
            trace: Box::new(|_start, _mins, _maxs, end| open_trace(end)),
            pointcontents: Box::new(|_| 0),
            character_bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            body_bounds: None,
            previous_bounds: None,
        }
    }

    #[test]
    fn ground_step_accelerates_and_snaps() {
        let mut pm = fixture();
        // Fake ground: the categorize trace reports a floor.
        pm.trace = Box::new(|start, _mins, _maxs, end| {
            if end[2] < start[2] && start[2] - end[2] <= 1.0 {
                let mut plane = super::super::types::plane();
                plane.normal = [0.0, 0.0, 1.0];
                TraceT {
                    allsolid: false,
                    startsolid: false,
                    fraction: 0.0,
                    endpos: start,
                    plane,
                    surface: None,
                    contents: 0,
                    ent: Some(super::super::super::types::TraceHit::World { model: 0 }),
                    plane2: super::super::types::plane(),
                    surface2: None,
                    native: None,
                }
            } else {
                open_trace(end)
            }
        });
        pmove_classic(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            0.0,
            false,
            false,
            1.0,
        );
        assert_eq!(pm.s.pm_flags & pm_flags::ON_GROUND, pm_flags::ON_GROUND);
        assert!(pm.s.velocity[1] > 0);
        assert_eq!(pm.viewheight, 22.0);
    }

    #[test]
    fn jump_requires_release_and_ground() {
        let mut pm = fixture();
        pm.cmd.upmove = 20.0;
        pm.trace = Box::new(|start, _mins, _maxs, end| {
            if end[2] < start[2] && start[2] - end[2] <= 1.0 {
                let mut plane = super::super::types::plane();
                plane.normal = [0.0, 0.0, 1.0];
                TraceT {
                    allsolid: false,
                    startsolid: false,
                    fraction: 0.0,
                    endpos: start,
                    plane,
                    surface: None,
                    contents: 0,
                    ent: Some(super::super::super::types::TraceHit::World { model: 0 }),
                    plane2: super::super::types::plane(),
                    surface2: None,
                    native: None,
                }
            } else {
                open_trace(end)
            }
        });
        pmove_classic(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            0.0,
            false,
            false,
            1.0,
        );
        assert_eq!(pm.s.pm_flags & pm_flags::JUMP_HELD, pm_flags::JUMP_HELD);
        // Jump launches at 270 then air-move gravity applies in the same frame.
        assert!(pm.s.velocity[2] > 200 * 8);
    }

    #[test]
    fn airborne_step_applies_gravity() {
        let mut pm = fixture();
        pm.s.pm_flags = 0;
        pm.cmd.forwardmove = 0.0;
        pmove_classic(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            0.0,
            false,
            false,
            1.0,
        );
        assert!(pm.s.velocity[2] < 0);
    }

    #[test]
    fn spectator_flies_without_snapping_ground() {
        let mut pm = fixture();
        pm.s.pm_type = pm_type::SPECTATOR;
        pmove_classic(
            &mut pm,
            NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            0.0,
            false,
            false,
            1.0,
        );
        assert!(pm.s.velocity[1] > 0);
    }

    #[test]
    fn to_short_wraps_like_source() {
        assert_eq!(to_short(1.0), 1);
        assert_eq!(to_short(65536.0), 0);
        assert_eq!(to_short(-1.0), -1);
    }
}
