#!/usr/bin/env python3
"""Compare shared entity lifetimes with unchanged original C allocation loops.

This is a headless developer check. It does not qualify gameplay, installation,
or native module ABI compatibility. Generation numbers are probe observations
of native lifetime transitions; original C entities have no generation handles.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time

from check_hull_trace import function


COMMON = r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <setjmp.h>
#include <stdarg.h>
static jmp_buf failure;
static _Noreturn void fatal(const char *format, ...) {
    (void)format;
    longjmp(failure, 1);
}
static void ignore_print(const char *format, ...) { (void)format; }
#define true 1
#define false 0
#define qtrue 1
#define qfalse 0
typedef int qboolean;
'''

EDICT = r'''
typedef float vec3_t[3];
typedef struct {
    float model, takedamage, modelindex, colormap, skin, frame;
    vec3_t origin, angles;
    float nextthink, solid;
} entvars_t;
typedef struct { int free; float freetime; entvars_t v; } edict_t;
static edict_t native_entities[MAX_EDICTS];
static struct { double time; int num_edicts; } sv;
static struct { int maxclients; } svs;
static struct { int entityfields; } program;
static typeof(program) *progs = &program;
static vec3_t vec3_origin;
#define EDICT_NUM(i) (&native_entities[(i)])
#define VectorCopy(a,b) memcpy((b),(a),sizeof(vec3_t))
#define Sys_Error fatal
#define Con_Printf ignore_print
static void SV_UnlinkEdict(edict_t *e) { (void)e; }
#define ENTITY edict_t
#define STORAGE MAX_EDICTS
#define ALLOC ED_Alloc
#define FREE ED_Free
#define LIVE(i) (!native_entities[(i)].free)
#define SET_TIME(value) (sv.time = strtod((value), NULL))
'''

Q2 = r'''
typedef struct {
    int inuse;
    const char *classname;
    float gravity, freetime;
    struct { int number; } s;
} edict_t;
static edict_t native_entities[MAX_EDICTS];
static edict_t *g_edicts = native_entities;
static struct { float value; } clients_value;
static typeof(clients_value) *maxclients = &clients_value;
static struct { int num_edicts; } globals;
static struct { int maxentities; } game;
static struct { float time; } level;
static void unlink_entity(edict_t *e) { (void)e; }
static struct {
    void (*error)(const char *, ...);
    void (*unlinkentity)(edict_t *);
} gi = { fatal, unlink_entity };
#define ENTITY edict_t
#define STORAGE MAX_EDICTS
#define ALLOC G_Spawn
#define FREE G_FreeEdict
#define LIVE(i) native_entities[(i)].inuse
#define SET_TIME(value) (level.time = strtof((value), NULL))
'''

Q3 = r'''
typedef struct {
    int inuse, neverFree, freetime;
    const char *classname;
    struct { int number; } s;
    struct { int ownerNum; } r;
} gentity_t;
static gentity_t g_entities[MAX_GENTITIES];
#define native_entities g_entities
static struct { int ps; } client_storage[MAX_CLIENTS];
static struct {
    int startTime, time, num_entities;
    gentity_t *gentities;
    typeof(client_storage[0]) *clients;
} level;
static void trap_UnlinkEntity(gentity_t *e) { (void)e; }
static void trap_LocateGameData(gentity_t *entities, int count, int stride,
                               int *players, int player_stride) {
    (void)entities; (void)count; (void)stride; (void)players; (void)player_stride;
}
#define G_Printf ignore_print
#define G_Error fatal
#define ENTITY gentity_t
#define STORAGE MAX_GENTITIES
#define ALLOC G_Spawn
#define FREE G_FreeEntity
#define LIVE(i) native_entities[(i)].inuse
#define SET_TIME(value) (level.time = (int)strtol((value), NULL, 10))
'''

DRIVER = r'''
/* The observer adds generations without changing any native allocator body. */
static uint32_t generations[STORAGE], handles[STORAGE];
static int capacity, reserved, live_count, step;
static char case_name[80];

static void row(const char *op, int slot, uint32_t generation, int accepted,
                int displaced_slot, uint32_t displaced_generation) {
    printf("%s %d %s %d %u %d %d %u %d\n", case_name, step++, op, slot,
           generation, accepted, displaced_slot, displaced_generation, live_count);
}

static void snapshot(void) {
    if (!case_name[0]) return;
    for (int i = 0; i < capacity; ++i)
        row("final", i, generations[i], LIVE(i), -1, 0);
}

static void reset_case(const char *name, int size, int prefix, int start) {
    if (size != NORMAL_CAPACITY || prefix < 1 || prefix > size) exit(3);
    snapshot();
    memset(native_entities, 0, sizeof(native_entities));
    memset(handles, 0, sizeof(handles));
    for (int i = 0; i < STORAGE; ++i) generations[i] = 1;
    native_reset(prefix, start);
    capacity = size;
    reserved = prefix;
    live_count = prefix;
    for (int i = 0; i < prefix; ++i) handles[i] = 1;
    strcpy(case_name, name);
    step = 0;
}

static void allocate_one(void) {
    ENTITY *e;
    /* A longjmp observes the original fatal exhaustion branch without exit. */
    if (setjmp(failure)) {
        row("alloc", -1, 0, 0, -1, 0);
        return;
    }
    /* Capture liveness before ALLOC, which itself clears or initializes e. */
    int before[STORAGE];
    for (int i = 0; i < capacity; ++i) before[i] = LIVE(i);
    e = ALLOC();
    int slot = (int)(e - native_entities);
    if (slot < reserved || slot >= capacity) exit(4);
    int displaced_slot = -1;
    uint32_t displaced_generation = 0;
    if (before[slot]) {
        displaced_slot = slot;
        displaced_generation = generations[slot];
        ++generations[slot];
    } else {
        ++live_count;
    }
    handles[slot] = generations[slot];
    row("alloc", slot, handles[slot], 1, displaced_slot, displaced_generation);
}

static void free_one(int slot) {
    if (slot < 0 || slot >= capacity || !handles[slot]) exit(5);
    int was_live = LIVE(slot);
    /* Stale handles are observer-only; native C has no stale-handle guard. */
    if (was_live && handles[slot] == generations[slot]) FREE(&native_entities[slot]);
    int accepted = was_live && !LIVE(slot);
    if (accepted) {
        ++generations[slot];
        --live_count;
    }
    row("free", slot, handles[slot], accepted, -1, 0);
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--synthetic")) return synthetic();
    if (argc != 2) return 2;
    FILE *input = fopen(argv[1], "r");
    if (!input) return 2;
    char line[240], command[24], time_text[80], name[80];
    int a, b, c;
    while (fgets(line, sizeof(line), input)) {
        if (line[0] == '#' || line[0] == '\n') continue;
        if (sscanf(line, "%23s", command) != 1) return 6;
        if (!strcmp(command, "reset")) {
            if (sscanf(line, "%*s %79s %d %d %d", name, &a, &b, &c) != 4) return 6;
            reset_case(name, a, b, c);
        } else if (!strcmp(command, "alloc")) {
            if (!case_name[0] || sscanf(line, "%*s %79s %d", time_text, &a) != 2 || a < 1 || a > STORAGE) return 6;
            SET_TIME(time_text);
            for (int i = 0; i < a; ++i) allocate_one();
        } else if (!strcmp(command, "free")) {
            if (!case_name[0] || sscanf(line, "%*s %79s %d", time_text, &a) != 2) return 6;
            SET_TIME(time_text);
            free_one(a);
        } else if (!strcmp(command, "protect")) {
            if (sscanf(line, "%*s %d %d", &a, &b) != 2 || a < 0 || a >= capacity || (b != 0 && b != 1)) return 6;
            int accepted = native_protect(a, b);
            row("protect", a, handles[a], accepted, -1, 0);
        } else if (!strcmp(command, "check")) {
            if (sscanf(line, "%*s %d %d", &a, &b) != 2 || a < 0 || a >= capacity || b < 1) return 6;
            row("check", a, (uint32_t)b, LIVE(a) && generations[a] == (uint32_t)b, -1, 0);
        } else return 6;
    }
    snapshot();
    fclose(input);
    return case_name[0] ? 0 : 6;
}
'''


def macro(source, name):
    match = re.search(r"^\s*#\s*define\s+" + re.escape(name) + r"\b[^\n]*", source, re.MULTILINE)
    if match is None:
        raise ValueError("native constant missing: " + name)
    return match[0].strip() + "\n"


def native_source(qsrc, family):
    references = []

    def read(path):
        return (qsrc / path).read_text()

    def extract(path, names):
        source = read(path)
        bodies = []
        for name in names:
            body = function(source, name)
            offset = source.index(body)
            references.append({"path": path, "function": name,
                               "line": source[:offset].count("\n") + 1})
            bodies.append(body)
        return "\n\n".join(bodies) + "\n"

    if family in ("q1", "qw"):
        if family == "q1":
            definitions = macro(read("quake/WinQuake/quakedef.h"), "MAX_EDICTS")
            path = "quake/WinQuake/pr_edict.c"
            reset = "svs.maxclients = prefix - 1;"
        else:
            definitions = macro(read("quake/QW/client/bothdefs.h"), "MAX_EDICTS")
            definitions += macro(read("quake/QW/client/protocol.h"), "MAX_CLIENTS")
            path = "quake/QW/server/pr_edict.c"
            reset = "if (prefix != MAX_CLIENTS + 1) exit(3);"
        bodies = extract(path, ["ED_ClearEdict", "ED_Alloc", "ED_Free"])
        setup = r'''
#define NORMAL_CAPACITY MAX_EDICTS
static void native_reset(int prefix, int start) {
    (void)start;
    RESET_CLIENTS
    sv.num_edicts = prefix;
    sv.time = 0;
    program.entityfields = sizeof(entvars_t) / 4;
    for (int i = 0; i < MAX_EDICTS; ++i) native_entities[i].free = i >= prefix;
}
static int native_protect(int slot, int value) { (void)slot; (void)value; return 0; }
static int synthetic(void) { return 2; }
'''.replace("RESET_CLIENTS", reset)
        return COMMON + definitions + EDICT + bodies + setup + DRIVER, references
    if family == "q2":
        definitions = macro(read("quake-2/game/q_shared.h"), "MAX_EDICTS")
        definitions += macro(read("quake-2/game/g_local.h"), "BODY_QUEUE_SIZE")
        bodies = extract("quake-2/game/g_utils.c", ["G_InitEdict", "G_Spawn", "G_FreeEdict"])
        setup = r'''
#define NORMAL_CAPACITY MAX_EDICTS
static void native_reset(int prefix, int start) {
    (void)start;
    maxclients->value = prefix - BODY_QUEUE_SIZE - 1;
    if (maxclients->value < 1) exit(3);
    globals.num_edicts = (int)maxclients->value + 1;
    game.maxentities = MAX_EDICTS;
    level.time = 0;
    for (int i = 0; i < globals.num_edicts; ++i) G_InitEdict(&native_entities[i]);
    /* Native body queue setup allocates its eight rows with G_Spawn. */
    for (int i = 0; i < BODY_QUEUE_SIZE; ++i) G_Spawn();
    if (globals.num_edicts != prefix) exit(3);
}
static int native_protect(int slot, int value) { (void)slot; (void)value; return 0; }
static int synthetic(void) { return 2; }
'''
        return COMMON + definitions + Q2 + bodies + setup + DRIVER, references
    definitions = "".join(macro(read("quake-iii-arena/code/game/q_shared.h"), name)
                          for name in ("MAX_CLIENTS", "GENTITYNUM_BITS", "MAX_GENTITIES",
                                       "ENTITYNUM_NONE", "ENTITYNUM_WORLD", "ENTITYNUM_MAX_NORMAL"))
    bodies = extract("quake-iii-arena/code/game/g_utils.c",
                     ["G_InitGentity", "G_Spawn", "G_FreeEntity"])
    setup = r'''
#define NORMAL_CAPACITY ENTITYNUM_MAX_NORMAL
static void native_reset(int prefix, int start) {
    if (prefix != MAX_CLIENTS) exit(3);
    level.startTime = start;
    level.time = start;
    level.num_entities = prefix;
    level.gentities = native_entities;
    level.clients = client_storage;
    for (int i = 0; i < prefix; ++i) G_InitGentity(&native_entities[i]);
}
static int native_protect(int slot, int value) {
    if (!LIVE(slot)) return 0;
    native_entities[slot].neverFree = value;
    return 1;
}
static int synthetic(void) {
    /* Defined force-pass observation only: never call with every row live. */
    native_reset(MAX_CLIENTS, 12345);
    for (int i = 0; i < MAX_GENTITIES; ++i) G_InitGentity(&native_entities[i]);
    level.num_entities = MAX_GENTITIES;
    level.time = 21001;
    G_FreeEntity(&native_entities[MAX_CLIENTS]);
    level.time = 21002;
    if (setjmp(failure)) return 7;
    gentity_t *e = G_Spawn();
    printf("%d %d %d %d\n", MAX_GENTITIES, ENTITYNUM_MAX_NORMAL,
           (int)(e - native_entities), level.num_entities);
    return 0;
}
'''
    return COMMON + definitions + Q3 + bodies + setup + DRIVER, references


def schedules(family):
    capacity, reserved = {"q1": (600, 2), "qw": (768, 33),
                          "q2": (1024, 13), "q3": (1022, 64)}[family]
    commands = []
    descriptions = []

    def case(name, description, operations, start=0):
        descriptions.append({"name": name, "covers": description})
        commands.append(f"reset {name} {capacity} {reserved} {start}")
        commands.extend(operations)

    if family != "q3":
        case("strict_age", "EDICT age >0.5; same slot only after boundary", [
            "alloc 10 3", f"free 10 {reserved}", "alloc 10.4990234375 1",
            "alloc 10.5 1", "alloc 10.5009765625 1", f"check {reserved} 1",
            f"check {reserved} 2"])
        case("early_grace", "EDICT freed <2 ignores delay", [
            "alloc 1.5 1", f"free 1.9990234375 {reserved}",
            "alloc 1.9990234375 1", f"check {reserved} 1"])
        case("exact_grace", "EDICT freed ==2 retains delay", [
            "alloc 1.5 1", f"free 2 {reserved}", "alloc 2.1 1",
            "alloc 2.5 1", "alloc 2.5009765625 1"])
        case("float_freetime", "float freed_at; double Q1/QW now vs float Q2 now", [
            "alloc 2 1", f"free 2.00000001 {reserved}",
            "alloc 2.50000001 1", "alloc 2.5009765625 1"])
        case("rounded_grace", "free just below2 rounds to native float2", [
            "alloc 1 1", f"free 1.99999999 {reserved}", "alloc 2.1 1"])
        case("ascending_reuse", "ascending eligible slots; skip recent earlier slot", [
            "alloc 10 4", f"free 10 {reserved}", f"free 9 {reserved + 2}",
            "alloc 10.25 1", "alloc 10.75 1"])
        if family == "qw":
            case("full_overwrite", "QW full table overwrites last live, then delayed last free", [
                f"alloc 10 {capacity - reserved}", "alloc 10 1",
                f"check {capacity - 1} 1", f"check {capacity - 1} 2",
                f"free 10 {capacity - 1}", "alloc 10.25 1",
                f"check {capacity - 1} 2", f"check {capacity - 1} 3",
                f"free 10 {reserved}", "alloc 10.501 1"])
        else:
            case("full_reject", "full table rejects at0.5 and reuses only after0.5", [
                f"alloc 10 {capacity - reserved}", f"free 10 {reserved}",
                "alloc 10.5 1", "alloc 10.5009765625 1",
                f"check {reserved} 1", f"check {reserved} 2", "alloc 11.1 1"])
        if family == "q2":
            case("protected_body_queue", "world/client and native eight-row body queue free refusal", [
                "free 10 0", "free 10 4", "free 10 5", "free 10 12",
                "alloc 10 1", "free 10 13", "check 13 1", "check 12 1"])
    else:
        for name, freed in (("before_grace", 14344), ("inclusive_grace", 14345)):
            case(name, "Q3 freed <=level.startTime+2000 ignores delay", [
                "alloc 13000 1", f"free {freed} {reserved}",
                f"alloc {freed} 1", f"check {reserved} 1"], start=12345)
        case("after_grace", "Q3 age999 skips; age1000 reuses after inclusive grace", [
            "alloc 13000 1", f"free 14346 {reserved}", "alloc 14346 1",
            "alloc 15345 1", "alloc 15346 1", f"check {reserved} 1",
            f"check {reserved} 2"], start=12345)
        case("decimal_ms", "exact21001/22001ms avoids seconds subtraction rounding", [
            "alloc 21000 1", f"free 21001 {reserved}", "alloc 22000 1",
            "alloc 22001 1", f"free 33003 {reserved}", "alloc 34002 1",
            "alloc 34003 1"], start=12345)
        case("never_free", "G_FreeEntity preserves neverFree lifetime", [
            "alloc 21001 1", f"protect {reserved} 1", f"free 21001 {reserved}",
            f"check {reserved} 1", "alloc 22001 1", f"protect {reserved} 0",
            f"free 22001 {reserved}", f"check {reserved} 1", "alloc 23001 1"], start=12345)
        case("normal_full", "normal1022 ceiling rejects delayed slot; no1024 force pass", [
            f"alloc 21001 {capacity - reserved}", f"free 21001 {reserved}",
            "alloc 22000 1", "alloc 22001 1", f"check {reserved} 1",
            f"check {reserved} 2", "alloc 24001 1"], start=12345)
    return "\n".join(commands) + "\n", descriptions, capacity, reserved


def validate_rows(text, descriptions, capacity, reserved):
    names = {case["name"] for case in descriptions}
    steps = {name: 0 for name in names}
    final_slots = {name: [] for name in names}
    rows = []
    for line in text.splitlines():
        columns = line.split()
        if len(columns) != 9 or columns[0] not in names:
            raise ValueError("invalid reference row: " + line)
        name, step, op = columns[:3]
        slot, generation, accepted, displaced, old_generation, live = map(int, columns[3:])
        if int(step) != steps[name] or accepted not in (0, 1) or not 0 <= live <= capacity:
            raise ValueError("invalid row state: " + line)
        steps[name] += 1
        if op not in ("alloc", "free", "protect", "check", "final"):
            raise ValueError("invalid operation: " + line)
        if op == "alloc":
            if accepted and not reserved <= slot < capacity:
                raise ValueError("allocator used protected or out-of-range slot")
            if not accepted and (slot != -1 or generation != 0 or displaced != -1):
                raise ValueError("invalid allocation rejection")
        elif not 0 <= slot < capacity or generation < 1:
            raise ValueError("invalid handle observation")
        if displaced != -1 and (displaced != slot or old_generation + 1 != generation):
            raise ValueError("invalid overwrite generation")
        if op == "final":
            final_slots[name].append(slot)
        rows.append((name, int(step), op, slot, generation, accepted, displaced, old_generation, live))
    if any(slots != list(range(capacity)) for slots in final_slots.values()):
        raise ValueError("missing or unordered final state observations")
    if not any(row[2] == "check" and row[5] == 0 for row in rows):
        raise ValueError("stale generation observation absent")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qsrc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cc", default="cc")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    commands = []
    result = {"issue": "THE-611", "passed": False,
              "scope": "headless original-C lifetime schedules and post-load Rust allocations; no gameplay",
              "limits": [
                  "Generation rows observe lifetime changes; native C has no generation-handle ABI.",
                  "Q1/QW allocator prefix exclusion is compared; their ED_Free does not protect that prefix.",
                  "QW original overwrites last edict; shared engine confines displacement to the requesting module.",
                  "Q3 normal comparison uses ENTITYNUM_MAX_NORMAL=1022; synthetic1024 force-pass has no Rust parity claim.",
                  "All-live synthetic Q3 num_entities1024 can initialize one-past-end; this undefined case is never executed.",
                  "Rust allocation counter covers this process's Rust allocations, not gameplay or foreign-library heaps.",
              ], "families": []}
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, text=True,
                            capture_output=True, check=False)
    dirty = subprocess.run(["git", "status", "--porcelain"], cwd=root, text=True,
                           capture_output=True, check=False)
    result["commit"] = commit.stdout.strip() if commit.returncode == 0 else None
    result["dirty"] = dirty.stdout.splitlines() if dirty.returncode == 0 else None

    def run(command, log, env=None):
        started = time.monotonic()
        completed = subprocess.run(["timeout", "300", *map(str, command)], cwd=root,
                                   env=env, text=True, capture_output=True, check=False)
        elapsed = time.monotonic() - started
        (output / log).write_text(completed.stdout + completed.stderr)
        commands.append({"command": list(map(str, command)), "log": log,
                         "seconds": elapsed, "returncode": completed.returncode})
        if completed.returncode != 0:
            raise RuntimeError(f"command failed ({completed.returncode}); see {output / log}")
        return completed.stdout, elapsed

    try:
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = env.get("CARGO_TARGET_DIR", "target")
        _, build_seconds = run(["cargo", "build", "--release", "-p", "qa-world",
                                "--example", "entity_reference"], "rust-build.log", env)
        result["rust_build_seconds"] = build_seconds
        target = Path(env["CARGO_TARGET_DIR"])
        if not target.is_absolute():
            target = root / target
        binary = target / "release/examples/entity_reference"
        for family in ("q1", "qw", "q2", "q3"):
            source, references = native_source(args.qsrc, family)
            source_path = output / f"{family}-original.c"
            source_path.write_text(source)
            schedule, cases, capacity, reserved = schedules(family)
            schedule_path = output / f"{family}-schedule.txt"
            schedule_path.write_text(schedule)
            native = output / f"{family}-original"
            _, c_seconds = run([args.cc, "-std=gnu11", "-O3", "-ffp-contract=off",
                                 source_path, "-o", native], f"{family}-build.log")
            c_text, _ = run([native, schedule_path], f"{family}-original.rows")
            rust_text, _ = run([binary, family, schedule], f"{family}-rust.rows", env)
            rows = validate_rows(c_text, cases, capacity, reserved)
            validate_rows(rust_text, cases, capacity, reserved)
            if c_text != rust_text:
                actual = rust_text.splitlines()
                expected = c_text.splitlines()
                mismatch = next((index for index, pair in enumerate(zip(actual, expected))
                                 if pair[0] != pair[1]), min(len(actual), len(expected)))
                raise ValueError(f"{family} row{mismatch} differs: Rust={actual[mismatch:mismatch + 1]} C={expected[mismatch:mismatch + 1]}")
            result["families"].append({
                "family": family, "capacity": capacity, "reserved": reserved,
                "rows_compared": len(rows), "cases": cases, "references": references,
                "c_build_seconds": c_seconds,
                "allocations_compared": sum(row[2] == "alloc" for row in rows),
                "explicit_generation_checks": sum(row[2] == "check" for row in rows),
            })
            if family == "q3":
                synthetic, _ = run([native, "--synthetic"], "q3-synthetic.rows")
                values = list(map(int, synthetic.split()))
                if values != [1024, 1022, 64, 1024]:
                    raise ValueError("unexpected original Q3 synthetic force-pass observation")
                result["q3_synthetic_observation"] = {
                    "MAX_GENTITIES": values[0], "ENTITYNUM_MAX_NORMAL": values[1],
                    "selected_slot": values[2], "num_entities_after": values[3],
                    "freed_ms": 21001, "now_ms": 21002, "rust_parity_claim": False,
                }
        churn_text, churn_seconds = run([binary, "--churn"], "rust-churn.json", env)
        churn = json.loads(churn_text)
        if (churn["capacity"] != 8192 or churn["allocations_after_load"] != 0
                or churn["measured_cycles"] != 600 or churn["warmup_cycles"] != 60
                or churn["allocate_release_pairs"] != 600 * 8191
                or churn["remaining_live"] != 1):
            raise ValueError("post-load churn qualification differs")
        result["post_load_churn"] = churn
        result["churn_process_seconds_not_renderer_timing"] = churn_seconds
        result["passed"] = True
    finally:
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        (output / "verification.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
