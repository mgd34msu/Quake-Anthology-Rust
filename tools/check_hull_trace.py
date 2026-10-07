#!/usr/bin/env python3
"""Compare the iterative tracer with functions extracted from original world.c."""
import argparse
import json
import os
from pathlib import Path
import random
import re
import struct
import subprocess
import time
from frame_timings import pinned_cores


def function(source, name):
    match = re.search(r"^\w+\s+" + re.escape(name) + r" \([^;\n]*\)\n\{", source, re.MULTILINE)
    if match is None:
        raise ValueError("reference function definition missing: " + name)
    start = match.start()
    brace = source.index("{", start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def geometry(pak_path):
    pak = pak_path.read_bytes()
    if pak[:4] != b"PACK":
        raise ValueError("not a PAK")
    offset, size = struct.unpack_from("<II", pak, 4)
    bsp = None
    for position in range(offset, offset + size, 64):
        name, start, length = struct.unpack_from("<56sII", pak, position)
        if name.split(b"\0", 1)[0] == b"maps/e1m1.bsp":
            bsp = pak[start:start + length]
            break
    if bsp is None or struct.unpack_from("<I", bsp)[0] != 29:
        raise ValueError("e1m1 BSP29 required")
    lumps = [bsp[start:start + length] for start, length in struct.iter_unpack("<II", bsp[4:124])]
    planes = lumps[1]
    leaves = [struct.unpack_from("<i", lumps[10], offset)[0] for offset in range(0, len(lumps[10]), 28)]
    drawing = []
    for offset in range(0, len(lumps[5]), 24):
        plane, front, back = struct.unpack_from("<Ihh", lumps[5], offset)
        drawing.append((plane, *(child if child >= 0 else leaves[-1 - child] for child in (front, back))))
    clips = list(struct.iter_unpack("<Ihh", lumps[9]))
    roots = struct.unpack_from("<3i", lumps[14], 36)
    bounds = struct.unpack_from("<6f", lumps[14])
    data = struct.pack("<3I3i", len(planes) // 20, len(drawing), len(clips), *roots) + planes
    data += b"".join(struct.pack("<Iii", *node) for node in drawing + clips)
    rng = random.Random(0x5155414B)
    cases = []
    for index in range(10_000):
        start = [rng.uniform(bounds[axis] - 32, bounds[axis + 3] + 32) for axis in range(3)]
        end = ([value + rng.uniform(-64, 64) for value in start] if index % 2 else
               [rng.uniform(bounds[axis] - 32, bounds[axis + 3] + 32) for axis in range(3)])
        cases.append(struct.pack("<I6f", index % 3, *start, *end))
    return data + b"".join(cases), {"planes": len(planes) // 20, "drawing_nodes": len(drawing), "clip_nodes": len(clips), "roots": roots, "seed": "0x5155414B", "cases": len(cases)}


PREFIX = r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef int qboolean;
typedef float vec3_t[3];
typedef struct { float normal[3], dist; int type; } mplane_t;
typedef struct { uint32_t planenum; int32_t children[2]; } dclipnode_t;
typedef struct { dclipnode_t *clipnodes; mplane_t *planes; int firstclipnode, lastclipnode; } hull_t;
typedef struct { int allsolid, startsolid, inopen, inwater; float fraction; vec3_t endpos; mplane_t plane; } trace_t;
static vec3_t vec3_origin;
#define true 1
#define false 0
#define CONTENTS_SOLID -2
#define CONTENTS_EMPTY -1
#define DIST_EPSILON (0.03125)
#define DotProduct(a,b) ((a)[0]*(b)[0]+(a)[1]*(b)[1]+(a)[2]*(b)[2])
#define VectorCopy(a,b) memcpy((b),(a),sizeof(vec3_t))
#define VectorSubtract(a,b,c) do { for(int v=0;v<3;v++) (c)[v]=(a)[v]-(b)[v]; } while(0)
#define Sys_Error(...) exit(2)
#define Con_DPrintf(...) ((void)0)
'''

SUFFIX = r'''
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    FILE *in=fopen(argv[1],"rb"), *out=fopen(argv[2],"wb");
    uint32_t counts[3]; int32_t roots[3];
    if (!in || !out || fread(counts,4,3,in)!=3 || fread(roots,4,3,in)!=3) return 2;
    mplane_t *planes=calloc(counts[0],sizeof(*planes));
    dclipnode_t *draw=calloc(counts[1],sizeof(*draw)), *clip=calloc(counts[2],sizeof(*clip));
    if (!planes || !draw || !clip || fread(planes,sizeof(*planes),counts[0],in)!=counts[0] ||
        fread(draw,sizeof(*draw),counts[1],in)!=counts[1] || fread(clip,sizeof(*clip),counts[2],in)!=counts[2]) return 2;
    uint32_t index;
    while (fread(&index,4,1,in)==1) {
        vec3_t start,end;
        if (index>2 || fread(start,4,3,in)!=3 || fread(end,4,3,in)!=3) return 2;
        hull_t hull={index ? clip : draw, planes, roots[index], (int)counts[index ? 2 : 1]-1};
        trace_t trace={0}; trace.allsolid=1; trace.fraction=1; VectorCopy(end,trace.endpos);
        SV_RecursiveHullCheck(&hull,hull.firstclipnode,0,1,start,end,&trace);
        uint32_t flags=trace.startsolid | trace.allsolid<<1 | trace.inopen<<2 | trace.inwater<<3;
        fwrite(&trace.fraction,4,1,out); fwrite(trace.endpos,4,3,out);
        fwrite(trace.plane.normal,4,3,out); fwrite(&trace.plane.dist,4,1,out); fwrite(&flags,4,1,out);
    }
    fclose(in); fclose(out); free(planes); free(draw); free(clip); return 0;
}
'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pak", type=Path, required=True)
    parser.add_argument("--qsrc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    world = args.qsrc / "quake/WinQuake/world.c"
    source = world.read_text()
    reference = args.output / "reference.c"
    reference.write_text(PREFIX + function(source, "SV_HullPointContents") + "\n" + function(source, "SV_RecursiveHullCheck") + SUFFIX)
    payload, workload = geometry(args.pak)
    data = args.output / "segments.bin"
    data.write_bytes(payload)
    expected = args.output / "expected.bin"
    executable = args.output / "reference"
    subprocess.run(["cc", "-O3", "-ffp-contract=off", str(reference), "-o", str(executable)], check=True, timeout=300)
    subprocess.run([str(executable), str(data), str(expected)], check=True, timeout=300)
    started = time.monotonic()
    env = dict(os.environ, CARGO_TARGET_DIR="target", RUSTFLAGS="")
    build = subprocess.run(["cargo", "build", "--release", "-p", "qa-world", "--example", "hull_trace"], env=env, capture_output=True, text=True, timeout=300)
    (args.output / "build.log").write_text(build.stdout + build.stderr)
    build.check_returncode()
    build_seconds = time.monotonic() - started
    cores = pinned_cores()
    run = subprocess.run(["taskset", "-c", cores, "target/release/examples/hull_trace", str(data), str(expected)], capture_output=True, text=True, timeout=300)
    (args.output / "run.log").write_text(run.stdout + run.stderr)
    run.check_returncode()
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], text=True))
    report = {"scope": "headless retail hull workload; no gameplay/install", "commit": commit, "source_tree_dirty": dirty, "reference": str(world), "c_flags": "-O3 -ffp-contract=off", "cpu_affinity": cores, "debugger": False, "target_cpu": "baseline", "build_seconds": build_seconds, "workload": workload, "result": json.loads(run.stdout), "muse_historical_us": 229, "comparison_limit": "Muse's historical workload was not remeasured; this is not a controlled before/after comparison"}
    (args.output / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
