#!/usr/bin/env python3
"""THE-860 Q2 native frame-prefix comparison, excluding player/entity bodies."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess
from check_hull_trace import function
from check_state_delta import kex_player_reference

ROOT = Path(__file__).resolve().parents[1]

STUBS = r'''
#include <stdio.h>
void *q2protoio_write_reserve_raw(uintptr_t arg,size_t size) {
 kex_io_t *io=(void*)arg;assert(io->size+size<=1400);
 void *ptr=io->bytes+io->size;io->size+=size;return ptr;
}
const void *q2protoio_read_raw(uintptr_t arg,size_t size,size_t *count) {
 kex_io_t *io=(void*)arg;assert(io->pos+size<=io->size);
 const void *ptr=io->bytes+io->pos;io->pos+=size;if(count)*count=size;return ptr;
}
/* Cold prefix-only bindings: no native player or entity body is tested here. */
static uint8_t decoded_extra;
static q2proto_error_t kex_server_write_playerstate(q2proto_servercontext_t *c,uintptr_t io,const q2proto_svc_playerstate_t *p) {
 (void)c;(void)p;q2protoio_write_u8(io,svc_playerinfo);return Q2P_ERR_SUCCESS;
}
static q2proto_error_t kex_client_read_playerstate(q2proto_clientcontext_t *c,uintptr_t io,q2proto_svc_playerstate_t *p) {
 (void)c;(void)io;(void)p;return Q2P_ERR_SUCCESS;
}
static q2proto_error_t q2repro_server_write_playerstate(q2proto_servercontext_t *c,uintptr_t io,const q2proto_svc_playerstate_t *p,uint8_t *extra) {
 (void)c;(void)io;*extra=p->clientnum;return Q2P_ERR_SUCCESS;
}
static q2proto_error_t q2repro_client_read_playerstate(q2proto_clientcontext_t *c,uintptr_t io,uint8_t extra,q2proto_svc_playerstate_t *p) {
 (void)c;(void)io;(void)p;decoded_extra=extra;return Q2P_ERR_SUCCESS;
}
static q2proto_error_t kex_client_read_delta_entities(q2proto_clientcontext_t *c,uintptr_t io,q2proto_svc_message_t *m) {
 (void)c;(void)io;(void)m;abort();
}
static q2proto_error_t q2repro_client_read_delta_entities(q2proto_clientcontext_t *c,uintptr_t io,q2proto_svc_message_t *m) {
 (void)c;(void)io;(void)m;abort();
}
'''

DRIVER = r'''
int main(void) {
 int mode;
 while((mode=fgetc(stdin))!=EOF) {
  q2proto_svc_frame_t f={0},decoded={0};uint8_t flags,extra,count,areas[255],wire[1400];
  if(mode>2||fread(&f.serverframe,4,1,stdin)!=1||fread(&f.deltaframe,4,1,stdin)!=1
   ||fread(&flags,1,1,stdin)!=1||fread(&extra,1,1,stdin)!=1||fread(&count,1,1,stdin)!=1
   ||fread(areas,1,count,stdin)!=count)return 2;
  f.q2pro_frame_flags=f.suppress_count=flags;f.playerstate.clientnum=extra;
  f.areabits_len=count;f.areabits=areas;
  q2proto_servercontext_t server={0};q2proto_clientcontext_t client={0};
  kex_io_t io={.bytes=wire};decoded_extra=0;
  if(mode==2)assert(q2repro_server_write_frame(&server,(uintptr_t)&io,&f)==Q2P_ERR_SUCCESS);
  else assert(kex_server_write_frame(&server,(uintptr_t)&io,&f)==Q2P_ERR_SUCCESS);
  assert(q2protoio_read_u8((uintptr_t)&io)==svc_frame);
  if(mode==2)assert(q2repro_client_read_frame(&client,(uintptr_t)&io,&decoded)==Q2P_ERR_SUCCESS);
  else assert(kex_client_read_frame(&client,(uintptr_t)&io,&decoded)==Q2P_ERR_SUCCESS);
  assert(io.pos==io.size);
  fwrite(&io.size,4,1,stdout);fwrite(wire,1,io.size,stdout);
  fwrite(&decoded.serverframe,4,1,stdout);fwrite(&decoded.deltaframe,4,1,stdout);
  uint8_t meta[3]={mode==2?decoded.q2pro_frame_flags:decoded.suppress_count,decoded_extra,decoded.areabits_len};
  fwrite(meta,1,3,stdout);fwrite(decoded.areabits,1,decoded.areabits_len,stdout);
 }
 return 0;
}
'''


def compile_reference(qsrc, evidence):
    base = qsrc / 'q2repro/q2proto'
    # The existing cold IO binding includes the actual native headers.
    source = kex_player_reference(qsrc).split('#define GUNBIT_OFFSET_X', 1)[0] + STUBS
    for path, names in [
        ('q2proto_proto_kex.c', ['kex_server_write_frame', 'kex_client_read_frame']),
        ('q2proto_proto_q2repro.c', ['q2repro_server_write_frame', 'q2repro_client_read_frame']),
    ]:
        original = (base / 'src' / path).read_text()
        for name in names:
            source += '\n' + function(original, name)
    code = evidence / 'original-frame-headers.c'
    code.write_text(source + DRIVER)
    binary = evidence / 'original-frame-headers'
    command = ['cc', '-O2', '-std=c11', '-DQ2PROTO_CONFIG_PROVIDED=1',
               '-DQ2PROTO_PLAYER_STATE_FEATURES=Q2PROTO_FEATURES_RERELEASE',
               '-DQ2PROTO_ENTITY_STATE_FEATURES=Q2PROTO_FEATURES_RERELEASE',
               '-I', str(base / 'inc'), '-I', str(base / 'src'), str(code), '-o', str(binary)]
    (evidence / 'compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
    subprocess.run(command, check=True)
    return binary


def fixture():
    rng = random.Random(8601038)
    output = bytearray()
    for mode in range(3):
        for case in range(2048):
            sequence = ([0, 1, 30, 0x07ffffff, 0x08000000, 0x7fffffff, 0xffffffff][case % 7]
                        if case < 256 else rng.getrandbits(32))
            distance = case % 32
            delta = 0xffffffff if distance == 31 else (sequence - distance) & 0xffffffff
            flags, extra = rng.getrandbits(8), rng.getrandbits(8)
            count = [0, 1, 32, 33, 255][case % 5]
            areas = bytes(rng.getrandbits(8) for _ in range(count))
            output += struct.pack('<BII3B', mode, sequence, delta, flags, extra, count) + areas
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--qsrc', type=Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--probe', type=Path, default=ROOT / 'target/release/examples/frame_headers')
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    native = compile_reference(args.qsrc, args.evidence)
    data = fixture()
    (args.evidence / 'fixture.bin').write_bytes(data)
    expected = subprocess.run([native], input=data, capture_output=True, check=True).stdout
    actual = subprocess.run([args.probe], input=data, capture_output=True, check=True)
    (args.evidence / 'original.bin').write_bytes(expected)
    (args.evidence / 'rust.bin').write_bytes(actual.stdout)
    (args.evidence / 'probe.json').write_bytes(actual.stderr)
    equal = expected == actual.stdout
    result = {'result': 'PASS' if equal else 'FAIL', 'cases': 6144,
              'per_format': 2048, 'formats': ['KEX2023', 'KEX2022 demo', 'q2repro1038'],
              'seed': 8601038, 'byte_exact': equal,
              'scope': 'Original C frame-prefix write/read with empty player stubs; no entity/frame/channel acceptance',
              'timing_run': False}
    (args.evidence / 'comparison.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
    if not equal:
        raise SystemExit('frame prefix mismatch')


if __name__ == '__main__':
    main()
