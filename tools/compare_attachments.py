#!/usr/bin/env python3
"""Compare shared Rust attachment transport with unchanged C-port functions.

The cold stubs supply bodies and count links. Native module field access and
touch callbacks are excluded. This never starts a game or qualifies an install.
"""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess

from check_thinks import exact_slice, extract_function

ROWS = 32
INPUT_WORDS = 18
OUTPUT_WORDS = ROWS * 12 + 2

PREFIX = r'''
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <math.h>
typedef struct { float x,y,z; } qa_vec3;
typedef struct { uint32_t slot,generation; } qa_actor_id;
typedef enum { QA_BODY_FOLLOW_TRANSLATION, QA_BODY_FOLLOW_CENTER, QA_BODY_FOLLOW_BOUNDS_MIN } qa_body_follow;
typedef struct { qa_actor_id anchor; qa_body_follow follow; qa_vec3 offset; } qa_body_attachment;
typedef struct { qa_vec3 mins,maxs; } qa_bounds;
typedef struct { qa_vec3 origin,velocity; qa_bounds bounds; } qa_body_state;
typedef struct { qa_actor_id actor; bool present,attached; qa_body_attachment attachment; uint64_t attachment_order; qa_body_state state; } qa_world_body;
typedef struct { uint32_t capacity; uint64_t attachment_order; qa_world_body bodies[32]; uint32_t links; } qa_world;
typedef struct { int code; } qa_error;
enum { QA_ERROR_ARGUMENT, QA_ERROR_NOT_FOUND, QA_ERROR_MEMORY };
static bool fail(qa_error *error,int code,const char *message) { (void)message; if(error)error->code=code; return false; }
static bool qa_actor_id_equal(qa_actor_id a,qa_actor_id b) { return a.slot==b.slot && a.generation==b.generation; }
static bool qa_vec_finite(qa_vec3 a) { return isfinite(a.x)&&isfinite(a.y)&&isfinite(a.z); }
static qa_vec3 qa_vec_add(qa_vec3 a,qa_vec3 b) { return (qa_vec3){a.x+b.x,a.y+b.y,a.z+b.z}; }
static qa_vec3 qa_vec_scale(qa_vec3 a,float scale) { return (qa_vec3){a.x*scale,a.y*scale,a.z*scale}; }
static qa_world_body *qa_world_raw_body(qa_world *world,uint32_t slot) { return slot<world->capacity?&world->bodies[slot]:NULL; }
static qa_world_body *qa_world_find_body(qa_world *world,qa_actor_id id) {
    qa_world_body *body=qa_world_raw_body(world,id.slot);
    return body && body->present && qa_actor_id_equal(body->actor,id)?body:NULL;
}
static bool qa_world_body_read(qa_world *world,qa_actor_id id,qa_body_state *state,qa_error *error) {
    (void)error; qa_world_body *body=qa_world_find_body(world,id); if(!body)return false; *state=body->state; return true;
}
static bool qa_world_body_write(qa_world *world,qa_actor_id id,const qa_body_state *state,qa_error *error) {
    (void)error; qa_world_body *body=qa_world_find_body(world,id); if(!body)return false; body->state=*state; return true;
}
static bool qa_world_link(qa_world *world,qa_actor_id id,void *query,qa_error *error) {
    (void)id;(void)query;(void)error; ++world->links; return true;
}
'''
SUFFIX = r'''
int main(void) {
    uint32_t words[32][18];
    for(;;) {
        size_t count=fread(words,1,sizeof(words),stdin);
        if(count==0)return ferror(stdin)?2:0;
        if(count!=sizeof(words))return 3;
        qa_world world={0}; world.capacity=32; qa_error error={0};
        for(uint32_t slot=0;slot<32;++slot) {
            qa_world_body *body=&world.bodies[slot]; body->actor=(qa_actor_id){slot,1}; body->present=true;
            memcpy(&body->state,words[slot],12*sizeof(uint32_t));
        }
        for(uint32_t order=1;order<=32;++order) for(uint32_t slot=0;slot<32;++slot) if(words[slot][17]==order) {
            qa_body_attachment follow={0}; follow.anchor=(qa_actor_id){words[slot][15],1};
            follow.follow=(qa_body_follow)words[slot][16]; memcpy(&follow.offset,&words[slot][12],3*sizeof(uint32_t));
            if(!qa_world_attach(&world,world.bodies[slot].actor,&follow,&error))return 4;
        }
        if(!qa_world_transport_attachments(&world,&error))return 5;
        uint32_t first=world.links;
        if(!qa_world_transport_attachments(&world,&error))return 6;
        for(uint32_t slot=0;slot<32;++slot) if(fwrite(&world.bodies[slot].state,1,12*sizeof(uint32_t),stdout)!=12*sizeof(uint32_t))return 7;
        uint32_t counts[2]={first,world.links-first};
        if(fwrite(counts,1,sizeof(counts),stdout)!=sizeof(counts))return 8;
    }
}
'''


def fixtures(cases):
    rng = random.Random(0x650)
    result = bytearray()
    for case in range(cases):
        order = list(range(1, ROWS))
        rng.shuffle(order)
        for slot in range(ROWS):
            floats = [rng.randint(-256, 256) * 0.125 for _ in range(6)]
            floats += [-rng.randint(0, 128) * 0.25 for _ in range(3)]
            floats += [rng.randint(0, 128) * 0.25 for _ in range(3)]
            floats += [rng.randint(-64, 64) * 0.25 for _ in range(3)]
            if case % 16 == 0:
                # Exact signed-zero behavior and a stationary second pass.
                floats = [-0.0] * 3 + floats[3:6] + [-0.0] * 9
            anchor = rng.randrange(slot) if slot else 0xFFFFFFFF
            mode = (case + slot) % 3
            insertion = order.index(slot) + 1 if slot else 0
            if case % 16 == 1 and slot:
                anchor = slot - 1  # Full-depth chains, random insertion order.
            result += struct.pack('<15f3I', *floats, anchor, mode, insertion)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--c-port', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--cases', type=int, default=512)
    args = parser.parse_args()
    if not 1 <= args.cases <= 10000:
        parser.error('cases outside 1..10000')
    args.evidence.mkdir(parents=True, exist_ok=True)
    source_path = args.c_port / 'src/world/body.c'
    source = source_path.read_text()
    attach, attach_span = extract_function(source, 'qa_world_attach')
    # This final contiguous source block includes same_float and the unchanged
    # transport function; abort if the reference's tail ceases to match.
    start = source.index('typedef struct attachment_transport')
    transport, transport_span = exact_slice(source, start, len(source), 'attachment_transport + same_float + qa_world_transport_attachments')
    extracted = args.evidence / 'attachment_reference.c'
    extracted.write_text(PREFIX + attach + '\n' + transport + '\n' + SUFFIX)
    native = args.evidence / 'attachment_reference'
    subprocess.run(['cc', '-std=c11', '-O2', '-ffp-contract=off', '-fno-fast-math',
                    str(extracted), '-lm', '-o', str(native)], check=True)
    data = fixtures(args.cases)
    fixture = args.evidence / 'fixtures.bin'
    fixture.write_bytes(data)
    c_output = args.evidence / 'c-output.bin'
    with c_output.open('wb') as output:
        subprocess.run([str(native)], input=data, stdout=output, check=True)
    rust_output = args.evidence / 'rust-output.bin'
    rust = subprocess.run([str(args.binary.resolve()), '--compare', str(fixture), str(rust_output)],
                          capture_output=True, text=True, check=True)
    (args.evidence / 'rust-report.json').write_text(rust.stdout)
    expected, actual = c_output.read_bytes(), rust_output.read_bytes()
    if len(expected) != args.cases * OUTPUT_WORDS * 4 or len(actual) != len(expected):
        raise RuntimeError('comparison output dimensions differ')
    unequal = [index for index, (a, b) in enumerate(zip(struct.iter_unpack('<I', expected), struct.iter_unpack('<I', actual))) if a != b]
    report = {'scope': 'unchanged proven C-port attachment functions; body/link stubs exclude native module reads/touches; no gameplay',
              'reference': str(source_path), 'exact_source_slices': [attach_span, transport_span],
              'seed': '0x650', 'cases': args.cases, 'body_rows': args.cases * ROWS,
              'state_words_per_case': ROWS * 12, 'transport_passes_per_case': 2,
              'mismatched_words': len(unequal), 'rust_transport': json.loads(rust.stdout)}
    (args.evidence / 'comparison.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    if unequal:
        raise RuntimeError(f'first unequal word: case {unequal[0] // OUTPUT_WORDS}, word {unequal[0] % OUTPUT_WORDS}')


if __name__ == '__main__':
    main()
