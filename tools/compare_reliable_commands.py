#!/usr/bin/env python3
"""THE-860 compare Q3 command payloads with unchanged original MSG/command/XOR functions."""
import argparse
import json
from pathlib import Path
import random
import re
import struct
import subprocess
from check_message import PREAMBLE
from check_hull_trace import function

ROOT = Path(__file__).resolve().parents[1]


def compile_reference(qsrc, evidence):
    msg = (qsrc / 'quake-iii-arena/code/qcommon/msg.c').read_text()
    huff = re.sub(r'^#include.*$', '', (qsrc / 'quake-iii-arena/code/qcommon/huffman.c').read_text(), flags=re.M)
    source = PREAMBLE + huff + re.search(r'int msg_hData\[256\] = \{.*?\n\};', msg, re.S).group()
    source += function(msg, 'MSG_WriteBits') + function(msg, 'MSG_ReadBits')
    source += r'''
#define MAX_STRING_CHARS 1024
#define MAX_RELIABLE_COMMANDS 64
#define ERR_FATAL 0
#define SV_ENCODE_START 4
#define CL_DECODE_START 4
#define svc_serverCommand 5
typedef struct { int reliableSequence,reliableAcknowledge,reliableSent,lastClientCommand,dropped;
 char reliableCommands[64][1024],lastClientCommandString[1024];
 struct {int outgoingSequence;} netchan;int challenge; } client_t;
static struct {int challenge;char reliableCommands[64][1024];} clc;
static void SV_DropClient(client_t *c,char *s) {(void)s;c->dropped=1;}
static void Com_Printf(char *fmt,...) {(void)fmt;}
'''
    source += function((qsrc / 'quake-iii-arena/code/game/q_shared.c').read_text(), 'Q_strncpyz')
    for name in ['MSG_WriteByte', 'MSG_WriteLong', 'MSG_WriteData', 'MSG_WriteString', 'MSG_ReadByte', 'MSG_ReadLong', 'MSG_ReadString']:
        source += function(msg, name)
    source += function((qsrc / 'quake-iii-arena/code/server/sv_main.c').read_text(), 'SV_AddServerCommand')
    source += function((qsrc / 'quake-iii-arena/code/server/sv_snapshot.c').read_text(), 'SV_UpdateServerCommandsToClient')
    source += function((qsrc / 'quake-iii-arena/code/server/sv_net_chan.c').read_text(), 'SV_Netchan_Encode')
    source += function((qsrc / 'quake-iii-arena/code/client/cl_net_chan.c').read_text(), 'CL_Netchan_Decode')
    source += r'''
static char text[4096];
static char *read_text(void) {uint16_t n;if(fread(&n,2,1,stdin)!=1||n>=sizeof(text)||fread(text,1,n,stdin)!=n)abort();text[n]=0;return text;}
int main(void) {
 Huff_Init(&msgHuff);for(int i=0;i<256;i++)for(int j=0;j<msg_hData[i];j++)Huff_addRef(&msgHuff.compressor,(byte)i);
 msgHuff.decompressor=msgHuff.compressor;msgHuff.decompressor.tree=msgHuff.compressor.tree;
 uint32_t context[3],count;
 while(fread(context,4,3,stdin)==3) {
  client_t client={.challenge=context[1],.netchan={.outgoingSequence=1},.lastClientCommand=1};
  byte data[32768]={0},full[32772]={0},key_data[8192]={0};
  msg_t key={.data=key_data,.maxsize=sizeof(key_data)};
  char raw[1024];Q_strncpyz(raw,read_text(),sizeof(raw));MSG_WriteString(&key,raw);
  key.bit=key.readcount=0;Q_strncpyz(client.lastClientCommandString,MSG_ReadString(&key),sizeof(client.lastClientCommandString));
  clc.challenge=context[1];memset(clc.reliableCommands,0,sizeof(clc.reliableCommands));Q_strncpyz(clc.reliableCommands[1],raw,1024);
  if(fread(&count,4,1,stdin)!=1||count>64)return 2;
  for(uint32_t i=0;i<count;i++)SV_AddServerCommand(&client,read_text());
  msg_t m={.data=data,.maxsize=sizeof(data)};
  MSG_WriteLong(&m,client.lastClientCommand);SV_UpdateServerCommandsToClient(&client,&m);MSG_WriteByte(&m,8);SV_Netchan_Encode(&client,&m);
  uint32_t size=m.cursize;fwrite(&size,4,1,stdout);fwrite(data,1,size,stdout);
  uint32_t seq=1;memcpy(full,&seq,4);memcpy(full+4,data,size);
  msg_t parsed={.data=full,.maxsize=sizeof(full),.cursize=size+4,.readcount=4,.bit=32};CL_Netchan_Decode(&parsed);
  uint32_t ack=MSG_ReadLong(&parsed),rows=0,sequences[64];uint16_t lengths[64];char strings[64][1024];int opcode;
  while((opcode=MSG_ReadByte(&parsed))==5) {
   if(rows>=64)return 3;sequences[rows]=MSG_ReadLong(&parsed);Q_strncpyz(strings[rows],MSG_ReadString(&parsed),1024);lengths[rows]=strlen(strings[rows]);rows++;
  }
  fwrite(&ack,4,1,stdout);fwrite(&rows,4,1,stdout);
  for(uint32_t i=0;i<rows;i++) {fwrite(&sequences[i],4,1,stdout);fwrite(&lengths[i],2,1,stdout);fwrite(strings[i],1,lengths[i],stdout);}
  byte end=opcode;fwrite(&end,1,1,stdout);
 }
 return ferror(stdin)?4:0;
}
'''
    code = evidence / 'original-reliable-commands.c'
    code.write_text(source)
    binary = evidence / 'original-reliable-commands'
    subprocess.run(['cc', '-std=c11', '-O2', '-ffp-contract=off', str(code), '-o', str(binary)], check=True)
    return binary


def fixture():
    rand = random.Random(86068)
    out = bytearray()
    for case in range(512):
        out += struct.pack('<III', case + 1, rand.getrandbits(32), rand.getrandbits(32))
        client = b'userinfo "native %' + bytes([128 + case % 128]) + b'"'
        out += struct.pack('<H', len(client)) + client
        count = 1 + case % 64
        out += struct.pack('<I', count)
        for index in range(count):
            text = f'cp "case {case} command {index} % '.encode() + bytes([128 + index % 128]) + b'"'
            if case in (0, 128, 256, 384) and index == 0:
                text = b'y' * (1022 + case // 128)
            out += struct.pack('<H', len(text)) + text
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--rust', type=Path, default=ROOT / 'target/release/examples/reliable_commands')
    args = parser.parse_args(); args.evidence.mkdir(parents=True, exist_ok=False)
    binary = compile_reference(args.qsrc, args.evidence)
    data = fixture(); (args.evidence / 'fixture.bin').write_bytes(data)
    expected = subprocess.check_output([str(binary)], input=data)
    actual = subprocess.check_output([str(args.rust)], input=data)
    (args.evidence / 'original.bin').write_bytes(expected); (args.evidence / 'rust.bin').write_bytes(actual)
    if expected != actual:
        at = next((i for i, (a, b) in enumerate(zip(expected, actual)) if a != b), min(len(expected), len(actual)))
        raise ValueError(f'native reliable-command mismatch at byte {at}; lengths {len(expected)}/{len(actual)}')
    result = dict(result='PASS', cases=512, bytes=len(actual), scope='native server command payload bytes and parsed command strings/cursor; no sign-on or gameplay')
    (args.evidence / 'comparison.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
