#!/usr/bin/env python3
"""THE-860 joined KEX receive comparison against original native functions."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess

from check_hull_trace import function
from check_state_delta import kex_player_reference, repro_entity_reference

ROOT = Path(__file__).resolve().parents[1]

BINDINGS = r'''
#include <stdio.h>
void *q2protoio_write_reserve_raw(uintptr_t arg,size_t size) {
 kex_io_t *io=(void*)arg;assert(io->size+size<=1400);
 void *ptr=io->bytes+io->size;io->size+=size;return ptr;
}
const void *q2protoio_read_raw(uintptr_t arg,size_t size,size_t *count) {
 kex_io_t *io=(void*)arg;assert(io->pos+size<=io->size);
 const void *ptr=io->bytes+io->pos;io->pos+=size;if(count)*count=size;return ptr;
}
static q2proto_error_t kex_client_read(q2proto_clientcontext_t *c,uintptr_t io,q2proto_svc_message_t *m) {
 /* The cold fixture stops exactly at the frame terminator. */
 (void)c;(void)io;(void)m;abort();
}
static q2proto_error_t kex_client_next_frame_entity_delta(q2proto_clientcontext_t*,uintptr_t,q2proto_svc_frame_entity_delta_t*);
'''

MERGE_BINDINGS = r'''
#define MAX_EDICTS 8192
#define PARSE_ENTITIES_MASK 8191
#define ERR_DROP 1
#undef SHOWNET
#define SHOWNET(...) ((void)0)
typedef struct {int number;uint32_t words[25];} entity_state_t;
typedef struct {int firstEntity,numEntities;uint32_t player[106];} server_frame_t;
static struct {int numEntityStates;entity_state_t entityStates[8192],baselines[8192];
 struct {int max_edicts;} csr;} cl;
static struct {q2proto_clientcontext_t q2proto_ctx;} cls;
static kex_io_t *active_io;
#define Q2PROTO_IOARG_CLIENT_READ ((uintptr_t)active_io)
static void Com_Error(int code,const char *fmt,...) {(void)code;(void)fmt;abort();}
static void Com_DPrintf(const char *fmt,...) {(void)fmt;}
q2proto_error_t q2proto_client_read(q2proto_clientcontext_t *c,uintptr_t io,q2proto_svc_message_t *m) {
 return c->client_read(c,io,m);
}
/* Cold packed-word application, not an engine/module ABI adapter. The merge
   below is the unmodified native CL_ParsePacketEntities. */
static void CL_ParseDeltaEntity(server_frame_t *frame,int number,const entity_state_t *old,
 const q2proto_entity_state_delta_t *delta) {
 assert(frame->numEntities<64);
 entity_state_t *to=&cl.entityStates[cl.numEntityStates++&PARSE_ENTITIES_MASK];
 frame->numEntities++;to->number=number;
 q2proto_entity_state_delta_t empty={0};
 enhanced_entity_get(delta? (q2proto_entity_state_delta_t*)delta:&empty,
                     (uint32_t*)old->words,to->words,true);
}
'''

DRIVER = r'''
static void input(void *p,size_t size,size_t count) {assert(fread(p,size,count,stdin)==count);}
static entity_state_t input_entity(void) {
 uint16_t number;entity_state_t e;input(&number,2,1);e.number=number;
 assert(number>0&&number<MAX_EDICTS);input(e.words,4,25);return e;
}
static const entity_state_t *entity_base(const server_frame_t *old,uint16_t number) {
 if(old)for(int i=0;i<old->numEntities;i++) {
  entity_state_t *e=&cl.entityStates[(old->firstEntity+i)&PARSE_ENTITIES_MASK];
  if(e->number==number)return e;
 }
 return &cl.baselines[number];
}
int main(int argc,char **argv) {
 assert(argc==3);FILE *fixture=fopen(argv[1],"wb"),*expected=fopen(argv[2],"wb");
 assert(fixture&&expected);uint32_t cases;input(&cases,4,1);fwrite(&cases,4,1,fixture);
 for(uint32_t k=0;k<cases;k++) {
  uint8_t demo,frames;uint16_t bases;input(&demo,1,1);input(&bases,2,1);
  memset(&cl,0,sizeof(cl));memset(&cls,0,sizeof(cls));cl.csr.max_edicts=MAX_EDICTS;
  q2proto_servercontext_t server={0};server.protocol=demo?Q2P_PROTOCOL_KEX_DEMOS:Q2P_PROTOCOL_KEX;
  cls.q2proto_ctx.server_protocol=server.protocol;
  fwrite(&demo,1,1,fixture);fwrite(&bases,2,1,fixture);
  for(int i=0;i<bases;i++) {
   entity_state_t e=input_entity();cl.baselines[e.number]=e;uint16_t n=e.number;
   fwrite(&n,2,1,fixture);fwrite(e.words,4,25,fixture);
   q2proto_set_entity_bit(server.kex_demo_baseline_nonzero_solid,n,e.words[19]!=0);
   q2proto_set_entity_bit(server.kex_demo_edict_nonzero_solid,n,e.words[19]!=0);
   q2proto_set_entity_bit(cls.q2proto_ctx.kex_demo_baseline_nonzero_solid,n,e.words[19]!=0);
   q2proto_set_entity_bit(cls.q2proto_ctx.kex_demo_edict_nonzero_solid,n,e.words[19]!=0);
  }
  input(&frames,1,1);fwrite(&frames,1,1,fixture);server_frame_t saved[32]={0};
  for(int j=0;j<frames;j++) {
   q2proto_svc_frame_t f={0};uint8_t flags,count,areas[255];uint16_t ops;
   uint32_t target[106],zero[106]={0};
   input(&f.serverframe,4,1);input(&f.deltaframe,4,1);input(&flags,1,1);input(&count,1,1);
   input(areas,1,count);input(target,4,106);input(&ops,2,1);
   server_frame_t *old=f.deltaframe>0?&saved[f.deltaframe&31]:NULL;
   f.suppress_count=flags;f.areabits_len=count;f.areabits=areas;
   f.playerstate=kex_player_delta(old?old->player:zero,target);
   uint8_t bytes[1400];kex_io_t io={.bytes=bytes};
   assert(kex_server_write_frame(&server,(uintptr_t)&io,&f)==Q2P_ERR_SUCCESS);
   for(int i=0;i<ops;i++) {
    uint16_t number;uint8_t remove,write_old;input(&number,2,1);input(&remove,1,1);input(&write_old,1,1);
    uint32_t words[25];input(words,4,25);assert(number>0&&number<MAX_EDICTS);
    q2proto_svc_frame_entity_delta_t d={.newnum=number,.remove=remove};
    if(!remove) {
     q2proto_packed_entity_state_t a={0},b={0};
     repro_entity_put(&a,(uint32_t*)entity_base(old,number)->words);repro_entity_put(&b,words);
     kex_server_make_entity_state_delta(&server,&a,&b,write_old,&d.entity_delta);
    }
    assert(kex_server_write_frame_entity_delta(&server,(uintptr_t)&io,&d)==Q2P_ERR_SUCCESS);
   }
   q2proto_svc_frame_entity_delta_t end={0};
   assert(kex_server_write_frame_entity_delta(&server,(uintptr_t)&io,&end)==Q2P_ERR_SUCCESS);
   uint16_t size=io.size;fwrite(&size,2,1,fixture);fwrite(bytes,1,size,fixture);
   q2proto_svc_frame_t decoded={0};kex_put(&decoded.playerstate,old?old->player:zero,true);
   assert(q2protoio_read_u8((uintptr_t)&io)==svc_frame);
   assert(kex_client_read_frame(&cls.q2proto_ctx,(uintptr_t)&io,&decoded)==Q2P_ERR_SUCCESS);
   server_frame_t result={0};active_io=&io;CL_ParsePacketEntities(old,&result);
   kex_get(&decoded.playerstate,result.player);saved[f.serverframe&31]=result;
   assert(io.pos==io.size);uint16_t consumed=io.pos,entities=result.numEntities;
   fwrite(&consumed,2,1,expected);fwrite(&decoded.serverframe,4,1,expected);
   fwrite(&decoded.suppress_count,1,1,expected);fwrite(&decoded.areabits_len,1,1,expected);
   fwrite(decoded.areabits,1,decoded.areabits_len,expected);fwrite(result.player,4,106,expected);
   fwrite(&entities,2,1,expected);
   for(int i=0;i<result.numEntities;i++) {
    entity_state_t *e=&cl.entityStates[(result.firstEntity+i)&PARSE_ENTITIES_MASK];uint16_t number=e->number;
    fwrite(&number,2,1,expected);fwrite(e->words,4,25,expected);
   }
  }
 }
 assert(fclose(fixture)==0);assert(fclose(expected)==0);return 0;
}
'''


def compile_reference(qsrc, evidence):
    base = qsrc / 'q2repro/q2proto'
    original = (base / 'src/q2proto_proto_kex.c').read_text()
    source = kex_player_reference(qsrc) + repro_entity_reference(qsrc) + BINDINGS
    debug_start = original.index('static MAYBE_UNUSED void kex_debug_shownet_entity_delta_bits')
    debug_end = original.index('static q2proto_error_t kex_client_read(', debug_start)
    source += original[debug_start:debug_end]
    names = ['kex_client_next_frame_entity_delta',
             'kex_client_read_delta_entities', 'kex_server_write_frame',
             'kex_client_read_frame', 'kex_server_write_frame_entity_delta']
    for name in names:
        source += '\n' + function(original, name)
    source += MERGE_BINDINGS
    source += function((qsrc / 'q2repro/src/client/parse.c').read_text(), 'CL_ParsePacketEntities')
    code = evidence / 'original-kex-frames.c'
    code.write_text(source + DRIVER)
    binary = evidence / 'original-kex-frames'
    command = ['cc', '-O2', '-std=c11', '-ffp-contract=off',
               '-DQ2PROTO_CONFIG_PROVIDED=1',
               '-DQ2PROTO_PLAYER_STATE_FEATURES=Q2PROTO_FEATURES_RERELEASE',
               '-DQ2PROTO_ENTITY_STATE_FEATURES=Q2PROTO_FEATURES_RERELEASE',
               '-I', str(base / 'inc'), '-I', str(base / 'src'), str(code),
               str(base / 'src/q2proto_coords.c'), '-lm', '-o', str(binary)]
    (evidence / 'compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
    subprocess.run(command, check=True)
    return binary


def float_word(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def fixture():
    rng = random.Random(8602023)
    output = bytearray(struct.pack('<I', 512))
    for mode in range(2):
        for case in range(256):
            def entity():
                words = [0] * 25
                for i in range(25):
                    if 8 <= i <= 16:
                        words[i] = float_word(rng.uniform(-512, 512))
                    elif i < 5:
                        words[i] = rng.randrange(65536)
                    elif i == 17:
                        words[i] = rng.randrange(16384)
                    elif i == 18 or i >= 21:
                        words[i] = rng.randrange(256)
                    else:
                        words[i] = rng.getrandbits(32)
                words[19] = case % 2
                return words

            def player():
                words = [0] * 106
                for i in range(106):
                    if (1 <= i <= 6 or 10 <= i <= 12 or 16 <= i <= 18 or 24 <= i <= 29):
                        words[i] = float_word(rng.uniform(-512, 512))
                    elif i in (7, 8, 22):
                        words[i] = rng.randrange(65536)
                    elif i == 23:
                        words[i] = rng.randrange(512)
                    elif i in (0, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40):
                        words[i] = rng.randrange(256)
                    elif i == 41:
                        words[i] = rng.randrange(-128, 128) & 0xffffffff
                    else:
                        words[i] = rng.randrange(-32768, 32768) & 0xffffffff
                return words

            numbers = [1, 255, 256, 8191]
            bases = [entity() for _ in numbers]
            output += struct.pack('<BH', mode, len(numbers))
            for number, words in zip(numbers, bases):
                output += struct.pack('<H25I', number, *words)
            output += bytes([6])
            first = [entity() for _ in range(3)]
            second = first[0].copy()
            second[19] = 1 - first[0][19]
            second[8] = float_word(7.03125)
            older = first[0].copy()
            older[8] = float_word(-1.01)
            removed = [0] * 25
            frames = [
                (-1, [(1, 0, first[0]), (255, 0, first[1]), (8191, 0, first[2])]),
                (1, [(1, 0, second), (255, 1, removed), (256, 0, entity())]),
                (1, [(1, 0, older)]),
                (3, [(1, 1, removed)]),
                (4, [(1, 0, entity())]),
                (5, [(2, 1, removed)]),
            ]
            ps = player()
            for sequence, (delta, ops) in enumerate(frames, 1):
                areas = bytes(rng.getrandbits(8) for _ in range([0, 1, 32, 33, 255][(case + sequence) % 5]))
                next_ps = player() if sequence % 2 else ps.copy()
                next_ps[42 + (case % 64)] = (-32768 if sequence % 2 else 32767) & 0xffffffff
                output += struct.pack('<IiBB', sequence, delta, rng.randrange(256), len(areas)) + areas
                output += struct.pack('<106IH', *next_ps, len(ops))
                for number, remove, words in ops:
                    output += struct.pack('<HBB25I', number, remove, int((case + sequence + number) % 3 == 0), *words)
                ps = next_ps
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--qsrc', type=Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--probe', type=Path, default=ROOT / 'target/release/examples/rerelease_snapshots')
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    native = compile_reference(args.qsrc, args.evidence)
    data = fixture()
    (args.evidence / 'source-fixture.bin').write_bytes(data)
    projected = args.evidence / 'fixture.bin'
    expected = args.evidence / 'original.bin'
    subprocess.run([native, projected, expected], input=data, check=True)
    actual = subprocess.run([args.probe, '--compare', projected], capture_output=True, check=True)
    (args.evidence / 'rust.bin').write_bytes(actual.stdout)
    (args.evidence / 'probe.json').write_bytes(actual.stderr)
    equal = actual.stdout == expected.read_bytes()
    result = {'result': 'PASS' if equal else 'FAIL', 'sequences': 512, 'frames': 3072,
              'formats': [2023, 2022], 'decoded_words_exact': equal,
              'scope': 'complete native KEX frame receive and original ordered packet-entity merge; cold projected-word application, no module ABI, channel, OS or app',
              'source_functions': ['q2proto KEX frame/player/entity writers/readers', 'CL_ParsePacketEntities'],
              'cold_bindings': ['projected-word player delta metadata', 'entity delta application', 'private IO'],
              'timing_run': False}
    (args.evidence / 'comparison.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
    if not equal:
        raise SystemExit('native KEX frame comparison differs')


if __name__ == '__main__':
    main()
