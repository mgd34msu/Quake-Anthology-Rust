#!/usr/bin/env python3
"""THE-860 native usercmd field-table oracle; original C is developer-only."""
import argparse
import json
import pathlib
import random
import re
import struct
import subprocess
from check_message import PREAMBLE, function

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINDINGS = r'''
typedef msg_t sizebuf_t;
typedef struct {float angles[3];int16_t forwardmove,sidemove,upmove;byte buttons,impulse,msec;} qwcmd_t;
typedef struct {int16_t angles[3],forwardmove,sidemove,upmove;byte buttons,impulse,msec,lightlevel;} q2cmd_t;
typedef struct {int serverTime,angles[3],buttons;signed char forwardmove,rightmove,upmove;byte weapon;} usercmd_t;
typedef struct {float angles[3],forwardmove,sidemove;byte buttons,msec;uint32_t server_frame;} rrcmd_t;
static msg_t stream;
static void byte_write(msg_t *msg,int value,int width) {
 for(int i=0;i<width/8;i++)msg->data[msg->cursize++]=(uint32_t)value>>(8*i);
 msg->bit=msg->cursize*8;
}
static int byte_read(int width) {
 uint32_t v=0;for(int i=0;i<width/8;i++)v|=(uint32_t)stream.data[stream.readcount++]<<(8*i);
 stream.bit=stream.readcount*8;return width==16?(int16_t)v:(int)v;
}
int oldsize;
'''
DRIVER = r'''
static float floating(uint32_t v) {float f;memcpy(&f,&v,4);return f;}
static uint32_t word(float f) {uint32_t v;memcpy(&v,&f,4);return v;}
static qwcmd_t qw(uint32_t *v) {return (qwcmd_t){.angles={floating(v[0]),floating(v[1]),floating(v[2])},.forwardmove=v[3],.sidemove=v[4],.upmove=v[5],.buttons=v[6],.impulse=v[7],.msec=v[8]};}
static q2cmd_t q2(uint32_t *v) {return (q2cmd_t){.angles={v[0],v[1],v[2]},.forwardmove=v[3],.sidemove=v[4],.upmove=v[5],.buttons=v[6],.impulse=v[7],.msec=v[8],.lightlevel=v[9]};}
static usercmd_t q3(uint32_t *v) {return (usercmd_t){.angles={v[0],v[1],v[2]},.forwardmove=v[3],.rightmove=v[4],.upmove=v[5],.buttons=v[6],.weapon=v[7],.serverTime=v[10]};}
static rrcmd_t rr(uint32_t *v) {return (rrcmd_t){.angles={floating(v[0]),floating(v[1]),floating(v[2])},.forwardmove=floating(v[3]),.sidemove=floating(v[4]),.buttons=v[6],.msec=v[8],.server_frame=v[10]};}
int main(void) {
 Huff_Init(&msgHuff);
 for(int i=0;i<256;i++)for(int j=0;j<msg_hData[i];j++)Huff_addRef(&msgHuff.compressor,(byte)i);
 msgHuff.decompressor=msgHuff.compressor;msgHuff.decompressor.tree=msgHuff.compressor.tree;
 byte mode,data[256];uint32_t key,from[11],to[11];
 while(fread(&mode,1,1,stdin)==1) {
  if(mode>3 || fread(&key,4,1,stdin)!=1 || fread(from,4,11,stdin)!=11 || fread(to,4,11,stdin)!=11)return 2;
  memset(data,0,sizeof(data));stream=(msg_t){.data=data,.maxsize=sizeof(data),.oob=mode!=2};
  qwcmd_t a0=qw(from),b0=qw(to),c0={0};q2cmd_t a1=q2(from),b1=q2(to),c1={0};usercmd_t a2=q3(from),b2=q3(to),c2={0};
  rrcmd_t a3=rr(from),b3=rr(to),c3={0};
  if(mode==0)QW_WriteDeltaUsercmd(&stream,&a0,&b0);
  else if(mode==1)Q2_WriteDeltaUsercmd(&stream,&a1,&b1);
  else if(mode==2)MSG_WriteDeltaUsercmdKey(&stream,key,&a2,&b2);
  else {RR_WriteDeltaUsercmd(&a3,&b3,1038,0);byte_write(&stream,0,8);}
  uint32_t header[2]={stream.bit,stream.cursize};fwrite(header,4,2,stdout);fwrite(data,1,stream.cursize,stdout);
  stream.bit=stream.readcount=0;uint32_t decoded[11]={0};
  if(mode==0) {QW_ReadDeltaUsercmd(&a0,&c0);for(int i=0;i<3;i++)decoded[i]=word(c0.angles[i]);decoded[3]=(int)c0.forwardmove;decoded[4]=(int)c0.sidemove;decoded[5]=(int)c0.upmove;decoded[6]=c0.buttons;decoded[7]=c0.impulse;decoded[8]=c0.msec;}
  else if(mode==1) {Q2_ReadDeltaUsercmd(&stream,&a1,&c1);for(int i=0;i<3;i++)decoded[i]=(int)c1.angles[i];decoded[3]=(int)c1.forwardmove;decoded[4]=(int)c1.sidemove;decoded[5]=(int)c1.upmove;decoded[6]=c1.buttons;decoded[7]=c1.impulse;decoded[8]=c1.msec;decoded[9]=c1.lightlevel;}
  else if(mode==2) {MSG_ReadDeltaUsercmdKey(&stream,key,&a2,&c2);for(int i=0;i<3;i++)decoded[i]=c2.angles[i];decoded[3]=(int)c2.forwardmove;decoded[4]=(int)c2.rightmove;decoded[5]=(int)c2.upmove;decoded[6]=c2.buttons;decoded[7]=c2.weapon;decoded[10]=c2.serverTime;}
  else {RR_ReadDeltaUsercmd(&a3,&c3);for(int i=0;i<3;i++)decoded[i]=word(c3.angles[i]);decoded[3]=word(c3.forwardmove);decoded[4]=word(c3.sidemove);decoded[6]=c3.buttons;decoded[8]=c3.msec;decoded[10]=c3.server_frame;}
  fwrite(decoded,4,11,stdout);
 }
 return ferror(stdin)?3:0;
}
'''


def compile_reference(qsrc, evidence):
    msg = (qsrc / 'quake-iii-arena/code/qcommon/msg.c').read_text()
    huff = re.sub(r'^#include.*$', '', (qsrc / 'quake-iii-arena/code/qcommon/huffman.c').read_text(), flags=re.M)
    freq = re.search(r'int msg_hData\[256\] = \{.*?\n\};', msg, re.S).group()
    source = PREAMBLE + huff + freq + function(msg, 'MSG_WriteBits') + function(msg, 'MSG_ReadBits') + BINDINGS
    for family, path, flags in [('QW', 'quake/QW/client/common.c', [1, 128, 2, 4, 8, 16, 32, 64]), ('Q2', 'quake-2/qcommon/common.c', [1, 2, 4, 8, 16, 32, 64, 128])]:
        original = (qsrc / path).read_text()
        names = ['CM_ANGLE1', 'CM_ANGLE2', 'CM_ANGLE3', 'CM_FORWARD', 'CM_SIDE', 'CM_UP', 'CM_BUTTONS', 'CM_IMPULSE']
        defines = {'usercmd_t': family.lower() + 'cmd_t', 'MSG_WriteDeltaUsercmd': family + '_WriteDeltaUsercmd', 'MSG_ReadDeltaUsercmd': family + '_ReadDeltaUsercmd',
                   'MSG_WriteByte(m,v)': 'byte_write(m,v,8)', 'MSG_WriteShort(m,v)': 'byte_write(m,v,16)', 'MSG_ReadByte(...)': 'byte_read(8)', 'MSG_ReadShort(...)': 'byte_read(16)',
                   'MSG_WriteAngle16': 'QW_WriteAngle16', 'MSG_ReadAngle16': 'QW_ReadAngle16'}
        defines.update(zip(names, map(str, flags)))
        source += ''.join(f'\n#define {k} {v}\n' for k, v in defines.items())
        if family == 'QW':
            source += function(original, 'MSG_WriteAngle16') + function(original, 'MSG_ReadAngle16')
        source += function(original, 'MSG_WriteDeltaUsercmd') + function(original, 'MSG_ReadDeltaUsercmd')
        source += ''.join(f'\n#undef {k.split("(")[0]}\n' for k in defines)
    original = (qsrc / 'q2repro/src/common/msg.c').read_text()
    defines = {
        'usercmd_t': 'rrcmd_t', 'Q_assert': 'assert',
        'nullUserCmd': 'rrNull',
        'MSG_WriteDeltaUsercmd': 'RR_WriteDeltaUsercmd',
        'MSG_ReadDeltaUsercmd': 'RR_ReadDeltaUsercmd',
        'MSG_WriteAngle16': 'RR_WriteAngle16', 'MSG_ReadAngle16': 'RR_ReadAngle16',
        'MSG_WriteByte(v)': 'byte_write(&stream,v,8)',
        'MSG_WriteChar(v)': 'byte_write(&stream,v,8)',
        'MSG_WriteShort(v)': 'byte_write(&stream,v,16)',
        'MSG_ReadByte()': 'byte_read(8)', 'MSG_ReadShort()': 'byte_read(16)',
        'PROTOCOL_VERSION_RERELEASE': '1038', 'PROTOCOL_VERSION_R1Q2_UCMD': '1904',
        'ANGLE2SHORT(x)': '((int)((x)*65536/360)&65535)',
        'SHORT2ANGLE(x)': '((x)*(360.0f/65536))',
        'BUTTON_ATTACK': '1', 'BUTTON_USE': '2', 'BUTTON_ANY': '128',
        'BUTTON_JUMP': '8', 'BUTTON_CROUCH': '16',
        'BUTTON_FORWARD': '4', 'BUTTON_SIDE': '8', 'BUTTON_UP': '16',
        'BUTTON_MASK': '(BUTTON_ATTACK|BUTTON_USE|BUTTON_ANY)',
    }
    defines.update(zip(['CM_ANGLE1','CM_ANGLE2','CM_ANGLE3','CM_FORWARD','CM_SIDE','CM_UP','CM_BUTTONS','CM_IMPULSE'],map(str,[1,2,4,8,16,32,64,128])))
    source += ''.join(f'\n#define {k} {v}\n' for k,v in defines.items())
    source += '\n#include <stdbool.h>\n#include <assert.h>\nstatic const rrcmd_t rrNull;\n'
    source += ''.join(function(original,n) for n in ['MSG_WriteAngle16','MSG_ReadAngle16','compute_buttons_upmove','MSG_WriteDeltaUsercmd','MSG_ReadDeltaUsercmd'])
    source += ''.join(f'\n#undef {k.split("(")[0]}\n' for k in defines)
    source += re.search(r'int kbitmask\[32\] = \{.*?\n\};', msg, re.S).group()
    source += ''.join(function(msg, n) for n in ['MSG_WriteDeltaKey', 'MSG_ReadDeltaKey', 'MSG_WriteDeltaUsercmdKey', 'MSG_ReadDeltaUsercmdKey']) + DRIVER
    code = evidence / 'original-command-delta.c'
    code.write_text(source)
    binary = evidence / 'original-command-delta'
    subprocess.run(['cc', '-O2', '-std=c11', '-ffp-contract=off', str(code), '-o', str(binary)], check=True)
    return binary


def fixture():
    rand = random.Random(860949)
    data = bytearray()
    for mode in range(4):
        for seed in range(1024):
            old, new = [0] * 11, [0] * 11
            for field in range(8):
                if (mode == 0 and field < 3) or (mode == 3 and field < 5):
                    a = struct.unpack('<I', struct.pack('<f', rand.uniform(-4096, 4096)))[0]
                    b = struct.unpack('<I', struct.pack('<f', rand.uniform(-4096, 4096)))[0]
                elif field < 6:
                    width = 8 if mode == 2 and field >= 3 else 16
                    a, b = (rand.randrange(-(1 << (width - 1)), 1 << (width - 1)) for _ in range(2))
                else:
                    width = 16 if mode == 2 and field == 6 else 8
                    a, b = rand.getrandbits(width), rand.getrandbits(width)
                if b == a:
                    b = a ^ 1
                old[field] = a & 0xffffffff
                new[field] = (b if seed & (1 << field) else a) & 0xffffffff
            if mode != 2:
                old[8], new[8] = rand.randrange(256), rand.randrange(256)
                if mode == 1:
                    old[9], new[9] = rand.randrange(256), rand.randrange(256)
                if mode == 3:
                    old[5] = new[5] = old[7] = new[7] = 0
                    old[10], new[10] = rand.getrandbits(32), rand.getrandbits(32)
            else:
                old[10] = rand.randrange(100000)
                new[10] = old[10] + [0, 1, 255, 256, 1000, -1, -255, 100000][seed % 8]
            if mode in (0,3) and seed < 8:
                old[0] = 0x80000000
                new[0] = 0 if seed % 2 == 0 else 0x80000000
                if seed == 2:
                    old[1], new[1] = (struct.unpack('<I', struct.pack('<f', f))[0] for f in [0.001, 0.002])
            if mode == 3 and seed < 16:
                endpoints = [-32768.75, 32767.75, -65535.5, 65535.5, -0.0, 0.0, -1.75, 1.75]
                old[3], new[3] = (struct.unpack('<I', struct.pack('<f', f))[0] for f in [endpoints[seed%8], endpoints[(seed+1)%8]])
            data += struct.pack('<BI22I', mode, rand.getrandbits(32), *old, *[v & 0xffffffff for v in new])
    return bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=pathlib.Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=pathlib.Path, required=True)
    parser.add_argument('--probe', type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    binary = compile_reference(args.qsrc, args.evidence)
    data = fixture()
    expected = subprocess.check_output([binary], input=data)
    actual = subprocess.check_output([args.probe, '--compare'], input=data)
    for name, contents in [('fixture.bin', data), ('original.bin', expected), ('rust.bin', actual)]:
        (args.evidence / name).write_bytes(contents)
    if actual != expected:
        at = next((i for i, (a, b) in enumerate(zip(actual, expected)) if a != b), min(len(actual), len(expected)))
        raise AssertionError(f'native command bytes/decoded fields differ at output byte {at}')
    masks = {0:set(),1:set(),3:set()}
    at = 0
    for case in range(len(data) // 93):
        _, length = struct.unpack_from('<II', expected, at)
        mode = data[case * 93]
        if mode in masks:
            masks[mode].add(expected[at + 8])
        at += 8 + length + 44
    assert at == len(expected) and len(masks[0])==len(masks[1])==256 and len(masks[3])==64
    result = {'cases': len(data) // 93, 'protocols': ['QW', 'Q2', 'Q3','Q2repro1038'], 'byte_exact': True, 'decoded_fields_exact': True,
              'native_mask_counts': {'QW': len(masks[0]), 'Q2': len(masks[1]), 'Q2repro1038':len(masks[3])},
              'original': 'Unchanged native command deltas, QW/Q2repro angle helpers and Q3 MSG/Huff; byte/struct bindings, Q2repro zero legacy light byte',
              'limits': 'Command delta records only; no full command packets, NQ/KEX records, rerelease snapshots/framing, Q3 XOR, live or installed acceptance'}
    (args.evidence / 'comparison.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
