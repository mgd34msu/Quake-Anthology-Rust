#!/usr/bin/env python3
"""THE-860 native entity/player deltas; unchanged original C is developer-only."""
import argparse
import json
from pathlib import Path
import random
import re
import struct
import subprocess
from check_message import PREAMBLE, function

ROOT = Path(__file__).resolve().parents[1]


def q2_layout():
    rust = (ROOT / 'crates/network/src/states.rs').read_text()
    block = re.search(r'pub const Q2_PLAYER_LAYOUT.*?=\s*\[(.*?)\n\];', rust, re.S).group(1)
    return [(name, int(width)) for name, width in re.findall(r'\("([^"]+)",\s*(-?\d+)\)', block)]


def q2_reference(qsrc):
    header = (qsrc / 'quake-2/game/q_shared.h').read_text()
    common = (qsrc / 'quake-2/qcommon/common.c').read_text()
    constants = (qsrc / 'quake-2/qcommon/qcommon.h').read_text()
    source = '\n#undef MAX_STATS\n#define MAX_STATS 32\n'
    for name, kind in [('pmtype_t', 'enum'), ('pmove_state_t', 'struct'), ('player_state_t', 'struct')]:
        end = header.index('} ' + name + ';') + len('} ' + name + ';')
        start = header.rfind('typedef ' + kind, 0, end)
        source += header[start:end] + '\n'
    source += '\n'.join(re.findall(r'^#define\s+PS_[A-Z_]+\s+.*$', constants, re.M)) + '\n'
    source += r'''
#define svc_playerinfo 17
#define PM_FREEZE 4
#define ANGLE2SHORT(x) ((int)((x)*65536/360)&65535)
#define SHORT2ANGLE(x) ((x)*(360.0/65536))
typedef msg_t sizebuf_t;
typedef struct {player_state_t ps;} client_frame_t;
typedef struct {player_state_t playerstate;} frame_t;
static msg_t net_message;
static struct {int attractloop;} cl;
static void q2_write(msg_t *m,int value,int width) {
 for(int i=0;i<width/8;i++)m->data[m->cursize++]=(uint32_t)value>>(8*i);
 m->bit=m->cursize*8;
}
static int q2_read(msg_t *m,int width,int sign) {
 uint32_t value=0;for(int i=0;i<width/8;i++)value|=(uint32_t)m->data[m->readcount++]<<(8*i);
 m->bit=m->readcount*8;return sign?(int32_t)(value<<(32-width))>>(32-width):(int)value;
}
#define MSG_WriteByte(m,v) q2_write(m,v,8)
#define MSG_WriteChar(m,v) q2_write(m,v,8)
#define MSG_WriteShort(m,v) q2_write(m,v,16)
#define MSG_WriteLong(m,v) q2_write(m,v,32)
#define MSG_ReadByte(m) q2_read(m,8,0)
#define MSG_ReadChar(m) q2_read(m,8,1)
#define MSG_ReadShort(m) q2_read(m,16,1)
#define MSG_ReadLong(m) q2_read(m,32,0)
#define MSG_WriteAngle16 Q2_WriteAngle16
#define MSG_ReadAngle16 Q2_ReadAngle16
'''
    source += function(common, 'MSG_WriteAngle16') + function(common, 'MSG_ReadAngle16')
    source += function((qsrc / 'quake-2/server/sv_ents.c').read_text(), 'SV_WritePlayerstateToClient')
    source += function((qsrc / 'quake-2/client/cl_ents.c').read_text(), 'CL_ParsePlayerstate')
    source += 'static void q2_put(player_state_t *p,uint32_t *words) {\n'
    for i, (name, _) in enumerate(q2_layout()):
        source += f' memcpy(&p->{name}, &words[{i}],4);\n' if 13 <= i <= 21 or 24 <= i <= 34 else f' p->{name}=words[{i}];\n'
    source += ' for(int i=0;i<32;i++)p->stats[i]=words[36+i];\n}\n'
    source += 'static void q2_get(player_state_t *p,uint32_t *words) {\n'
    for i, (name, _) in enumerate(q2_layout()):
        source += f' memcpy(&words[{i}], &p->{name},4);\n' if 13 <= i <= 21 or 24 <= i <= 34 else f' words[{i}]=p->{name};\n'
    source += ' for(int i=0;i<32;i++)words[36+i]=(int)p->stats[i];\n}\n'
    source += r'''
static void q2_decode(msg_t *m,player_state_t *from,player_state_t *to) {
 net_message=*m;frame_t a={.playerstate=*from},b={0};
 if(MSG_ReadByte(&net_message)!=17)abort();CL_ParsePlayerstate(&a,&b);*to=b.playerstate;
}
#undef MSG_WriteByte
#undef MSG_WriteChar
#undef MSG_WriteShort
#undef MSG_WriteLong
#undef MSG_ReadByte
#undef MSG_ReadChar
#undef MSG_ReadShort
#undef MSG_ReadLong
#undef MSG_WriteAngle16
#undef MSG_ReadAngle16
'''
    return source


def layouts(msg):
    result = []
    for name, macro in [('entityStateFields', 'NETF'), ('playerStateFields', 'PSF')]:
        block = re.search(r'netField_t\s+' + name + r'\[\]\s*=\s*\{.*?\n\};', msg, re.S).group()
        rows = re.findall(r'\{\s*' + macro + r'\((.*?)\),\s*(-?\d+|GENTITYNUM_BITS)\s*\}', block)
        result.append([(field, 10 if bits == 'GENTITYNUM_BITS' else int(bits)) for field, bits in rows])
    return result


def compile_reference(qsrc, evidence):
    msg = (qsrc / 'quake-iii-arena/code/qcommon/msg.c').read_text()
    shared = (qsrc / 'quake-iii-arena/code/game/q_shared.h').read_text()
    huff = re.sub(r'^#include.*$', '', (qsrc / 'quake-iii-arena/code/qcommon/huffman.c').read_text(), flags=re.M)
    source = PREAMBLE + '\n#include <stddef.h>\n#include <assert.h>\n#define ERR_FATAL 0\n'
    source += huff + re.search(r'int msg_hData\[256\] = \{.*?\n\};', msg, re.S).group()
    source += function(msg, 'MSG_WriteBits') + function(msg, 'MSG_ReadBits')
    for name in ['MSG_WriteByte', 'MSG_WriteShort', 'MSG_WriteLong', 'MSG_ReadByte', 'MSG_ReadShort', 'MSG_ReadLong']:
        source += function(msg, name)
    source += r'''
typedef float vec3_t[3];
#define MAX_STATS 16
#define MAX_PERSISTANT 16
#define MAX_POWERUPS 16
#define MAX_WEAPONS 16
#define MAX_PS_EVENTS 2
#define MAX_GENTITIES 1024
#define GENTITYNUM_BITS 10
#define FLOAT_INT_BITS 13
#define FLOAT_INT_BIAS 4096
#define LOG(x)
static struct {int integer;} shownet;
static void Com_Printf(char *fmt,...) {(void)fmt;}
#define cl_shownet (&shownet)
'''
    source += re.search(r'typedef enum \{\s*TR_STATIONARY,.*?\} trType_t;', shared, re.S).group()
    source += re.search(r'typedef struct \{\s*trType_t\s+trType;.*?\} trajectory_t;', shared, re.S).group()
    for tag, name in [('entityState_s', 'entityState_t'), ('playerState_s', 'playerState_t')]:
        source += re.search(r'typedef struct ' + tag + r' \{.*?\} ' + name + ';', shared, re.S).group()
    source += r'''
typedef struct {char *name;int offset,bits;} netField_t;
/* Original field lists; offsetof only replaces the null-pointer offset macro. */
#define NETF(x) #x,(int)offsetof(entityState_t,x)
#define PSF(x) #x,(int)offsetof(playerState_t,x)
'''
    for name in ['entityStateFields', 'playerStateFields']:
        source += re.search(r'netField_t\s+' + name + r'\[\]\s*=\s*\{.*?\n\};', msg, re.S).group()
    for name in ['MSG_WriteDeltaEntity', 'MSG_ReadDeltaEntity', 'MSG_WriteDeltaPlayerstate', 'MSG_ReadDeltaPlayerstate']:
        # cl_shownet is already bound to the private fixture's cvar; no print path runs.
        source += function(msg, name)
    source += q2_reference(qsrc)
    source += r'''
static void put(void *record,netField_t *fields,int count,uint32_t *words) {
 for(int i=0;i<count;i++)memcpy((byte*)record+fields[i].offset,&words[i],4);
}
static void get(void *record,netField_t *fields,int count,uint32_t *words) {
 for(int i=0;i<count;i++)memcpy(&words[i],(byte*)record+fields[i].offset,4);
}
static void arrays(playerState_t *p,uint32_t *words,int output) {
 int *parts[]={p->stats,p->persistant,p->ammo,p->powerups};
 for(int i=0;i<4;i++) {
  if(output)memcpy(words+48+16*i,parts[i],64);else memcpy(parts[i],words+48+16*i,64);
 }
}
int main(void) {
 Huff_Init(&msgHuff);for(int i=0;i<256;i++)for(int j=0;j<msg_hData[i];j++)Huff_addRef(&msgHuff.compressor,(byte)i);
 msgHuff.decompressor=msgHuff.compressor;msgHuff.decompressor.tree=msgHuff.compressor.tree;
 byte mode,flags;uint16_t number;uint32_t from[112],to[112];
 while(fread(&mode,1,1,stdin)==1) {
  if(mode>2||fread(&flags,1,1,stdin)!=1||fread(&number,2,1,stdin)!=1||fread(from,4,112,stdin)!=112||fread(to,4,112,stdin)!=112)return 2;
  byte data[1400]={0};msg_t m={.data=data,.maxsize=sizeof(data)};
  entityState_t a={.number=number},b={.number=number},c={0};playerState_t p={0},q={0},r={0};
  if(mode==0) {
   put(&a,entityStateFields,51,from);put(&b,entityStateFields,51,to);
   MSG_WriteDeltaEntity(&m,&a,(flags&2)?NULL:&b,flags&1);
  } else if(mode==1) {
   put(&p,playerStateFields,48,from);arrays(&p,from,0);put(&q,playerStateFields,48,to);arrays(&q,to,0);
   MSG_WriteDeltaPlayerstate(&m,&p,&q);
  } else {
   client_frame_t x={0},y={0};q2_put(&x.ps,from);q2_put(&y.ps,to);SV_WritePlayerstateToClient(&x,&y,&m);
  }
  uint32_t header[2]={m.bit,m.cursize};fwrite(header,4,2,stdout);fwrite(data,1,m.cursize,stdout);
  uint32_t decoded[112]={0},wire_number=mode==0?number:0;byte removed=0;m.bit=m.readcount=0;
  if(mode==0) {
   if(header[0]) {wire_number=MSG_ReadBits(&m,10);MSG_ReadDeltaEntity(&m,&a,&c,wire_number);removed=(flags&2)!=0;}
   else c=a;
   get(&c,entityStateFields,51,decoded);
  } else if(mode==1) {MSG_ReadDeltaPlayerstate(&m,&p,&r);get(&r,playerStateFields,48,decoded);arrays(&r,decoded,1);}
  else {player_state_t x={0},y={0};q2_put(&x,from);q2_decode(&m,&x,&y);q2_get(&y,decoded);}
  fwrite(decoded,4,112,stdout);fwrite(&wire_number,4,1,stdout);fwrite(&removed,1,1,stdout);
 }
 return ferror(stdin)?3:0;
}
'''
    code = evidence / 'original-state-delta.c'
    code.write_text(source)
    binary = evidence / 'original-state-delta'
    subprocess.run(['cc', '-O2', '-std=c11', '-fno-strict-aliasing', '-ffp-contract=off', str(code), '-o', str(binary)], check=True)
    return binary, layouts(msg) + [q2_layout()]


def fixture(tables):
    rng = random.Random(8606851)
    output = bytearray()
    floats = [-0.0, 0.0, -4096.0, -4097.0, 4095.0, 4096.0, 0.125, -0.125, 123456.75]
    for mode, table in enumerate(tables):
        for case in range(2048):
            old, new = [0] * 112, [0] * 112
            for i, (_, width) in enumerate(table):
                if mode == 2:
                    if 13 <= i <= 21 or 24 <= i <= 34:
                        bounds = (0,1) if 30 <= i <= 33 else (-32,32) if i < 16 or 19 <= i <= 29 else (-1024,1024)
                        value = lambda: struct.unpack('<I', struct.pack('<f', rng.uniform(*bounds)))[0]
                    elif i < 13:
                        value = lambda: rng.randrange(-(1 << 15), 1 << 15) & 0xffffffff if width == -16 else rng.randrange(256)
                    else:
                        value = lambda: rng.getrandbits(32)
                elif width == 0:
                    value = lambda: struct.unpack('<I', struct.pack('<f', rng.choice(floats) if case < 512 else rng.uniform(-1000000, 1000000)))[0]
                else:
                    value = lambda: rng.getrandbits(32)
                old[i] = value()
                # Cover every last-changed count, including unchanged and a single late change.
                change = i == case - 1 if case <= len(table) else rng.randrange(4) == 0
                new[i] = value() if change else old[i]
                if change and new[i] == old[i]:
                    new[i] = struct.unpack('<I', struct.pack('<f', 0.25))[0] if width == 0 or mode == 2 and (13 <= i <= 21 or 24 <= i <= 34) else old[i] ^ 1
            if mode == 1 and case > len(table):
                for i in range(48, 112):
                    old[i] = rng.getrandbits(32)
                    new[i] = rng.getrandbits(32) if rng.randrange(4) == 0 else old[i]
            if mode == 2:
                for i in range(36,68):
                    old[i] = rng.randrange(-32768,32768) & 0xffffffff
                    new[i] = rng.randrange(-32768,32768) & 0xffffffff if case > 36 and rng.randrange(4) == 0 else old[i]
            if case >= 256 and case % 8 == 0:
                new = old.copy()
            flags = case % 2 | (2 if mode == 0 and case % 17 == 0 else 0)
            output += struct.pack('<BBH224I', mode, flags, case % 1023, *old, *new)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--probe', type=Path, required=True)
    args = parser.parse_args(); args.evidence.mkdir(parents=True, exist_ok=True)
    binary, tables = compile_reference(args.qsrc, args.evidence)
    # Match the production table data against qsrc, rather than letting two copied lists agree.
    rust = (ROOT / 'crates/network/src/states.rs').read_text()
    for table, label in zip(tables[:2], ['ENTITY_LAYOUT', 'PLAYER_LAYOUT']):
        block = re.search(r'pub const ' + label + r'.*?=\s*\[(.*?)\n\];', rust, re.S).group(1)
        actual = [(name, int(width)) for name, width in re.findall(r'\("([^"]+)",\s*(-?\d+)\)', block)]
        assert actual == table, f'{label} differs from original qsrc'
    data = fixture(tables)
    expected = subprocess.check_output([binary], input=data)
    actual = subprocess.check_output([args.probe, '--compare'], input=data)
    for name, contents in [('fixture.bin', data), ('original.bin', expected), ('rust.bin', actual)]:
        (args.evidence / name).write_bytes(contents)
    if actual != expected:
        at = next((i for i, (a, b) in enumerate(zip(actual, expected)) if a != b), min(len(actual), len(expected)))
        raise AssertionError(f'native state bytes/decoded fields differ at output byte {at}; lengths {len(actual)}/{len(expected)}')
    result = dict(result='PASS', cases=6144, entity_fields=51, player_fields=48, player_arrays=64, q2_player_fields=36, q2_stats=32, bytes=len(actual), byte_exact=True, decoded_words_exact=True,
                  original='Q3 MSG entity/player and Q2 server player writer/client parser, struct layouts and Huffman unchanged; offsetof and private bindings only',
                  limits='Seeded native delta records; no snapshot framing, common-state ABI projection, sign-on, captures, live or installed acceptance')
    (args.evidence / 'comparison.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
