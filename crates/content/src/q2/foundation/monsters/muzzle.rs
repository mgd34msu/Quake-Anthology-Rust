//! Monster muzzle offsets (`src/content/q2/foundation/monsters/muzzle.ts`).
//!
//! Monster muzzle offsets from Quake II `m_flash.c` and rerelease
//! `m_flash.h` (id Software, GPL-2.0-or-later). Donor `Math.fround`
//! expressions evaluate in binary64, then convert, exactly like the
//! source tables.

use qa_core::math::Vec3;

use super::types::record_at;
use crate::q2::foundation::host::Q2Edition;

/// Classic muzzle offsets.
const CLASSIC: [Vec3; 212] = [
    Vec3 {
        x: (0.0) as f32,
        y: (0.0) as f32,
        z: (0.0) as f32,
    },
    Vec3 {
        x: (20.7) as f32,
        y: (-18.5) as f32,
        z: (28.7) as f32,
    },
    Vec3 {
        x: (16.6) as f32,
        y: (-21.5) as f32,
        z: (30.1) as f32,
    },
    Vec3 {
        x: (11.8) as f32,
        y: (-23.9) as f32,
        z: (32.1) as f32,
    },
    Vec3 {
        x: (22.9) as f32,
        y: (-0.7) as f32,
        z: (25.3) as f32,
    },
    Vec3 {
        x: (22.2) as f32,
        y: (6.2) as f32,
        z: (22.3) as f32,
    },
    Vec3 {
        x: (19.4) as f32,
        y: (13.1) as f32,
        z: (18.6) as f32,
    },
    Vec3 {
        x: (19.4) as f32,
        y: (18.8) as f32,
        z: (18.6) as f32,
    },
    Vec3 {
        x: (17.9) as f32,
        y: (25.0) as f32,
        z: (18.6) as f32,
    },
    Vec3 {
        x: (14.1) as f32,
        y: (30.5) as f32,
        z: (20.6) as f32,
    },
    Vec3 {
        x: (9.3) as f32,
        y: (35.3) as f32,
        z: (22.1) as f32,
    },
    Vec3 {
        x: (4.7) as f32,
        y: (38.4) as f32,
        z: (22.1) as f32,
    },
    Vec3 {
        x: (-1.1) as f32,
        y: (40.4) as f32,
        z: (24.1) as f32,
    },
    Vec3 {
        x: (-6.5) as f32,
        y: (41.2) as f32,
        z: (24.1) as f32,
    },
    Vec3 {
        x: (3.2) as f32,
        y: (40.1) as f32,
        z: (24.7) as f32,
    },
    Vec3 {
        x: (11.7) as f32,
        y: (36.7) as f32,
        z: (26.0) as f32,
    },
    Vec3 {
        x: (18.9) as f32,
        y: (31.3) as f32,
        z: (26.0) as f32,
    },
    Vec3 {
        x: (24.4) as f32,
        y: (24.4) as f32,
        z: (26.4) as f32,
    },
    Vec3 {
        x: (27.1) as f32,
        y: (17.1) as f32,
        z: (27.2) as f32,
    },
    Vec3 {
        x: (28.5) as f32,
        y: (9.1) as f32,
        z: (28.0) as f32,
    },
    Vec3 {
        x: (27.1) as f32,
        y: (2.2) as f32,
        z: (28.0) as f32,
    },
    Vec3 {
        x: (24.9) as f32,
        y: (-2.8) as f32,
        z: (28.0) as f32,
    },
    Vec3 {
        x: (21.6) as f32,
        y: (-7.0) as f32,
        z: (26.4) as f32,
    },
    Vec3 {
        x: (6.2) as f32,
        y: (29.1) as f32,
        z: (49.1) as f32,
    },
    Vec3 {
        x: (6.9) as f32,
        y: (23.8) as f32,
        z: (49.1) as f32,
    },
    Vec3 {
        x: (8.3) as f32,
        y: (17.8) as f32,
        z: (49.5) as f32,
    },
    Vec3 {
        x: (26.6) as f32,
        y: (7.1) as f32,
        z: (13.1) as f32,
    },
    Vec3 {
        x: (18.2) as f32,
        y: (7.5) as f32,
        z: (15.4) as f32,
    },
    Vec3 {
        x: (17.2) as f32,
        y: (10.3) as f32,
        z: (17.9) as f32,
    },
    Vec3 {
        x: (17.0) as f32,
        y: (12.8) as f32,
        z: (20.1) as f32,
    },
    Vec3 {
        x: (15.1) as f32,
        y: (14.1) as f32,
        z: (21.8) as f32,
    },
    Vec3 {
        x: (11.8) as f32,
        y: (17.2) as f32,
        z: (23.1) as f32,
    },
    Vec3 {
        x: (11.4) as f32,
        y: (20.2) as f32,
        z: (21.0) as f32,
    },
    Vec3 {
        x: (9.0) as f32,
        y: (23.0) as f32,
        z: (18.9) as f32,
    },
    Vec3 {
        x: (13.9) as f32,
        y: (18.6) as f32,
        z: (17.7) as f32,
    },
    Vec3 {
        x: (15.4) as f32,
        y: (15.6) as f32,
        z: (15.8) as f32,
    },
    Vec3 {
        x: (10.2) as f32,
        y: (15.2) as f32,
        z: (25.1) as f32,
    },
    Vec3 {
        x: (-1.9) as f32,
        y: (15.1) as f32,
        z: (28.2) as f32,
    },
    Vec3 {
        x: (-12.4) as f32,
        y: (13.0) as f32,
        z: (20.2) as f32,
    },
    Vec3 {
        x: (10.6 * 1.2) as f32,
        y: (7.7 * 1.2) as f32,
        z: (7.8 * 1.2) as f32,
    },
    Vec3 {
        x: (21.1 * 1.2) as f32,
        y: (3.6 * 1.2) as f32,
        z: (19.0 * 1.2) as f32,
    },
    Vec3 {
        x: (10.6 * 1.2) as f32,
        y: (7.7 * 1.2) as f32,
        z: (7.8 * 1.2) as f32,
    },
    Vec3 {
        x: (21.1 * 1.2) as f32,
        y: (3.6 * 1.2) as f32,
        z: (19.0 * 1.2) as f32,
    },
    Vec3 {
        x: (10.6 * 1.2) as f32,
        y: (7.7 * 1.2) as f32,
        z: (7.8 * 1.2) as f32,
    },
    Vec3 {
        x: (21.1 * 1.2) as f32,
        y: (3.6 * 1.2) as f32,
        z: (19.0 * 1.2) as f32,
    },
    Vec3 {
        x: (30.1 * 1.15) as f32,
        y: (3.9 * 1.15) as f32,
        z: (19.6 * 1.15) as f32,
    },
    Vec3 {
        x: (29.1 * 1.15) as f32,
        y: (2.5 * 1.15) as f32,
        z: (20.7 * 1.15) as f32,
    },
    Vec3 {
        x: (28.2 * 1.15) as f32,
        y: (2.5 * 1.15) as f32,
        z: (22.2 * 1.15) as f32,
    },
    Vec3 {
        x: (28.2 * 1.15) as f32,
        y: (3.6 * 1.15) as f32,
        z: (22.0 * 1.15) as f32,
    },
    Vec3 {
        x: (26.9 * 1.15) as f32,
        y: (2.0 * 1.15) as f32,
        z: (23.4 * 1.15) as f32,
    },
    Vec3 {
        x: (26.5 * 1.15) as f32,
        y: (0.6 * 1.15) as f32,
        z: (20.8 * 1.15) as f32,
    },
    Vec3 {
        x: (26.9 * 1.15) as f32,
        y: (0.5 * 1.15) as f32,
        z: (21.5 * 1.15) as f32,
    },
    Vec3 {
        x: (29.0 * 1.15) as f32,
        y: (2.4 * 1.15) as f32,
        z: (19.5 * 1.15) as f32,
    },
    Vec3 {
        x: (4.6 * 1.15) as f32,
        y: (-16.8 * 1.15) as f32,
        z: (7.3 * 1.15) as f32,
    },
    Vec3 {
        x: (4.6 * 1.15) as f32,
        y: (-16.8 * 1.15) as f32,
        z: (7.3 * 1.15) as f32,
    },
    Vec3 {
        x: (4.6 * 1.15) as f32,
        y: (-16.8 * 1.15) as f32,
        z: (7.3 * 1.15) as f32,
    },
    Vec3 {
        x: (4.6 * 1.15) as f32,
        y: (-16.8 * 1.15) as f32,
        z: (7.3 * 1.15) as f32,
    },
    Vec3 {
        x: (24.8) as f32,
        y: (-9.0) as f32,
        z: (39.0) as f32,
    },
    Vec3 {
        x: (12.1) as f32,
        y: (13.4) as f32,
        z: (-14.5) as f32,
    },
    Vec3 {
        x: (12.1) as f32,
        y: (-7.4) as f32,
        z: (-14.5) as f32,
    },
    Vec3 {
        x: (12.1) as f32,
        y: (5.4) as f32,
        z: (16.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (18.0) as f32,
        z: (28.0) as f32,
    },
    Vec3 {
        x: (32.5) as f32,
        y: (-0.8) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (18.4) as f32,
        y: (7.4) as f32,
        z: (9.6) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (30.0) as f32,
        z: (88.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (30.0) as f32,
        z: (88.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (30.0) as f32,
        z: (88.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (30.0) as f32,
        z: (88.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (30.0) as f32,
        z: (88.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (30.0) as f32,
        z: (88.5) as f32,
    },
    Vec3 {
        x: (16.0) as f32,
        y: (-22.5) as f32,
        z: (91.2) as f32,
    },
    Vec3 {
        x: (16.0) as f32,
        y: (-33.4) as f32,
        z: (86.7) as f32,
    },
    Vec3 {
        x: (16.0) as f32,
        y: (-42.8) as f32,
        z: (83.3) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (-40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (-40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (-40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (-40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (-40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (22.0) as f32,
        y: (16.0) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (22.0) as f32,
        y: (8.0) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (22.0) as f32,
        y: (-8.0) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (22.0) as f32,
        y: (-16.0) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (32.5) as f32,
        y: (-0.8) as f32,
        z: (10) as f32,
    },
    Vec3 {
        x: (20.8 * 1.2) as f32,
        y: (10.1 * 1.2) as f32,
        z: (-2.7 * 1.2) as f32,
    },
    Vec3 {
        x: (20.8 * 1.2) as f32,
        y: (10.1 * 1.2) as f32,
        z: (-2.7 * 1.2) as f32,
    },
    Vec3 {
        x: (20.8 * 1.2) as f32,
        y: (10.1 * 1.2) as f32,
        z: (-2.7 * 1.2) as f32,
    },
    Vec3 {
        x: (7.6 * 1.2) as f32,
        y: (9.3 * 1.2) as f32,
        z: (0.8 * 1.2) as f32,
    },
    Vec3 {
        x: (7.6 * 1.2) as f32,
        y: (9.3 * 1.2) as f32,
        z: (0.8 * 1.2) as f32,
    },
    Vec3 {
        x: (7.6 * 1.2) as f32,
        y: (9.3 * 1.2) as f32,
        z: (0.8 * 1.2) as f32,
    },
    Vec3 {
        x: (30.5 * 1.2) as f32,
        y: (9.9 * 1.2) as f32,
        z: (-18.7 * 1.2) as f32,
    },
    Vec3 {
        x: (30.5 * 1.2) as f32,
        y: (9.9 * 1.2) as f32,
        z: (-18.7 * 1.2) as f32,
    },
    Vec3 {
        x: (30.5 * 1.2) as f32,
        y: (9.9 * 1.2) as f32,
        z: (-18.7 * 1.2) as f32,
    },
    Vec3 {
        x: (27.6 * 1.2) as f32,
        y: (3.4 * 1.2) as f32,
        z: (-10.4 * 1.2) as f32,
    },
    Vec3 {
        x: (27.6 * 1.2) as f32,
        y: (3.4 * 1.2) as f32,
        z: (-10.4 * 1.2) as f32,
    },
    Vec3 {
        x: (27.6 * 1.2) as f32,
        y: (3.4 * 1.2) as f32,
        z: (-10.4 * 1.2) as f32,
    },
    Vec3 {
        x: (28.9 * 1.2) as f32,
        y: (4.6 * 1.2) as f32,
        z: (-8.1 * 1.2) as f32,
    },
    Vec3 {
        x: (28.9 * 1.2) as f32,
        y: (4.6 * 1.2) as f32,
        z: (-8.1 * 1.2) as f32,
    },
    Vec3 {
        x: (28.9 * 1.2) as f32,
        y: (4.6 * 1.2) as f32,
        z: (-8.1 * 1.2) as f32,
    },
    Vec3 {
        x: (31.5 * 1.2) as f32,
        y: (9.6 * 1.2) as f32,
        z: (10.1 * 1.2) as f32,
    },
    Vec3 {
        x: (34.5 * 1.2) as f32,
        y: (9.6 * 1.2) as f32,
        z: (6.1 * 1.2) as f32,
    },
    Vec3 {
        x: (34.5 * 1.2) as f32,
        y: (9.6 * 1.2) as f32,
        z: (6.1 * 1.2) as f32,
    },
    Vec3 {
        x: (17) as f32,
        y: (-19.5) as f32,
        z: (62.9) as f32,
    },
    Vec3 {
        x: (-3.6) as f32,
        y: (-24.1) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-1.6) as f32,
        y: (-19.3) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-0.1) as f32,
        y: (-14.4) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (2.0) as f32,
        y: (-7.6) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (3.4) as f32,
        y: (1.3) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (3.7) as f32,
        y: (11.1) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-0.3) as f32,
        y: (22.3) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-6) as f32,
        y: (33) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-9.3) as f32,
        y: (36.4) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-7) as f32,
        y: (35) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-2.1) as f32,
        y: (29) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (3.9) as f32,
        y: (17.3) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (6.1) as f32,
        y: (5.8) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (5.9) as f32,
        y: (-4.4) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (4.2) as f32,
        y: (-14.1) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (2.4) as f32,
        y: (-18.8) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-1.8) as f32,
        y: (-25.5) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-17.3) as f32,
        y: (7.8) as f32,
        z: (72.4) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96) as f32,
    },
    Vec3 {
        x: (6.3) as f32,
        y: (-9) as f32,
        z: (111.2) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (40) as f32,
        z: (70) as f32,
    },
    Vec3 {
        x: (56) as f32,
        y: (-32) as f32,
        z: (32) as f32,
    },
    Vec3 {
        x: (56) as f32,
        y: (32) as f32,
        z: (32) as f32,
    },
    Vec3 {
        x: (42) as f32,
        y: (24) as f32,
        z: (50) as f32,
    },
    Vec3 {
        x: (16) as f32,
        y: (0) as f32,
        z: (0) as f32,
    },
    Vec3 {
        x: (16) as f32,
        y: (0) as f32,
        z: (0) as f32,
    },
    Vec3 {
        x: (16) as f32,
        y: (0) as f32,
        z: (0) as f32,
    },
    Vec3 {
        x: (24) as f32,
        y: (0) as f32,
        z: (6) as f32,
    },
    Vec3 {
        x: (32.5) as f32,
        y: (-0.8) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (12.1) as f32,
        y: (5.4) as f32,
        z: (16.5) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (0) as f32,
        z: (6) as f32,
    },
    Vec3 {
        x: (57.72) as f32,
        y: (14.50) as f32,
        z: (88.81) as f32,
    },
    Vec3 {
        x: (56) as f32,
        y: (32) as f32,
        z: (32) as f32,
    },
    Vec3 {
        x: (62) as f32,
        y: (-20) as f32,
        z: (84) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (0) as f32,
        z: (6) as f32,
    },
    Vec3 {
        x: (61) as f32,
        y: (-32) as f32,
        z: (12) as f32,
    },
    Vec3 {
        x: (61) as f32,
        y: (32) as f32,
        z: (12) as f32,
    },
    Vec3 {
        x: (17) as f32,
        y: (-62) as f32,
        z: (91) as f32,
    },
    Vec3 {
        x: (68) as f32,
        y: (12) as f32,
        z: (86) as f32,
    },
    Vec3 {
        x: (47.5) as f32,
        y: (56) as f32,
        z: (89) as f32,
    },
    Vec3 {
        x: (54) as f32,
        y: (52) as f32,
        z: (91) as f32,
    },
    Vec3 {
        x: (58) as f32,
        y: (40) as f32,
        z: (91) as f32,
    },
    Vec3 {
        x: (68) as f32,
        y: (30) as f32,
        z: (88) as f32,
    },
    Vec3 {
        x: (74) as f32,
        y: (20) as f32,
        z: (88) as f32,
    },
    Vec3 {
        x: (73) as f32,
        y: (11) as f32,
        z: (87) as f32,
    },
    Vec3 {
        x: (73) as f32,
        y: (3) as f32,
        z: (87) as f32,
    },
    Vec3 {
        x: (70) as f32,
        y: (-12) as f32,
        z: (87) as f32,
    },
    Vec3 {
        x: (67) as f32,
        y: (-20) as f32,
        z: (90) as f32,
    },
    Vec3 {
        x: (-20) as f32,
        y: (76) as f32,
        z: (90) as f32,
    },
    Vec3 {
        x: (-8) as f32,
        y: (74) as f32,
        z: (90) as f32,
    },
    Vec3 {
        x: (0) as f32,
        y: (72) as f32,
        z: (90) as f32,
    },
    Vec3 {
        x: (10) as f32,
        y: (71) as f32,
        z: (89) as f32,
    },
    Vec3 {
        x: (23) as f32,
        y: (70) as f32,
        z: (87) as f32,
    },
    Vec3 {
        x: (32) as f32,
        y: (64) as f32,
        z: (85) as f32,
    },
    Vec3 {
        x: (40) as f32,
        y: (58) as f32,
        z: (84) as f32,
    },
    Vec3 {
        x: (48) as f32,
        y: (50) as f32,
        z: (83) as f32,
    },
    Vec3 {
        x: (54) as f32,
        y: (42) as f32,
        z: (82) as f32,
    },
    Vec3 {
        x: (56) as f32,
        y: (34) as f32,
        z: (82) as f32,
    },
    Vec3 {
        x: (58) as f32,
        y: (26) as f32,
        z: (82) as f32,
    },
    Vec3 {
        x: (60) as f32,
        y: (16) as f32,
        z: (82) as f32,
    },
    Vec3 {
        x: (59) as f32,
        y: (6) as f32,
        z: (81) as f32,
    },
    Vec3 {
        x: (58) as f32,
        y: (-2) as f32,
        z: (80) as f32,
    },
    Vec3 {
        x: (57) as f32,
        y: (-10) as f32,
        z: (79) as f32,
    },
    Vec3 {
        x: (54) as f32,
        y: (-18) as f32,
        z: (78) as f32,
    },
    Vec3 {
        x: (42) as f32,
        y: (-32) as f32,
        z: (80) as f32,
    },
    Vec3 {
        x: (36) as f32,
        y: (-40) as f32,
        z: (78) as f32,
    },
    Vec3 {
        x: (68.4) as f32,
        y: (10.88) as f32,
        z: (82.08) as f32,
    },
    Vec3 {
        x: (68.51) as f32,
        y: (8.64) as f32,
        z: (85.14) as f32,
    },
    Vec3 {
        x: (68.66) as f32,
        y: (6.38) as f32,
        z: (88.78) as f32,
    },
    Vec3 {
        x: (68.73) as f32,
        y: (5.1) as f32,
        z: (84.47) as f32,
    },
    Vec3 {
        x: (68.82) as f32,
        y: (4.79) as f32,
        z: (80.52) as f32,
    },
    Vec3 {
        x: (68.77) as f32,
        y: (6.11) as f32,
        z: (85.37) as f32,
    },
    Vec3 {
        x: (68.67) as f32,
        y: (7.99) as f32,
        z: (90.24) as f32,
    },
    Vec3 {
        x: (68.55) as f32,
        y: (9.54) as f32,
        z: (87.36) as f32,
    },
    Vec3 {
        x: (0) as f32,
        y: (0) as f32,
        z: (-5) as f32,
    },
    Vec3 {
        x: (0) as f32,
        y: (0) as f32,
        z: (-5) as f32,
    },
    Vec3 {
        x: (0) as f32,
        y: (0) as f32,
        z: (-5) as f32,
    },
    Vec3 {
        x: (0) as f32,
        y: (0) as f32,
        z: (-5) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-17.63) as f32,
        z: (93.77) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-17.08) as f32,
        z: (89.82) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-18.40) as f32,
        z: (90.70) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-18.34) as f32,
        z: (94.32) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-18.30) as f32,
        z: (97.98) as f32,
    },
    Vec3 {
        x: (45.04) as f32,
        y: (-59.02) as f32,
        z: (92.24) as f32,
    },
    Vec3 {
        x: (50.68) as f32,
        y: (-54.70) as f32,
        z: (91.96) as f32,
    },
    Vec3 {
        x: (56.57) as f32,
        y: (-47.72) as f32,
        z: (91.65) as f32,
    },
    Vec3 {
        x: (61.75) as f32,
        y: (-38.75) as f32,
        z: (91.38) as f32,
    },
    Vec3 {
        x: (65.55) as f32,
        y: (-28.76) as f32,
        z: (91.24) as f32,
    },
    Vec3 {
        x: (67.79) as f32,
        y: (-18.90) as f32,
        z: (91.22) as f32,
    },
    Vec3 {
        x: (68.60) as f32,
        y: (-9.52) as f32,
        z: (91.23) as f32,
    },
    Vec3 {
        x: (68.08) as f32,
        y: (0.18) as f32,
        z: (91.32) as f32,
    },
    Vec3 {
        x: (66.14) as f32,
        y: (9.79) as f32,
        z: (91.44) as f32,
    },
    Vec3 {
        x: (62.77) as f32,
        y: (18.91) as f32,
        z: (91.65) as f32,
    },
    Vec3 {
        x: (58.29) as f32,
        y: (27.11) as f32,
        z: (92.00) as f32,
    },
    Vec3 {
        x: (0.0) as f32,
        y: (0.0) as f32,
        z: (0.0) as f32,
    },
];

/// Rerelease muzzle offsets.
const RERELEASE: [Vec3; 290] = [
    Vec3 {
        x: (0.0) as f32,
        y: (0.0) as f32,
        z: (0.0) as f32,
    },
    Vec3 {
        x: (28.7) as f32,
        y: (-18.5) as f32,
        z: (28.7) as f32,
    },
    Vec3 {
        x: (24.6) as f32,
        y: (-21.5) as f32,
        z: (30.1) as f32,
    },
    Vec3 {
        x: (19.8) as f32,
        y: (-23.9) as f32,
        z: (32.1) as f32,
    },
    Vec3 {
        x: (22.9) as f32,
        y: (-0.7) as f32,
        z: (25.3) as f32,
    },
    Vec3 {
        x: (22.2) as f32,
        y: (6.2) as f32,
        z: (22.3) as f32,
    },
    Vec3 {
        x: (19.4) as f32,
        y: (13.1) as f32,
        z: (18.6) as f32,
    },
    Vec3 {
        x: (19.4) as f32,
        y: (18.8) as f32,
        z: (18.6) as f32,
    },
    Vec3 {
        x: (17.9) as f32,
        y: (25.0) as f32,
        z: (18.6) as f32,
    },
    Vec3 {
        x: (14.1) as f32,
        y: (30.5) as f32,
        z: (20.6) as f32,
    },
    Vec3 {
        x: (9.3) as f32,
        y: (35.3) as f32,
        z: (22.1) as f32,
    },
    Vec3 {
        x: (4.7) as f32,
        y: (38.4) as f32,
        z: (22.1) as f32,
    },
    Vec3 {
        x: (-1.1) as f32,
        y: (40.4) as f32,
        z: (24.1) as f32,
    },
    Vec3 {
        x: (-6.5) as f32,
        y: (41.2) as f32,
        z: (24.1) as f32,
    },
    Vec3 {
        x: (3.2) as f32,
        y: (40.1) as f32,
        z: (24.7) as f32,
    },
    Vec3 {
        x: (11.7) as f32,
        y: (36.7) as f32,
        z: (26.0) as f32,
    },
    Vec3 {
        x: (18.9) as f32,
        y: (31.3) as f32,
        z: (26.0) as f32,
    },
    Vec3 {
        x: (24.4) as f32,
        y: (24.4) as f32,
        z: (26.4) as f32,
    },
    Vec3 {
        x: (27.1) as f32,
        y: (17.1) as f32,
        z: (27.2) as f32,
    },
    Vec3 {
        x: (28.5) as f32,
        y: (9.1) as f32,
        z: (28.0) as f32,
    },
    Vec3 {
        x: (27.1) as f32,
        y: (2.2) as f32,
        z: (28.0) as f32,
    },
    Vec3 {
        x: (24.9) as f32,
        y: (-2.8) as f32,
        z: (28.0) as f32,
    },
    Vec3 {
        x: (21.6) as f32,
        y: (-7.0) as f32,
        z: (26.4) as f32,
    },
    Vec3 {
        x: (6.2) as f32,
        y: (29.1) as f32,
        z: (49.1) as f32,
    },
    Vec3 {
        x: (6.9) as f32,
        y: (23.8) as f32,
        z: (49.1) as f32,
    },
    Vec3 {
        x: (8.3) as f32,
        y: (17.8) as f32,
        z: (49.5) as f32,
    },
    Vec3 {
        x: (26.6) as f32,
        y: (7.1) as f32,
        z: (13.1) as f32,
    },
    Vec3 {
        x: (18.2) as f32,
        y: (7.5) as f32,
        z: (15.4) as f32,
    },
    Vec3 {
        x: (17.2) as f32,
        y: (10.3) as f32,
        z: (17.9) as f32,
    },
    Vec3 {
        x: (17.0) as f32,
        y: (12.8) as f32,
        z: (20.1) as f32,
    },
    Vec3 {
        x: (15.1) as f32,
        y: (14.1) as f32,
        z: (21.8) as f32,
    },
    Vec3 {
        x: (11.8) as f32,
        y: (17.2) as f32,
        z: (23.1) as f32,
    },
    Vec3 {
        x: (11.4) as f32,
        y: (20.2) as f32,
        z: (21.0) as f32,
    },
    Vec3 {
        x: (9.0) as f32,
        y: (23.0) as f32,
        z: (18.9) as f32,
    },
    Vec3 {
        x: (13.9) as f32,
        y: (18.6) as f32,
        z: (17.7) as f32,
    },
    Vec3 {
        x: (15.4) as f32,
        y: (15.6) as f32,
        z: (15.8) as f32,
    },
    Vec3 {
        x: (10.2) as f32,
        y: (15.2) as f32,
        z: (25.1) as f32,
    },
    Vec3 {
        x: (-1.9) as f32,
        y: (15.1) as f32,
        z: (28.2) as f32,
    },
    Vec3 {
        x: (-12.4) as f32,
        y: (13.0) as f32,
        z: (20.2) as f32,
    },
    Vec3 {
        x: (10.6 * 1.2) as f32,
        y: (7.7 * 1.2) as f32,
        z: (7.8 * 1.2) as f32,
    },
    Vec3 {
        x: (25.1 * 1.2) as f32,
        y: (3.6 * 1.2) as f32,
        z: (19.0 * 1.2) as f32,
    },
    Vec3 {
        x: (10.6 * 1.2) as f32,
        y: (7.7 * 1.2) as f32,
        z: (7.8 * 1.2) as f32,
    },
    Vec3 {
        x: (25.1 * 1.2) as f32,
        y: (3.6 * 1.2) as f32,
        z: (19.0 * 1.2) as f32,
    },
    Vec3 {
        x: (10.6 * 1.2) as f32,
        y: (7.7 * 1.2) as f32,
        z: (7.8 * 1.2) as f32,
    },
    Vec3 {
        x: (25.1 * 1.2) as f32,
        y: (3.6 * 1.2) as f32,
        z: (19.0 * 1.2) as f32,
    },
    Vec3 {
        x: (30.1 * 1.15) as f32,
        y: (3.9 * 1.15) as f32,
        z: (19.6 * 1.15) as f32,
    },
    Vec3 {
        x: (29.1 * 1.15) as f32,
        y: (2.5 * 1.15) as f32,
        z: (20.7 * 1.15) as f32,
    },
    Vec3 {
        x: (28.2 * 1.15) as f32,
        y: (2.5 * 1.15) as f32,
        z: (22.2 * 1.15) as f32,
    },
    Vec3 {
        x: (28.2 * 1.15) as f32,
        y: (3.6 * 1.15) as f32,
        z: (22.0 * 1.15) as f32,
    },
    Vec3 {
        x: (26.9 * 1.15) as f32,
        y: (2.0 * 1.15) as f32,
        z: (23.4 * 1.15) as f32,
    },
    Vec3 {
        x: (26.5 * 1.15) as f32,
        y: (0.6 * 1.15) as f32,
        z: (20.8 * 1.15) as f32,
    },
    Vec3 {
        x: (26.9 * 1.15) as f32,
        y: (0.5 * 1.15) as f32,
        z: (21.5 * 1.15) as f32,
    },
    Vec3 {
        x: (29.0 * 1.15) as f32,
        y: (2.4 * 1.15) as f32,
        z: (19.5 * 1.15) as f32,
    },
    Vec3 {
        x: (4.6 * 1.15) as f32,
        y: (-16.8 * 1.15) as f32,
        z: (7.3 * 1.15) as f32,
    },
    Vec3 {
        x: (4.6 * 1.15) as f32,
        y: (-16.8 * 1.15) as f32,
        z: (7.3 * 1.15) as f32,
    },
    Vec3 {
        x: (4.6 * 1.15) as f32,
        y: (-16.8 * 1.15) as f32,
        z: (7.3 * 1.15) as f32,
    },
    Vec3 {
        x: (4.6 * 1.15) as f32,
        y: (-16.8 * 1.15) as f32,
        z: (7.3 * 1.15) as f32,
    },
    Vec3 {
        x: (24.8) as f32,
        y: (-9.0) as f32,
        z: (39.0) as f32,
    },
    Vec3 {
        x: (14.1) as f32,
        y: (13.4) as f32,
        z: (-7.0) as f32,
    },
    Vec3 {
        x: (14.1) as f32,
        y: (-13.4) as f32,
        z: (-7.0) as f32,
    },
    Vec3 {
        x: (44.0) as f32,
        y: (3.0) as f32,
        z: (14.4) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (18.0) as f32,
        z: (28.0) as f32,
    },
    Vec3 {
        x: (1.7) as f32,
        y: (7.0) as f32,
        z: (11.3) as f32,
    },
    Vec3 {
        x: (18.4) as f32,
        y: (7.4) as f32,
        z: (9.6) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (39.0) as f32,
        z: (85.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (39.0) as f32,
        z: (85.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (39.0) as f32,
        z: (85.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (39.0) as f32,
        z: (85.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (39.0) as f32,
        z: (85.5) as f32,
    },
    Vec3 {
        x: (30.0) as f32,
        y: (39.0) as f32,
        z: (85.5) as f32,
    },
    Vec3 {
        x: (16.0) as f32,
        y: (-22.5) as f32,
        z: (108.7) as f32,
    },
    Vec3 {
        x: (16.0) as f32,
        y: (-33.4) as f32,
        z: (106.7) as f32,
    },
    Vec3 {
        x: (16.0) as f32,
        y: (-42.8) as f32,
        z: (104.7) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (-40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (-40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (-40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (-40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (-40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (22.0) as f32,
        y: (16.0) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (22.0) as f32,
        y: (8.0) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (22.0) as f32,
        y: (-8.0) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (22.0) as f32,
        y: (-16.0) as f32,
        z: (10.0) as f32,
    },
    Vec3 {
        x: (32.5) as f32,
        y: (-0.8) as f32,
        z: (10.) as f32,
    },
    Vec3 {
        x: (20.8 * 1.2) as f32,
        y: (10.1 * 1.2) as f32,
        z: (-2.7 * 1.2) as f32,
    },
    Vec3 {
        x: (20.8 * 1.2) as f32,
        y: (10.1 * 1.2) as f32,
        z: (-2.7 * 1.2) as f32,
    },
    Vec3 {
        x: (20.8 * 1.2) as f32,
        y: (10.1 * 1.2) as f32,
        z: (-2.7 * 1.2) as f32,
    },
    Vec3 {
        x: (7.6 * 1.2) as f32,
        y: (9.3 * 1.2) as f32,
        z: (0.8 * 1.2) as f32,
    },
    Vec3 {
        x: (7.6 * 1.2) as f32,
        y: (9.3 * 1.2) as f32,
        z: (0.8 * 1.2) as f32,
    },
    Vec3 {
        x: (7.6 * 1.2) as f32,
        y: (9.3 * 1.2) as f32,
        z: (0.8 * 1.2) as f32,
    },
    Vec3 {
        x: (30.5 * 1.2) as f32,
        y: (9.9 * 1.2) as f32,
        z: (-18.7 * 1.2) as f32,
    },
    Vec3 {
        x: (30.5 * 1.2) as f32,
        y: (9.9 * 1.2) as f32,
        z: (-18.7 * 1.2) as f32,
    },
    Vec3 {
        x: (30.5 * 1.2) as f32,
        y: (9.9 * 1.2) as f32,
        z: (-18.7 * 1.2) as f32,
    },
    Vec3 {
        x: (27.6 * 1.2) as f32,
        y: (3.4 * 1.2) as f32,
        z: (-10.4 * 1.2) as f32,
    },
    Vec3 {
        x: (27.6 * 1.2) as f32,
        y: (3.4 * 1.2) as f32,
        z: (-10.4 * 1.2) as f32,
    },
    Vec3 {
        x: (27.6 * 1.2) as f32,
        y: (3.4 * 1.2) as f32,
        z: (-10.4 * 1.2) as f32,
    },
    Vec3 {
        x: (28.9 * 1.2) as f32,
        y: (4.6 * 1.2) as f32,
        z: (-8.1 * 1.2) as f32,
    },
    Vec3 {
        x: (28.9 * 1.2) as f32,
        y: (4.6 * 1.2) as f32,
        z: (-8.1 * 1.2) as f32,
    },
    Vec3 {
        x: (28.9 * 1.2) as f32,
        y: (4.6 * 1.2) as f32,
        z: (-8.1 * 1.2) as f32,
    },
    Vec3 {
        x: (31.5 * 1.2) as f32,
        y: (9.6 * 1.2) as f32,
        z: (10.1 * 1.2) as f32,
    },
    Vec3 {
        x: (34.5 * 1.2) as f32,
        y: (9.6 * 1.2) as f32,
        z: (6.1 * 1.2) as f32,
    },
    Vec3 {
        x: (34.5 * 1.2) as f32,
        y: (9.6 * 1.2) as f32,
        z: (6.1 * 1.2) as f32,
    },
    Vec3 {
        x: (17.) as f32,
        y: (-19.5) as f32,
        z: (62.9) as f32,
    },
    Vec3 {
        x: (-3.6) as f32,
        y: (-24.1) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-1.6) as f32,
        y: (-19.3) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-0.1) as f32,
        y: (-14.4) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (2.0) as f32,
        y: (-7.6) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (3.4) as f32,
        y: (1.3) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (3.7) as f32,
        y: (11.1) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-0.3) as f32,
        y: (22.3) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-6.) as f32,
        y: (33.) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-9.3) as f32,
        y: (36.4) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-7.) as f32,
        y: (35.) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-2.1) as f32,
        y: (29.) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (3.9) as f32,
        y: (17.3) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (6.1) as f32,
        y: (5.8) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (5.9) as f32,
        y: (-4.4) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (4.2) as f32,
        y: (-14.1) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (2.4) as f32,
        y: (-18.8) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (-1.8) as f32,
        y: (-25.5) as f32,
        z: (59.5) as f32,
    },
    Vec3 {
        x: (18.1) as f32,
        y: (7.8) as f32,
        z: (74.4) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (-47.1) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (78.5) as f32,
        y: (46.7) as f32,
        z: (96.) as f32,
    },
    Vec3 {
        x: (6.3) as f32,
        y: (-9.) as f32,
        z: (111.2) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (40.) as f32,
        z: (70.) as f32,
    },
    Vec3 {
        x: (56.) as f32,
        y: (-32.) as f32,
        z: (32.) as f32,
    },
    Vec3 {
        x: (56.) as f32,
        y: (32.) as f32,
        z: (32.) as f32,
    },
    Vec3 {
        x: (42.) as f32,
        y: (24.) as f32,
        z: (50.) as f32,
    },
    Vec3 {
        x: (20.) as f32,
        y: (0.) as f32,
        z: (0.) as f32,
    },
    Vec3 {
        x: (20.) as f32,
        y: (0.) as f32,
        z: (0.) as f32,
    },
    Vec3 {
        x: (20.) as f32,
        y: (0.) as f32,
        z: (0.) as f32,
    },
    Vec3 {
        x: (24.) as f32,
        y: (0.) as f32,
        z: (6.) as f32,
    },
    Vec3 {
        x: (1.7) as f32,
        y: (7.0) as f32,
        z: (11.3) as f32,
    },
    Vec3 {
        x: (44.0) as f32,
        y: (3.0) as f32,
        z: (14.4) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (0.) as f32,
        z: (6.) as f32,
    },
    Vec3 {
        x: (64.72) as f32,
        y: (14.50) as f32,
        z: (88.81) as f32,
    },
    Vec3 {
        x: (56.) as f32,
        y: (32.) as f32,
        z: (32.) as f32,
    },
    Vec3 {
        x: (62.) as f32,
        y: (-20.) as f32,
        z: (84.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (0.) as f32,
        z: (6.) as f32,
    },
    Vec3 {
        x: (61.) as f32,
        y: (-32.) as f32,
        z: (12.) as f32,
    },
    Vec3 {
        x: (61.) as f32,
        y: (32.) as f32,
        z: (12.) as f32,
    },
    Vec3 {
        x: (17.) as f32,
        y: (-62.) as f32,
        z: (91.) as f32,
    },
    Vec3 {
        x: (68.) as f32,
        y: (12.) as f32,
        z: (86.) as f32,
    },
    Vec3 {
        x: (47.5) as f32,
        y: (56.) as f32,
        z: (89.) as f32,
    },
    Vec3 {
        x: (54.) as f32,
        y: (52.) as f32,
        z: (91.) as f32,
    },
    Vec3 {
        x: (58.) as f32,
        y: (40.) as f32,
        z: (91.) as f32,
    },
    Vec3 {
        x: (68.) as f32,
        y: (30.) as f32,
        z: (88.) as f32,
    },
    Vec3 {
        x: (74.) as f32,
        y: (20.) as f32,
        z: (88.) as f32,
    },
    Vec3 {
        x: (73.) as f32,
        y: (11.) as f32,
        z: (87.) as f32,
    },
    Vec3 {
        x: (73.) as f32,
        y: (3.) as f32,
        z: (87.) as f32,
    },
    Vec3 {
        x: (70.) as f32,
        y: (-12.) as f32,
        z: (87.) as f32,
    },
    Vec3 {
        x: (67.) as f32,
        y: (-20.) as f32,
        z: (90.) as f32,
    },
    Vec3 {
        x: (-20.) as f32,
        y: (76.) as f32,
        z: (90.) as f32,
    },
    Vec3 {
        x: (-8.) as f32,
        y: (74.) as f32,
        z: (90.) as f32,
    },
    Vec3 {
        x: (0.) as f32,
        y: (72.) as f32,
        z: (90.) as f32,
    },
    Vec3 {
        x: (10.) as f32,
        y: (71.) as f32,
        z: (89.) as f32,
    },
    Vec3 {
        x: (23.) as f32,
        y: (70.) as f32,
        z: (87.) as f32,
    },
    Vec3 {
        x: (32.) as f32,
        y: (64.) as f32,
        z: (85.) as f32,
    },
    Vec3 {
        x: (40.) as f32,
        y: (58.) as f32,
        z: (84.) as f32,
    },
    Vec3 {
        x: (48.) as f32,
        y: (50.) as f32,
        z: (83.) as f32,
    },
    Vec3 {
        x: (54.) as f32,
        y: (42.) as f32,
        z: (82.) as f32,
    },
    Vec3 {
        x: (56.) as f32,
        y: (34.) as f32,
        z: (82.) as f32,
    },
    Vec3 {
        x: (58.) as f32,
        y: (26.) as f32,
        z: (82.) as f32,
    },
    Vec3 {
        x: (60.) as f32,
        y: (16.) as f32,
        z: (82.) as f32,
    },
    Vec3 {
        x: (59.) as f32,
        y: (6.) as f32,
        z: (81.) as f32,
    },
    Vec3 {
        x: (58.) as f32,
        y: (-2.) as f32,
        z: (80.) as f32,
    },
    Vec3 {
        x: (57.) as f32,
        y: (-10.) as f32,
        z: (79.) as f32,
    },
    Vec3 {
        x: (54.) as f32,
        y: (-18.) as f32,
        z: (78.) as f32,
    },
    Vec3 {
        x: (42.) as f32,
        y: (-32.) as f32,
        z: (80.) as f32,
    },
    Vec3 {
        x: (36.) as f32,
        y: (-40.) as f32,
        z: (78.) as f32,
    },
    Vec3 {
        x: (68.4) as f32,
        y: (10.88) as f32,
        z: (82.08) as f32,
    },
    Vec3 {
        x: (68.51) as f32,
        y: (8.64) as f32,
        z: (85.14) as f32,
    },
    Vec3 {
        x: (68.66) as f32,
        y: (6.38) as f32,
        z: (88.78) as f32,
    },
    Vec3 {
        x: (68.73) as f32,
        y: (5.1) as f32,
        z: (84.47) as f32,
    },
    Vec3 {
        x: (68.82) as f32,
        y: (4.79) as f32,
        z: (80.52) as f32,
    },
    Vec3 {
        x: (68.77) as f32,
        y: (6.11) as f32,
        z: (85.37) as f32,
    },
    Vec3 {
        x: (68.67) as f32,
        y: (7.99) as f32,
        z: (90.24) as f32,
    },
    Vec3 {
        x: (68.55) as f32,
        y: (9.54) as f32,
        z: (87.36) as f32,
    },
    Vec3 {
        x: (0.) as f32,
        y: (0.) as f32,
        z: (-5.) as f32,
    },
    Vec3 {
        x: (0.) as f32,
        y: (0.) as f32,
        z: (-5.) as f32,
    },
    Vec3 {
        x: (0.) as f32,
        y: (0.) as f32,
        z: (-5.) as f32,
    },
    Vec3 {
        x: (0.) as f32,
        y: (0.) as f32,
        z: (-5.) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-17.63) as f32,
        z: (93.77) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-17.08) as f32,
        z: (89.82) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-18.40) as f32,
        z: (90.70) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-18.34) as f32,
        z: (94.32) as f32,
    },
    Vec3 {
        x: (69.00) as f32,
        y: (-18.30) as f32,
        z: (97.98) as f32,
    },
    Vec3 {
        x: (45.04) as f32,
        y: (-59.02) as f32,
        z: (92.24) as f32,
    },
    Vec3 {
        x: (50.68) as f32,
        y: (-54.70) as f32,
        z: (91.96) as f32,
    },
    Vec3 {
        x: (56.57) as f32,
        y: (-47.72) as f32,
        z: (91.65) as f32,
    },
    Vec3 {
        x: (61.75) as f32,
        y: (-38.75) as f32,
        z: (91.38) as f32,
    },
    Vec3 {
        x: (65.55) as f32,
        y: (-28.76) as f32,
        z: (91.24) as f32,
    },
    Vec3 {
        x: (67.79) as f32,
        y: (-18.90) as f32,
        z: (91.22) as f32,
    },
    Vec3 {
        x: (68.60) as f32,
        y: (-9.52) as f32,
        z: (91.23) as f32,
    },
    Vec3 {
        x: (68.08) as f32,
        y: (0.18) as f32,
        z: (91.32) as f32,
    },
    Vec3 {
        x: (66.14) as f32,
        y: (9.79) as f32,
        z: (91.44) as f32,
    },
    Vec3 {
        x: (62.77) as f32,
        y: (18.91) as f32,
        z: (91.65) as f32,
    },
    Vec3 {
        x: (58.29) as f32,
        y: (27.11) as f32,
        z: (92.00) as f32,
    },
    Vec3 {
        x: (10.6 * 1.2) as f32,
        y: (7.7 * 1.2) as f32,
        z: (7.8 * 1.2) as f32,
    },
    Vec3 {
        x: (25.1 * 1.2) as f32,
        y: (3.6 * 1.2) as f32,
        z: (19.0 * 1.2) as f32,
    },
    Vec3 {
        x: (20.8 * 1.2) as f32,
        y: (10.1 * 1.2) as f32,
        z: (-2.7 * 1.2) as f32,
    },
    Vec3 {
        x: (7.6 * 1.2) as f32,
        y: (9.3 * 1.2) as f32,
        z: (0.8 * 1.2) as f32,
    },
    Vec3 {
        x: (30.5 * 1.2) as f32,
        y: (9.9 * 1.2) as f32,
        z: (-18.7 * 1.2) as f32,
    },
    Vec3 {
        x: (27.6 * 1.2) as f32,
        y: (3.4 * 1.2) as f32,
        z: (-10.4 * 1.2) as f32,
    },
    Vec3 {
        x: (28.9 * 1.2) as f32,
        y: (4.6 * 1.2) as f32,
        z: (-8.1 * 1.2) as f32,
    },
    Vec3 {
        x: (31.5 * 1.2) as f32,
        y: (9.6 * 1.2) as f32,
        z: (10.1 * 1.2) as f32,
    },
    Vec3 {
        x: (10.6 * 1.2) as f32,
        y: (7.7 * 1.2) as f32,
        z: (7.8 * 1.2) as f32,
    },
    Vec3 {
        x: (25.1 * 1.2) as f32,
        y: (3.6 * 1.2) as f32,
        z: (19.0 * 1.2) as f32,
    },
    Vec3 {
        x: (20.8 * 1.2) as f32,
        y: (10.1 * 1.2) as f32,
        z: (-2.7 * 1.2) as f32,
    },
    Vec3 {
        x: (7.6 * 1.2) as f32,
        y: (9.3 * 1.2) as f32,
        z: (0.8 * 1.2) as f32,
    },
    Vec3 {
        x: (30.5 * 1.2) as f32,
        y: (9.9 * 1.2) as f32,
        z: (-18.7 * 1.2) as f32,
    },
    Vec3 {
        x: (27.6 * 1.2) as f32,
        y: (3.4 * 1.2) as f32,
        z: (-10.4 * 1.2) as f32,
    },
    Vec3 {
        x: (28.9 * 1.2) as f32,
        y: (4.6 * 1.2) as f32,
        z: (-8.1 * 1.2) as f32,
    },
    Vec3 {
        x: (31.5 * 1.2) as f32,
        y: (9.6 * 1.2) as f32,
        z: (10.1 * 1.2) as f32,
    },
    Vec3 {
        x: (88.) as f32,
        y: (50.) as f32,
        z: (60.) as f32,
    },
    Vec3 {
        x: (58.) as f32,
        y: (20.) as f32,
        z: (17.2) as f32,
    },
    Vec3 {
        x: (64.) as f32,
        y: (-22.) as f32,
        z: (24.) as f32,
    },
    Vec3 {
        x: (37.) as f32,
        y: (13.) as f32,
        z: (72.) as f32,
    },
    Vec3 {
        x: (58.) as f32,
        y: (-25.) as f32,
        z: (72.) as f32,
    },
    Vec3 {
        x: (34.) as f32,
        y: (11.) as f32,
        z: (13.) as f32,
    },
    Vec3 {
        x: (28.) as f32,
        y: (13.) as f32,
        z: (10.5) as f32,
    },
    Vec3 {
        x: (29.) as f32,
        y: (13.) as f32,
        z: (8.5) as f32,
    },
    Vec3 {
        x: (30.) as f32,
        y: (12.5) as f32,
        z: (12.) as f32,
    },
    Vec3 {
        x: (29.) as f32,
        y: (12.5) as f32,
        z: (14.7) as f32,
    },
    Vec3 {
        x: (30.) as f32,
        y: (6.5) as f32,
        z: (12.) as f32,
    },
    Vec3 {
        x: (29.) as f32,
        y: (1.5) as f32,
        z: (8.5) as f32,
    },
    Vec3 {
        x: (29.) as f32,
        y: (6.0) as f32,
        z: (10.) as f32,
    },
    Vec3 {
        x: (25.0) as f32,
        y: (11.) as f32,
        z: (21.) as f32,
    },
    Vec3 {
        x: (26.5) as f32,
        y: (5.) as f32,
        z: (21.) as f32,
    },
    Vec3 {
        x: (27.) as f32,
        y: (6.5) as f32,
        z: (4.0) as f32,
    },
    Vec3 {
        x: (28.) as f32,
        y: (4.) as f32,
        z: (4.0) as f32,
    },
    Vec3 {
        x: (27.) as f32,
        y: (1.7) as f32,
        z: (4.0) as f32,
    },
    Vec3 {
        x: (21.7) as f32,
        y: (-1.5) as f32,
        z: (22.5) as f32,
    },
    Vec3 {
        x: (22.) as f32,
        y: (0.) as f32,
        z: (20.5) as f32,
    },
    Vec3 {
        x: (22.5) as f32,
        y: (3.7) as f32,
        z: (20.5) as f32,
    },
    Vec3 {
        x: (8.0) as f32,
        y: (40.0) as f32,
        z: (18.0) as f32,
    },
    Vec3 {
        x: (29.0) as f32,
        y: (16.0) as f32,
        z: (19.0) as f32,
    },
    Vec3 {
        x: (4.7) as f32,
        y: (-30.0) as f32,
        z: (20.0) as f32,
    },
    Vec3 {
        x: (36.33) as f32,
        y: (12.24) as f32,
        z: (-17.39) as f32,
    },
    Vec3 {
        x: (36.33) as f32,
        y: (12.24) as f32,
        z: (-17.39) as f32,
    },
    Vec3 {
        x: (36.33) as f32,
        y: (12.24) as f32,
        z: (-17.39) as f32,
    },
    Vec3 {
        x: (36.33) as f32,
        y: (12.24) as f32,
        z: (-17.39) as f32,
    },
    Vec3 {
        x: (36.33) as f32,
        y: (12.24) as f32,
        z: (-17.39) as f32,
    },
    Vec3 {
        x: (36.) as f32,
        y: (-6.2) as f32,
        z: (19.59) as f32,
    },
    Vec3 {
        x: (36.) as f32,
        y: (-6.2) as f32,
        z: (19.59) as f32,
    },
    Vec3 {
        x: (36.) as f32,
        y: (-6.2) as f32,
        z: (19.59) as f32,
    },
    Vec3 {
        x: (36.) as f32,
        y: (-6.2) as f32,
        z: (19.59) as f32,
    },
    Vec3 {
        x: (14.8) as f32,
        y: (10.5) as f32,
        z: (8.82) as f32,
    },
    Vec3 {
        x: (31.31) as f32,
        y: (-37.) as f32,
        z: (54.32) as f32,
    },
    Vec3 {
        x: (31.31) as f32,
        y: (37.) as f32,
        z: (54.32) as f32,
    },
    Vec3 {
        x: (1.7) as f32,
        y: (-7.0) as f32,
        z: (11.3) as f32,
    },
    Vec3 {
        x: (1.7) as f32,
        y: (-7.0) as f32,
        z: (11.3) as f32,
    },
    Vec3 {
        x: (33.0 + 1.) as f32,
        y: (12.5) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (32.4 + 1.) as f32,
        y: (11.2) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (35.6 + 1.) as f32,
        y: (7.4) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (34.0 + 1.) as f32,
        y: (4.1) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (36.6 + 1.) as f32,
        y: (1.0) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (34.7 + 1.) as f32,
        y: (-1.9) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (36.6 + 1.) as f32,
        y: (-0.5) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (34.2 + 1.) as f32,
        y: (2.8) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (36.5 + 1.) as f32,
        y: (3.8) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (33.5 + 1.) as f32,
        y: (6.9) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (32.7 + 1.) as f32,
        y: (9.9) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (34.5 + 1.) as f32,
        y: (11.0) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (33.0 + 1.) as f32,
        y: (12.5) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (32.4 + 1.) as f32,
        y: (11.2) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (35.6 + 1.) as f32,
        y: (7.4) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (34.0 + 1.) as f32,
        y: (4.1) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (36.6 + 1.) as f32,
        y: (1.0) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (34.7 + 1.) as f32,
        y: (-1.9) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (36.6 + 1.) as f32,
        y: (-0.5) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (34.2 + 1.) as f32,
        y: (2.8) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (36.5 + 1.) as f32,
        y: (3.8) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (33.5 + 1.) as f32,
        y: (6.9) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (32.7 + 1.) as f32,
        y: (9.9) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (34.5 + 1.) as f32,
        y: (11.0) as f32,
        z: (15.0) as f32,
    },
    Vec3 {
        x: (0.0) as f32,
        y: (0.0) as f32,
        z: (0.0) as f32,
    },
];

/// Muzzle id `INFANTRY_MACHINEGUN_1`.
pub const INFANTRY_MACHINEGUN_1: usize = 26;
/// Muzzle id `INFANTRY_MACHINEGUN_2`.
pub const INFANTRY_MACHINEGUN_2: usize = 27;
/// Muzzle id `INFANTRY_MACHINEGUN_3`.
pub const INFANTRY_MACHINEGUN_3: usize = 28;
/// Muzzle id `INFANTRY_MACHINEGUN_4`.
pub const INFANTRY_MACHINEGUN_4: usize = 29;
/// Muzzle id `INFANTRY_MACHINEGUN_5`.
pub const INFANTRY_MACHINEGUN_5: usize = 30;
/// Muzzle id `INFANTRY_MACHINEGUN_6`.
pub const INFANTRY_MACHINEGUN_6: usize = 31;
/// Muzzle id `INFANTRY_MACHINEGUN_7`.
pub const INFANTRY_MACHINEGUN_7: usize = 32;
/// Muzzle id `INFANTRY_MACHINEGUN_8`.
pub const INFANTRY_MACHINEGUN_8: usize = 33;
/// Muzzle id `INFANTRY_MACHINEGUN_9`.
pub const INFANTRY_MACHINEGUN_9: usize = 34;
/// Muzzle id `INFANTRY_MACHINEGUN_10`.
pub const INFANTRY_MACHINEGUN_10: usize = 35;
/// Muzzle id `INFANTRY_MACHINEGUN_11`.
pub const INFANTRY_MACHINEGUN_11: usize = 36;
/// Muzzle id `INFANTRY_MACHINEGUN_12`.
pub const INFANTRY_MACHINEGUN_12: usize = 37;
/// Muzzle id `INFANTRY_MACHINEGUN_13`.
pub const INFANTRY_MACHINEGUN_13: usize = 38;
/// Muzzle id `SOLDIER_BLASTER_1`.
pub const SOLDIER_BLASTER_1: usize = 39;
/// Muzzle id `SOLDIER_BLASTER_2`.
pub const SOLDIER_BLASTER_2: usize = 40;
/// Muzzle id `SOLDIER_SHOTGUN_1`.
pub const SOLDIER_SHOTGUN_1: usize = 41;
/// Muzzle id `SOLDIER_SHOTGUN_2`.
pub const SOLDIER_SHOTGUN_2: usize = 42;
/// Muzzle id `SOLDIER_MACHINEGUN_1`.
pub const SOLDIER_MACHINEGUN_1: usize = 43;
/// Muzzle id `SOLDIER_MACHINEGUN_2`.
pub const SOLDIER_MACHINEGUN_2: usize = 44;
/// Muzzle id `SOLDIER_BLASTER_3`.
pub const SOLDIER_BLASTER_3: usize = 83;
/// Muzzle id `SOLDIER_SHOTGUN_3`.
pub const SOLDIER_SHOTGUN_3: usize = 84;
/// Muzzle id `SOLDIER_MACHINEGUN_3`.
pub const SOLDIER_MACHINEGUN_3: usize = 85;
/// Muzzle id `SOLDIER_BLASTER_4`.
pub const SOLDIER_BLASTER_4: usize = 86;
/// Muzzle id `SOLDIER_SHOTGUN_4`.
pub const SOLDIER_SHOTGUN_4: usize = 87;
/// Muzzle id `SOLDIER_MACHINEGUN_4`.
pub const SOLDIER_MACHINEGUN_4: usize = 88;
/// Muzzle id `SOLDIER_BLASTER_5`.
pub const SOLDIER_BLASTER_5: usize = 89;
/// Muzzle id `SOLDIER_SHOTGUN_5`.
pub const SOLDIER_SHOTGUN_5: usize = 90;
/// Muzzle id `SOLDIER_MACHINEGUN_5`.
pub const SOLDIER_MACHINEGUN_5: usize = 91;
/// Muzzle id `SOLDIER_BLASTER_6`.
pub const SOLDIER_BLASTER_6: usize = 92;
/// Muzzle id `SOLDIER_SHOTGUN_6`.
pub const SOLDIER_SHOTGUN_6: usize = 93;
/// Muzzle id `SOLDIER_MACHINEGUN_6`.
pub const SOLDIER_MACHINEGUN_6: usize = 94;
/// Muzzle id `SOLDIER_BLASTER_7`.
pub const SOLDIER_BLASTER_7: usize = 95;
/// Muzzle id `SOLDIER_SHOTGUN_7`.
pub const SOLDIER_SHOTGUN_7: usize = 96;
/// Muzzle id `SOLDIER_MACHINEGUN_7`.
pub const SOLDIER_MACHINEGUN_7: usize = 97;
/// Muzzle id `SOLDIER_BLASTER_8`.
pub const SOLDIER_BLASTER_8: usize = 98;
/// Muzzle id `SOLDIER_SHOTGUN_8`.
pub const SOLDIER_SHOTGUN_8: usize = 99;
/// Muzzle id `SOLDIER_MACHINEGUN_8`.
pub const SOLDIER_MACHINEGUN_8: usize = 100;
/// Muzzle id `SOLDIER_RIPPER_1`.
pub const SOLDIER_RIPPER_1: usize = 211;
/// Muzzle id `SOLDIER_RIPPER_2`.
pub const SOLDIER_RIPPER_2: usize = 212;
/// Muzzle id `SOLDIER_RIPPER_3`.
pub const SOLDIER_RIPPER_3: usize = 213;
/// Muzzle id `SOLDIER_RIPPER_4`.
pub const SOLDIER_RIPPER_4: usize = 214;
/// Muzzle id `SOLDIER_RIPPER_5`.
pub const SOLDIER_RIPPER_5: usize = 215;
/// Muzzle id `SOLDIER_RIPPER_6`.
pub const SOLDIER_RIPPER_6: usize = 216;
/// Muzzle id `SOLDIER_RIPPER_7`.
pub const SOLDIER_RIPPER_7: usize = 217;
/// Muzzle id `SOLDIER_RIPPER_8`.
pub const SOLDIER_RIPPER_8: usize = 218;
/// Muzzle id `SOLDIER_HYPERGUN_1`.
pub const SOLDIER_HYPERGUN_1: usize = 219;
/// Muzzle id `SOLDIER_HYPERGUN_2`.
pub const SOLDIER_HYPERGUN_2: usize = 220;
/// Muzzle id `SOLDIER_HYPERGUN_3`.
pub const SOLDIER_HYPERGUN_3: usize = 221;
/// Muzzle id `SOLDIER_HYPERGUN_4`.
pub const SOLDIER_HYPERGUN_4: usize = 222;
/// Muzzle id `SOLDIER_HYPERGUN_5`.
pub const SOLDIER_HYPERGUN_5: usize = 223;
/// Muzzle id `SOLDIER_HYPERGUN_6`.
pub const SOLDIER_HYPERGUN_6: usize = 224;
/// Muzzle id `SOLDIER_HYPERGUN_7`.
pub const SOLDIER_HYPERGUN_7: usize = 225;
/// Muzzle id `SOLDIER_HYPERGUN_8`.
pub const SOLDIER_HYPERGUN_8: usize = 226;
/// Muzzle id `INFANTRY_MACHINEGUN_14`.
pub const INFANTRY_MACHINEGUN_14: usize = 232;
/// Muzzle id `INFANTRY_MACHINEGUN_15`.
pub const INFANTRY_MACHINEGUN_15: usize = 233;
/// Muzzle id `INFANTRY_MACHINEGUN_16`.
pub const INFANTRY_MACHINEGUN_16: usize = 234;
/// Muzzle id `INFANTRY_MACHINEGUN_17`.
pub const INFANTRY_MACHINEGUN_17: usize = 235;
/// Muzzle id `INFANTRY_MACHINEGUN_18`.
pub const INFANTRY_MACHINEGUN_18: usize = 236;
/// Muzzle id `INFANTRY_MACHINEGUN_19`.
pub const INFANTRY_MACHINEGUN_19: usize = 237;
/// Muzzle id `INFANTRY_MACHINEGUN_20`.
pub const INFANTRY_MACHINEGUN_20: usize = 238;
/// Muzzle id `INFANTRY_MACHINEGUN_21`.
pub const INFANTRY_MACHINEGUN_21: usize = 239;
/// Muzzle id `SOLDIER_BLASTER_9`.
pub const SOLDIER_BLASTER_9: usize = 251;
/// Muzzle id `SOLDIER_SHOTGUN_9`.
pub const SOLDIER_SHOTGUN_9: usize = 252;
/// Muzzle id `SOLDIER_MACHINEGUN_9`.
pub const SOLDIER_MACHINEGUN_9: usize = 253;
/// Muzzle id `SOLDIER_RIPPER_9`.
pub const SOLDIER_RIPPER_9: usize = 254;
/// Muzzle id `SOLDIER_HYPERGUN_9`.
pub const SOLDIER_HYPERGUN_9: usize = 255;
/// Muzzle id `INFANTRY_MACHINEGUN_22`.
pub const INFANTRY_MACHINEGUN_22: usize = 260;

/// Muzzle offset for an edition and flash number (`muzzleOffset`).
pub fn muzzle_offset(edition: Q2Edition, flash: usize) -> Vec3 {
    *record_at(
        if edition == Q2Edition::Classic {
            &CLASSIC
        } else {
            &RERELEASE
        },
        flash,
    )
}
