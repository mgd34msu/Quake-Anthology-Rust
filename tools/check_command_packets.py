#!/usr/bin/env python3
"""THE-860 original native move packet construction and decoded-field oracle."""
import argparse
import json
import pathlib
import random
import re
import struct
import subprocess
from check_command_delta import compile_reference as delta_reference
from check_hull_trace import function

ROOT = pathlib.Path(__file__).resolve().parents[1]


def block(text, start, end):
    at = text.index(start)
    return text[at:text.index(end, at)]


def compile_reference(qsrc, evidence):
    delta_reference(qsrc, evidence)
    source = (evidence / 'original-command-delta.c').read_text().split('int main(void)')[0]
    crc = (qsrc / 'quake-2/qcommon/crc.c').read_text()
    source += re.sub(r'^#include.*$', '', crc, flags=re.M)
    source += '\nvoid Sys_Error(char *fmt,...) {(void)fmt;abort();}\n'
    for family, path in [('QW', 'quake/QW/client/common.c'), ('Q2', 'quake-2/qcommon/common.c')]:
        original = (qsrc / path).read_text()
        source += re.search(r'static byte chktbl\[[^\]]+\] = \{.*?\};', original, re.S).group().replace('chktbl', family + 'Table')
        source += function(original, 'COM_BlockSequenceCRCByte').replace('chktbl', family + 'Table').replace('COM_BlockSequenceCRCByte', family + 'Sequence')
    source += r'''
static byte wire[1400];
static void float_write(msg_t *m,float f) {uint32_t v;memcpy(&v,&f,4);byte_write(m,v,32);}
static int in_impulse;
static struct {int state;} in_attack,in_jump;
static struct {qwcmd_t cmd;float viewangles[3],mtime[2];int movemessages;} q1cl;
static struct {int demoplayback;void *netcon;} q1cls;
static int NET_SendUnreliableMessage(void *c,msg_t *m) {(void)c;memcpy(wire,m->data,m->cursize);stream=*m;stream.data=wire;return 0;}
static void CL_Disconnect(void) {abort();}
static void Con_Printf(char *fmt,...) {(void)fmt;abort();}
'''
    original = (qsrc / 'quake/WinQuake/cl_input.c').read_text()
    source += '''
#define cl q1cl
#define cls q1cls
#define usercmd_t qwcmd_t
#define clc_move 3
#define MSG_WriteByte(m,v) byte_write(m,v,8)
#define MSG_WriteShort(m,v) byte_write(m,v,16)
#define MSG_WriteFloat float_write
'''
    source += function((qsrc / 'quake/WinQuake/common.c').read_text(), 'MSG_WriteAngle')
    source += '\n#define MSG_ReadChar() ((int8_t)byte_read(8))\n'
    source += function((qsrc / 'quake/WinQuake/common.c').read_text(), 'MSG_ReadAngle')
    source += function(original, 'CL_SendMove')
    source += '\n#undef cl\n#undef cls\n#undef usercmd_t\n#undef clc_move\n'
    qw = (qsrc / 'quake/QW/client/cl_input.c').read_text()
    qw_block = block(qw, '\tbuf.maxsize = 128;', '\t// request delta compression')
    source += r'''
static struct {struct {qwcmd_t cmd;} frames[64];} qwcl;
static struct {struct {int outgoing_sequence;} netchan;} qwcls;
static int loss;
static int CL_CalcNet(void) {return loss;}
#define cl qwcl
#define cls qwcls
#define clc_move 3
#define UPDATE_MASK 63
#define COM_BlockSequenceCRCByte QWSequence
#define MSG_WriteDeltaUsercmd QW_WriteDeltaUsercmd
static void qw_packet(int seq_hash) {msg_t buf;byte *data=wire;int i,lost,checksumIndex;qwcmd_t *cmd,*oldcmd,nullcmd={0};
'''
    source += qw_block + '\nstream=buf;stream.bit=stream.cursize*8;}\n'
    source += '\n#undef cl\n#undef cls\n#undef clc_move\n#undef COM_BlockSequenceCRCByte\n#undef MSG_WriteDeltaUsercmd\n'
    q2 = (qsrc / 'quake-2/client/cl_input.c').read_text()
    q2_block = block(q2, '\t// begin a client move command', '\t// deliver the message')
    source += r'''
static struct {struct {int valid,serverframe;} frame;int demowaiting;q2cmd_t cmds[64];} q2cl;
static struct {struct {int outgoing_sequence;} netchan;int demowaiting;} q2cls;
static struct {float value;} q2nodelta={1};
#define cl q2cl
#define cls q2cls
#define clc_move 2
#define cl_nodelta (&q2nodelta)
#define CMD_BACKUP 64
#define COM_BlockSequenceCRCByte Q2Sequence
#define MSG_WriteDeltaUsercmd Q2_WriteDeltaUsercmd
#define MSG_WriteLong(m,v) byte_write(m,v,32)
static void q2_packet(void) {msg_t buf={.data=wire,.maxsize=sizeof(wire)};int i,checksumIndex;q2cmd_t *cmd,*oldcmd,nullcmd;
'''
    source += q2_block + '\nstream=buf;stream.bit=stream.cursize*8;}\n'
    source += '\n#undef cl\n#undef cls\n#undef clc_move\n#undef COM_BlockSequenceCRCByte\n#undef MSG_WriteDeltaUsercmd\n#undef MSG_WriteByte\n#undef MSG_WriteShort\n#undef MSG_WriteLong\n#undef MSG_WriteFloat\n'
    q3 = (qsrc / 'quake-iii-arena/code/client/cl_input.c').read_text()
    source += r'''
static struct {int serverId,cmdNumber;usercmd_t cmds[64];} cl;
static struct {int serverMessageSequence,serverCommandSequence,checksumFeed,challenge;byte serverCommands[64][256];} clc;
#define MAX_RELIABLE_COMMANDS 64
#define CMD_MASK 63
#define CL_ENCODE_START 12
static void MSG_WriteByte(msg_t *m,int v) {MSG_WriteBits(m,v,8);}
static void MSG_WriteLong(msg_t *m,int v) {MSG_WriteBits(m,v,32);}
static int MSG_ReadLong(msg_t *m) {return MSG_ReadBits(m,32);}
'''
    source += function((qsrc / 'quake-iii-arena/code/qcommon/common.c').read_text(), 'Com_HashKey')
    source += function((qsrc / 'quake-iii-arena/code/client/cl_net_chan.c').read_text(), 'CL_Netchan_Encode')
    source += '\nstatic void q3_packet(void) {msg_t buf={.data=wire,.maxsize=sizeof(wire)};int count=3,i,j,key;usercmd_t nullcmd={0},*cmd,*oldcmd=&nullcmd;\n'
    source += block(q3, '\t// write the current serverId', '\t// write any unacknowledged clientCommands')
    source += '\nMSG_WriteByte(&buf,3);\n'
    source += block(q3, '\t\t// write the command count', '\n\t}\n\n\t//\n\t// deliver the message')
    source += '\nMSG_WriteByte(&buf,5);CL_Netchan_Encode(&buf);stream=buf;}\n'
    source += r'''
int main(void) {
 Huff_Init(&msgHuff);for(int i=0;i<256;i++)for(int j=0;j<msg_hData[i];j++)Huff_addRef(&msgHuff.compressor,(byte)i);
 msgHuff.decompressor=msgHuff.compressor;msgHuff.decompressor.tree=msgHuff.compressor.tree;
 byte mode,len;uint32_t sequence,context[5],rows[3][11];float timestamp;
 while(fread(&mode,1,1,stdin)==1) {
  if(fread(&sequence,4,1,stdin)!=1||fread(context,4,5,stdin)!=5||fread(&len,1,1,stdin)!=1)return 2;loss=len;
  if(fread(&timestamp,4,1,stdin)!=1||fread(rows,4,33,stdin)!=33||fread(&len,1,1,stdin)!=1)return 2;
  byte text[256]={0};if(fread(text,1,len,stdin)!=len)return 2;
  memset(wire,0,sizeof(wire));
  if(mode==0) {qwcmd_t c=qw(rows[2]);memcpy(q1cl.viewangles,c.angles,12);q1cl.mtime[0]=timestamp;q1cl.movemessages=3;in_attack.state=c.buttons&1;in_jump.state=(c.buttons&2)?1:0;in_impulse=c.impulse;CL_SendMove(&c);}
  else if(mode==1) {qwcls.netchan.outgoing_sequence=sequence;for(int i=0;i<3;i++)qwcl.frames[(sequence-2+i)&63].cmd=qw(rows[i]);qw_packet(sequence);}
  else if(mode==2) {q2cls.netchan.outgoing_sequence=sequence;for(int i=0;i<3;i++)q2cl.cmds[(sequence-2+i)&63]=q2(rows[i]);q2_packet();}
  else if(mode==3) {cl.serverId=context[0];cl.cmdNumber=3;clc.serverMessageSequence=context[1];clc.serverCommandSequence=context[2];clc.challenge=context[3];clc.checksumFeed=context[4];memcpy(clc.serverCommands[context[2]&63],text,256);for(int i=0;i<3;i++)cl.cmds[i+1]=q3(rows[i]);q3_packet();}
  else return 2;
  uint32_t n=stream.cursize;fwrite(&n,4,1,stdout);fwrite(wire,1,n,stdout);
  uint32_t decoded[3][11]={{0}};
  if(mode==0) {stream.readcount=5;for(int i=0;i<3;i++)decoded[2][i]=word(MSG_ReadAngle());for(int i=3;i<6;i++)decoded[2][i]=(int)byte_read(16);decoded[2][6]=byte_read(8);decoded[2][7]=byte_read(8);}
  else if(mode==1) {stream.readcount=3;qwcmd_t a={0},b;for(int i=0;i<3;i++){QW_ReadDeltaUsercmd(&a,&b);for(int j=0;j<3;j++)decoded[i][j]=word(b.angles[j]);decoded[i][3]=(int)b.forwardmove;decoded[i][4]=(int)b.sidemove;decoded[i][5]=(int)b.upmove;decoded[i][6]=b.buttons;decoded[i][7]=b.impulse;decoded[i][8]=b.msec;a=b;}}
  else if(mode==2) {stream.readcount=6;q2cmd_t a={0},b;for(int i=0;i<3;i++){Q2_ReadDeltaUsercmd(&stream,&a,&b);for(int j=0;j<3;j++)decoded[i][j]=(int)b.angles[j];decoded[i][3]=(int)b.forwardmove;decoded[i][4]=(int)b.sidemove;decoded[i][5]=(int)b.upmove;decoded[i][6]=b.buttons;decoded[i][7]=b.impulse;decoded[i][8]=b.msec;decoded[i][9]=b.lightlevel;a=b;}}
  else {CL_Netchan_Encode(&stream);stream.bit=stream.readcount=0;for(int i=0;i<3;i++)MSG_ReadLong(&stream);MSG_ReadBits(&stream,8);MSG_ReadBits(&stream,8);int key=clc.checksumFeed^clc.serverMessageSequence^Com_HashKey((char*)text,32);usercmd_t a={0},b;for(int i=0;i<3;i++){MSG_ReadDeltaUsercmdKey(&stream,key,&a,&b);for(int j=0;j<3;j++)decoded[i][j]=b.angles[j];decoded[i][3]=(int)b.forwardmove;decoded[i][4]=(int)b.rightmove;decoded[i][5]=(int)b.upmove;decoded[i][6]=b.buttons;decoded[i][7]=b.weapon;decoded[i][10]=b.serverTime;a=b;}}
  fwrite(decoded,4,33,stdout);
 }
 return ferror(stdin)?3:0;
}
'''
    code = evidence / 'original-command-packets.c'
    code.write_text(source)
    binary = evidence / 'original-command-packets'
    subprocess.run(['cc', '-O2', '-std=c11', '-ffp-contract=off', str(code), '-o', str(binary)], check=True)
    return binary


def fixture():
    rand = random.Random(86015)
    data = bytearray()
    for mode in range(4):
        for case in range(512):
            sequence = rand.randrange(2, 0x7fffffff)
            context = [rand.randrange(1, 10000), rand.randrange(1, 4096), case & 63, rand.getrandbits(32), rand.getrandbits(32)]
            data += struct.pack('<BI5IBf', mode, sequence, *context, rand.randrange(256), rand.uniform(0, 100))
            for command in range(3):
                words = [0] * 11
                for i in range(3):
                    if mode < 2:
                        words[i] = struct.unpack('<I', struct.pack('<f', rand.uniform(-4096, 4096)))[0]
                    else:
                        words[i] = rand.randrange(65536)
                    words[i + 3] = rand.randrange(-127, 128) if mode == 3 else rand.randrange(-32768, 32768)
                words[6] = rand.randrange(4) if mode == 0 else rand.randrange(65536 if mode == 3 else 256)
                words[7], words[8], words[9] = (rand.randrange(256) for _ in range(3))
                words[10] = case * 1000 + command * 16 + 123
                data += struct.pack('<11I', *(v & 0xffffffff for v in words))
            text = [b'', b'print "hello"', b'cp "100% ready"', b'print "\x80high"'][case & 3]
            data += bytes([len(text)]) + text
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
    actual = subprocess.check_output([args.probe], input=data)
    for name, contents in [('fixture.bin', data), ('original.bin', expected), ('rust.bin', actual)]:
        (args.evidence / name).write_bytes(contents)
    if actual != expected:
        at = next((i for i, (a,b) in enumerate(zip(actual,expected)) if a != b), min(len(actual),len(expected)))
        raise AssertionError(f'native packet bytes/decoded fields differ at output byte {at}')
    result = {'cases': 2048, 'protocols': [15,28,34,68], 'wire_bytes_exact': True, 'native_decoded_fields_exact': True,
              'original': 'Unchanged CL_SendMove, original QW/Q2/Q3 packet construction blocks, CRC, Com_HashKey, CL_Netchan_Encode, native command readers and MSG/Huff; cold struct/byte bindings',
              'limits': 'Move-only packets; native command strings/handshake, NQ666/999 and rerelease transport remain open'}
    (args.evidence / 'comparison.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
