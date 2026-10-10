#!/usr/bin/env python3
"""THE-860 original native snapshot/packet oracle; C stays developer-only."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess
from check_message import PREAMBLE, function
from check_state_delta import (reference_source, q2_reference, q2_entity_reference,
                               q2_layout, q2_entity_layout, qw_reference)

ROOT = Path(__file__).resolve().parents[1]

BINDINGS = r'''
#define cl snapshot_client
#define PACKET_BACKUP 32
#define PACKET_MASK 31
#define MAX_PARSE_ENTITIES 2048
#define CS_ACTIVE 4
#define SNAPFLAG_RATE_DELAYED 1
#define SNAPFLAG_NOT_ACTIVE 2
#define svc_snapshot 7
#define svc_nop 1
#define SHOWNET(m,s) ((void)0)
static void Com_DPrintf(char *fmt,...) {(void)fmt;}
typedef struct {int valid,serverCommandNum,serverTime,messageNum,deltaNum,snapFlags;
 int parseEntitiesNum,numEntities,ping;byte areamask[32];playerState_t ps;} clSnapshot_t;
static struct {clSnapshot_t snap,snapshots[32];int parseEntitiesNum,newSnapshots;
 entityState_t parseEntities[2048],entityBaselines[1024];
 struct {int p_serverTime,p_realtime;} outPackets[32];} cl;
static struct {int serverCommandSequence,serverMessageSequence,demowaiting;
 struct {int outgoingSequence;} netchan;} clc;
static struct {int realtime;} cls;
typedef struct {playerState_t ps;int first_entity,num_entities,areabytes;byte areabits[32];} clientSnapshot_t;
typedef struct {clientSnapshot_t frames[32];int deltaMessage,state,rateDelayed;
 char name[16];struct {int outgoingSequence;} netchan;} client_t;
static struct {entityState_t snapshotEntities[4096];
 int numSnapshotEntities,nextSnapshotEntities,time,snapFlagServerBit;} svs;
static struct {struct {entityState_t baseline;} svEntities[1024];} sv;
static struct {int integer;} padding;
#define sv_padPackets (&padding)
'''

DRIVER = r'''
static int entity_input(entityState_t *e) {
 uint16_t number;uint32_t words[51];
 if(fread(&number,2,1,stdin)!=1||fread(words,4,51,stdin)!=51)return 0;
 memset(e,0,sizeof(*e));e->number=number;put(e,entityStateFields,51,words);return 1;
}
static void parse(msg_t *m,int sequence) {
 m->bit=m->readcount=0;clc.serverMessageSequence=sequence;
 if(MSG_ReadByte(m)!=7)abort();CL_ParseSnapshot(m);if(MSG_ReadByte(m)!=8)abort();
}
int main(void) {
 Huff_Init(&msgHuff);for(int i=0;i<256;i++)for(int j=0;j<msg_hData[i];j++)Huff_addRef(&msgHuff.compressor,(byte)i);
 msgHuff.decompressor=msgHuff.compressor;msgHuff.decompressor.tree=msgHuff.compressor.tree;
 uint32_t cases;if(fread(&cases,4,1,stdin)!=1)return 2;
 for(uint32_t k=0;k<cases;k++) {
  byte h[4];uint16_t counts[3];uint32_t p[112],q[112];
  if(fread(h,1,4,stdin)!=4||fread(counts,2,3,stdin)!=3||counts[0]>64||counts[1]>64||counts[2]>64||h[3]>32
   ||fread(p,4,112,stdin)!=112||fread(q,4,112,stdin)!=112)return 3;
  memset(&cl,0,sizeof(cl));memset(&sv,0,sizeof(sv));memset(&svs,0,sizeof(svs));
  clc.serverCommandSequence=12;clc.netchan.outgoingSequence=1;
  client_t client={0};client.state=CS_ACTIVE;client.rateDelayed=h[2]&1;
  int sequence=h[0]?1+h[0]:2;clientSnapshot_t *old=&client.frames[1],*to=&client.frames[sequence&31];
  old->first_entity=0;old->num_entities=counts[0];old->areabytes=h[3];
  byte area[32]={0};if(fread(area,1,h[3],stdin)!=h[3])return 4;
  put(&old->ps,playerStateFields,48,p);arrays(&old->ps,p,0);
  for(int i=0;i<counts[2];i++) {entityState_t e;if(!entity_input(&e))return 5;
   sv.svEntities[e.number].baseline=e;cl.entityBaselines[e.number]=e;}
  for(int i=0;i<counts[0];i++)if(!entity_input(&svs.snapshotEntities[i]))return 6;
  /* Keep the old native frame separate when sequence wraps to slot one. */
  clientSnapshot_t saved=*old;
  memset(to,0,sizeof(*to));to->first_entity=128;to->num_entities=counts[1];to->areabytes=h[3];
  put(&to->ps,playerStateFields,48,q);arrays(&to->ps,q,0);memcpy(to->areabits,area,h[3]);
  for(int i=0;i<counts[1];i++)if(!entity_input(&svs.snapshotEntities[128+i]))return 7;
  svs.numSnapshotEntities=4096;svs.nextSnapshotEntities=128+counts[1];svs.snapFlagServerBit=h[2]&~1;
  if(h[1]) {
   clientSnapshot_t target=*to;client.frames[1]=saved;memcpy(client.frames[1].areabits,area,h[3]);
   byte first[8192]={0};msg_t m={.data=first,.maxsize=sizeof(first)};
   client.netchan.outgoingSequence=1;client.deltaMessage=-1;svs.time=100;
   SV_WriteSnapshotToClient(&client,&m);MSG_WriteByte(&m,8);parse(&m,1);
   *to=target;
  }
  cl.newSnapshots=0;client.netchan.outgoingSequence=sequence;client.deltaMessage=h[0]?1:-1;svs.time=300;
  byte wire[8192]={0};msg_t m={.data=wire,.maxsize=sizeof(wire)};
  SV_WriteSnapshotToClient(&client,&m);MSG_WriteByte(&m,8);
  uint32_t bits=m.bit,length=m.cursize;parse(&m,sequence);
  uint32_t header[4]={bits,length,cl.newSnapshots,m.bit};fwrite(header,4,4,stdout);fwrite(wire,1,length,stdout);
  if(cl.newSnapshots) {
   uint32_t words[112],meta[5]={cl.snap.serverTime,cl.snap.serverCommandNum,cl.snap.snapFlags,h[3],cl.snap.numEntities};
   get(&cl.snap.ps,playerStateFields,48,words);arrays(&cl.snap.ps,words,1);
   fwrite(meta,4,5,stdout);fwrite(cl.snap.areamask,1,h[3],stdout);fwrite(words,4,112,stdout);
   for(int i=0;i<cl.snap.numEntities;i++) {
    entityState_t *e=&cl.parseEntities[(cl.snap.parseEntitiesNum+i)&2047];uint32_t number=e->number,words[51];
    get(e,entityStateFields,51,words);fwrite(&number,4,1,stdout);fwrite(words,4,51,stdout);
   }
  }
 }
 return 0;
}
'''


def compile_reference(qsrc, evidence):
    source, tables = reference_source(qsrc)
    source = source.split('\nint main(void) {', 1)[0] + BINDINGS
    msg = (qsrc/'quake-iii-arena/code/qcommon/msg.c').read_text()
    for name in ['MSG_WriteData', 'MSG_ReadData']:
        source += function(msg, name)
    server = (qsrc/'quake-iii-arena/code/server/sv_snapshot.c').read_text()
    client = (qsrc/'quake-iii-arena/code/client/cl_parse.c').read_text()
    for name in ['SV_EmitPacketEntities', 'SV_WriteSnapshotToClient']:
        source += function(server, name)
    for name in ['CL_DeltaEntity', 'CL_ParsePacketEntities', 'CL_ParseSnapshot']:
        source += function(client, name)
    code = evidence/'original-snapshots.c'
    code.write_text(source + DRIVER)
    binary = evidence/'original-snapshots'
    subprocess.run(['cc', '-O2', '-std=c11', '-fno-strict-aliasing', '-ffp-contract=off', str(code), '-o', str(binary)], check=True)
    return binary, tables


def compile_q2_reference(qsrc, evidence):
    """Whole original frame/packet bodies; binding types provide private state."""
    source = PREAMBLE + '\ntypedef float vec3_t[3];\n#define true 1\n#define false 0\n#define ERR_FATAL 0\n'
    player = q2_reference(qsrc)
    player = player.replace('typedef struct {player_state_t ps;} client_frame_t;',
        'typedef struct {player_state_t ps;int first_entity,num_entities,areabytes;byte areabits[32];} client_frame_t;')
    player = player.replace('typedef struct {player_state_t playerstate;} frame_t;',
        'typedef struct {player_state_t playerstate;int valid,serverframe,deltaframe,servertime,parse_entities,num_entities;byte areabits[32];} frame_t;')
    player = player.replace('static struct {int attractloop;} cl;', r'''
static struct {int attractloop,parse_entities,time,surpressCount,force_refdef,
 servercount,refresh_prepped,sound_prepped;vec3_t predicted_origin,predicted_angles;
 frame_t frame,frames[32];} cl;
''')
    source += player + q2_entity_reference(qsrc) + r'''
#define UPDATE_BACKUP 32
#define UPDATE_MASK 31
#define MAX_PARSE_ENTITIES 1024
#define ca_active 3
#define svc_frame 20
#define svc_packetentities 18
#define MSG_WriteByte(m,v) q2_write(m,v,8)
#define MSG_WriteChar(m,v) q2_write(m,v,8)
#define MSG_WriteShort(m,v) q2_write(m,v,16)
#define MSG_WriteLong(m,v) q2_write(m,v,32)
#define MSG_ReadByte(m) q2_read(m,8,0)
#define MSG_ReadChar(m) q2_read(m,8,1)
#define MSG_ReadShort(m) q2_read(m,16,1)
#define MSG_ReadLong(m) q2_read(m,32,0)
#define MSG_WriteDeltaEntity Q2_WriteDeltaEntity
#define CL_ParseDelta Q2_ParseDelta
#define SHOWNET(x) ((void)0)
typedef struct {entity_state_t baseline,current,prev;int serverframe,trailcount;vec3_t lerp_origin;} centity_t;
static centity_t cl_entities[1024];static entity_state_t cl_parse_entities[1024];
static struct {int state,serverProtocol,demowaiting,disable_servercount;} cls;
typedef struct {client_frame_t frames[32];int lastframe,surpressCount;} client_t;
static struct {int framenum;entity_state_t baselines[1024];} sv;
static struct {entity_state_t client_entities[4096];int num_client_entities;} svs;
static struct {float value;} maxclients_value={16},shownet_value={0};
#define maxclients (&maxclients_value)
#define cl_shownet (&shownet_value)
static void Com_Printf(char *fmt,...) {(void)fmt;}
static void SCR_EndLoadingPlaque(void) {}
static void CL_FireEntityEvents(frame_t *f) {(void)f;}
static void CL_CheckPredictionError(void) {}
static void MSG_ReadData(msg_t *m,void *buffer,int n) {byte *out=buffer;for(int i=0;i<n;i++)out[i]=MSG_ReadByte(m);}
static void SZ_Write(msg_t *m,byte *data,int n) {for(int i=0;i<n;i++)MSG_WriteByte(m,data[i]);}
'''
    header = (qsrc/'quake-2/game/q_shared.h').read_text()
    end = header.index('} entity_event_t;') + len('} entity_event_t;')
    source += header[header.rfind('typedef enum', 0, end):end] + '\n'
    server = (qsrc/'quake-2/server/sv_ents.c').read_text()
    client = (qsrc/'quake-2/client/cl_ents.c').read_text()
    for name in ['SV_EmitPacketEntities', 'SV_WriteFrameToClient']:
        source += function(server, name)
    for name in ['CL_DeltaEntity', 'CL_ParsePacketEntities', 'CL_ParseFrame']:
        source += function(client, name)
    source += r'''
static int entity_input(entity_state_t *e) {
 uint16_t number;uint32_t words[20];
 if(fread(&number,2,1,stdin)!=1||fread(words,4,20,stdin)!=20)return 0;
 memset(e,0,sizeof(*e));e->number=number;q2_entity_put(e,words);return 1;
}
static void parse(msg_t *m) {
 net_message=*m;net_message.bit=net_message.readcount=0;
 if(MSG_ReadByte(&net_message)!=20)abort();CL_ParseFrame();
 if(MSG_ReadByte(&net_message)!=6)abort();
}
int main(void) {
 uint32_t cases;if(fread(&cases,4,1,stdin)!=1)return 2;
 for(uint32_t k=0;k<cases;k++) {
  byte h[4];uint16_t counts[3];uint32_t p[68],q[68];
  if(fread(h,1,4,stdin)!=4||fread(counts,2,3,stdin)!=3||counts[0]>64||counts[1]>64||counts[2]>64||h[3]>32
   ||fread(p,4,68,stdin)!=68||fread(q,4,68,stdin)!=68)return 3;
  memset(&cl,0,sizeof(cl));memset(&sv,0,sizeof(sv));memset(&svs,0,sizeof(svs));
  memset(cl_entities,0,sizeof(cl_entities));memset(cl_parse_entities,0,sizeof(cl_parse_entities));
  cls.state=ca_active;cls.serverProtocol=34;client_t client={0};
  int sequence=1+(h[0]?h[0]:1);client_frame_t *old=&client.frames[1],*to=&client.frames[sequence&31];
  old->first_entity=0;old->num_entities=counts[0];old->areabytes=h[3];
  byte area[32]={0};if(fread(area,1,h[3],stdin)!=h[3])return 4;
  q2_put(&old->ps,p);memcpy(old->areabits,area,h[3]);
  for(int i=0;i<counts[2];i++) {entity_state_t e;if(!entity_input(&e))return 5;
   sv.baselines[e.number]=e;cl_entities[e.number].baseline=e;}
  for(int i=0;i<counts[0];i++)if(!entity_input(&svs.client_entities[i]))return 6;
  client_frame_t saved=*old;
  memset(to,0,sizeof(*to));to->first_entity=128;to->num_entities=counts[1];to->areabytes=h[3];
  q2_put(&to->ps,q);memcpy(to->areabits,area,h[3]);
  for(int i=0;i<counts[1];i++)if(!entity_input(&svs.client_entities[128+i]))return 7;
  svs.num_client_entities=4096;
  if(h[1]) {
   client_frame_t target=*to;client.frames[1]=saved;byte first[8192]={0};msg_t m={.data=first,.maxsize=sizeof(first)};
   client.lastframe=-1;client.surpressCount=h[2];sv.framenum=1;
   SV_WriteFrameToClient(&client,&m);MSG_WriteByte(&m,6);parse(&m);*to=target;
  }
  client.lastframe=h[0]?1:-1;client.surpressCount=h[2];sv.framenum=sequence;
  byte wire[8192]={0};msg_t m={.data=wire,.maxsize=sizeof(wire)};
  SV_WriteFrameToClient(&client,&m);MSG_WriteByte(&m,6);parse(&m);
  uint32_t result[4]={m.cursize*8,m.cursize,cl.frame.valid,net_message.readcount*8};
  fwrite(result,4,4,stdout);fwrite(wire,1,m.cursize,stdout);
  if(cl.frame.valid) {
   uint32_t words[68],meta[5]={cl.frame.servertime,0,cl.surpressCount,h[3],cl.frame.num_entities};
   q2_get(&cl.frame.playerstate,words);fwrite(meta,4,5,stdout);fwrite(cl.frame.areabits,1,h[3],stdout);fwrite(words,4,68,stdout);
   for(int i=0;i<cl.frame.num_entities;i++) {
    entity_state_t *e=&cl_parse_entities[(cl.frame.parse_entities+i)&1023];uint32_t number=e->number,words[20];
    q2_entity_get(e,words);fwrite(&number,4,1,stdout);fwrite(words,4,20,stdout);
   }
  }
 }
 return 0;
}
'''
    code = evidence/'original-q2-frames.c'
    code.write_text(source)
    binary = evidence/'original-q2-frames'
    subprocess.run(['cc', '-O2', '-std=c11', '-fno-strict-aliasing', '-ffp-contract=off', str(code), '-o', str(binary)], check=True)
    return binary, (q2_entity_layout(), q2_layout())


def fixture(tables, q2=False):
    rng = random.Random(8603232)
    def words(layout, player=False):
        out = []
        for i, (_, width) in enumerate(layout):
            floating = (13 <= i <= 21 or 24 <= i <= 34) if player else 8 <= i <= 16
            if q2 and floating:
                if not player:
                    value = rng.randrange(-32000,32000)*.125
                elif 30 <= i <= 33:
                    value = rng.randrange(256)/255.
                elif i == 34:
                    value = rng.randrange(10,141)
                else:
                    value = rng.randrange(-240,241)*.125
                out.append(struct.unpack('<I', struct.pack('<f', value))[0])
            elif q2 and player and width < 0:
                out.append(rng.randrange(-32000,32000)&0xffffffff)
            elif q2 and not player and width == 0:
                out.append(rng.choice([0,128,255,256,32767,32768,65535,65536,1<<31]))
            elif width == 0 and not q2:
                value = rng.choice([0., -0., -4096., 4095., rng.randrange(-100, 100)*.125])
                out.append(struct.unpack('<I', struct.pack('<f', value))[0])
            else:
                out.append(rng.randrange(1 << min(abs(width), 31)))
        if player:
            out += [rng.randrange(-32000, 32000) & 0xffffffff for _ in range(32 if q2 else 48)]
            if not q2:
                out += [rng.randrange(1 << 31) for _ in range(16)]
        return out
    output = bytearray(struct.pack('<I', 512))
    for case in range(512):
        distance = [0, 1, 2, 28, 29, 31, 32][case % 7]
        prime = case % 9 != 0
        area = bytes(rng.randrange(256) for _ in range(case % 33))
        pool = range(1,513) if q2 else range(96)
        old_numbers = sorted(rng.sample(pool, case % 25))
        new_numbers = sorted(set(n for n in old_numbers if rng.randrange(4) != 0) | set(rng.sample(pool, case % 8)))
        old = {n: words(tables[0]) for n in old_numbers}
        new = {n: old[n].copy() if n in old else words(tables[0]) for n in new_numbers}
        for n in new_numbers:
            if n in old and rng.randrange(3) == 0:
                new[n][8 if q2 else 1] = struct.unpack('<I', struct.pack('<f', 0.5*case))[0]
        baselines = {n: words(tables[0]) for n in rng.sample(pool, 4)}
        p, q = words(tables[1], True), words(tables[1], True)
        output += struct.pack('<4B3H', distance, prime, 4 | (case & 1), len(area), len(old), len(new), len(baselines))
        output += struct.pack(f'<{len(p+q)}I', *(p+q)) + area
        for records in [baselines, old, new]:
            for number, record in records.items():
                output += struct.pack(f'<H{len(tables[0])}I', number, *record)
    return bytes(output)


def compile_qw_reference(qsrc, evidence):
    """Original QW packet merge bodies with private native bindings."""
    source = PREAMBLE + '\ntypedef float vec3_t[3];\n#define true 1\n#define false 0\n'
    source += '#define cl q2_player_client\n#define frame_t q2_frame_t\n#define client_frame_t q2_client_frame_t\n'
    source += q2_reference(qsrc)
    source += '#undef cl\n#undef frame_t\n#undef client_frame_t\n'
    source += qw_reference(qsrc)
    source += r'''
typedef qw_entity_state_t entity_state_t;
#define UPDATE_BACKUP 64
#define UPDATE_MASK 63
#define MAX_PACKET_ENTITIES 64
#define svc_packetentities 47
#define svc_deltapacketentities 48
#define MSG_WriteByte(m,v) q2_write(m,v,8)
#define MSG_WriteShort(m,v) q2_write(m,v,16)
#define MSG_ReadByte() q2_read(&net_message,8,0)
#define MSG_ReadShort() q2_read(&net_message,16,1)
typedef struct {int num_entities;entity_state_t entities[64];} packet_entities_t;
typedef struct {packet_entities_t entities;} client_frame_t;
typedef struct {client_frame_t frames[64];int delta_sequence;} client_t;
typedef struct {packet_entities_t packet_entities;int delta_sequence,invalid;} frame_t;
typedef struct {entity_state_t baseline;} edict_t;
static edict_t server_baselines[512];static entity_state_t cl_baselines[512];
#define EDICT_NUM(n) (&server_baselines[n])
static struct {int validsequence;frame_t frames[64];} cl;
static struct {struct {int incoming_sequence,outgoing_sequence;} netchan;} cls;
static int msg_badread;
static void Con_DPrintf(char *fmt,...) {(void)fmt;}
static void Con_Printf(char *fmt,...) {(void)fmt;}
static void Host_EndGame(char *fmt,...) {(void)fmt;abort();}
'''
    server = (qsrc/'quake/QW/server/sv_ents.c').read_text()
    client = (qsrc/'quake/QW/client/cl_ents.c').read_text()
    source += function(server, 'SV_EmitPacketEntities')
    source += function(client, 'FlushEntityPacket') + function(client, 'CL_ParsePacketEntities')
    source += r'''
static int entity_input(entity_state_t *e) {
 uint16_t n;uint32_t words[12];if(fread(&n,2,1,stdin)!=1||fread(words,4,12,stdin)!=12)return 0;
 memset(e,0,sizeof(*e));e->number=n;qw_put(e,words);return 1;
}
static void parse(msg_t *m,int sequence) {
 net_message=*m;net_message.readcount=net_message.bit=0;
 cls.netchan.incoming_sequence=sequence;cls.netchan.outgoing_sequence=sequence+1;
 int opcode=MSG_ReadByte();if(opcode!=47&&opcode!=48)abort();
 cl.frames[sequence&63].delta_sequence=opcode==48?1:-1;
 CL_ParsePacketEntities(opcode==48);if(MSG_ReadByte()!=6)abort();
}
int main(void) {
 uint32_t cases;if(fread(&cases,4,1,stdin)!=1)return 2;
 for(uint32_t k=0;k<cases;k++) {
  byte h[4];uint16_t counts[3];
  if(fread(h,1,4,stdin)!=4||fread(counts,2,3,stdin)!=3||h[3]||counts[0]>64||counts[1]>64)return 3;
  memset(&cl,0,sizeof(cl));memset(server_baselines,0,sizeof(server_baselines));memset(cl_baselines,0,sizeof(cl_baselines));
  client_t client={0};packet_entities_t to={0};
  for(int i=0;i<counts[2];i++) {entity_state_t e;if(!entity_input(&e))return 4;
   server_baselines[e.number].baseline=e;cl_baselines[e.number]=e;}
  packet_entities_t *old=&client.frames[1].entities;old->num_entities=counts[0];to.num_entities=counts[1];
  for(int i=0;i<counts[0];i++)if(!entity_input(&old->entities[i]))return 5;
  for(int i=0;i<counts[1];i++)if(!entity_input(&to.entities[i]))return 6;
  if(h[1]) {byte wire[8192]={0};msg_t m={.data=wire,.maxsize=sizeof(wire)};
   client.delta_sequence=-1;SV_EmitPacketEntities(&client,old,&m);MSG_WriteByte(&m,6);parse(&m,1);}
  /* The unified engine retains 32 snapshots, selecting a native full response
     when that actual requested frame is no longer resident. */
  client.delta_sequence=h[0]&&h[0]<32?1:-1;
  int sequence=1+(h[0]?h[0]:1);byte wire[8192]={0};msg_t m={.data=wire,.maxsize=sizeof(wire)};
  SV_EmitPacketEntities(&client,&to,&m);MSG_WriteByte(&m,6);parse(&m,sequence);
  uint32_t accepted=cl.validsequence==sequence;
  uint32_t header[4]={m.bit,m.cursize,accepted,net_message.bit};fwrite(header,4,4,stdout);fwrite(wire,1,m.cursize,stdout);
  if(accepted) {packet_entities_t *p=&cl.frames[sequence&63].packet_entities;
   uint32_t meta[5]={0,0,0,0,p->num_entities};fwrite(meta,4,5,stdout);
   for(int i=0;i<p->num_entities;i++) {uint32_t n=p->entities[i].number,words[12];qw_get(&p->entities[i],words);
    fwrite(&n,4,1,stdout);fwrite(words,4,12,stdout);}
  }
 }
 return 0;
}
'''
    code = evidence/'original-qw-packets.c'
    code.write_text(source)
    binary = evidence/'original-qw-packets'
    subprocess.run(['cc','-O2','-std=c11','-fno-strict-aliasing','-ffp-contract=off',str(code),'-o',str(binary)],check=True)
    return binary


def qw_fixture():
    rng = random.Random(86028)
    def words():
        values = [rng.randrange(256) for _ in range(5)]
        values += [struct.unpack('<I',struct.pack('<f',rng.randrange(-16000,16000)*.125))[0] for _ in range(3)]
        values += [struct.unpack('<I',struct.pack('<f',rng.randrange(-256,256)*1.25))[0] for _ in range(3)]
        return values + [rng.choice([0,64])]
    output = bytearray(struct.pack('<I',512))
    for case in range(512):
        distance = [0,1,2,28,29,31,32][case%7]
        pool = range(1,512)
        old_numbers = sorted(rng.sample(pool,case%40))
        new_numbers = sorted(set(n for n in old_numbers if rng.randrange(4)) | set(rng.sample(pool,case%9)))
        old = {n:words() for n in old_numbers}
        new = {n:old[n].copy() if n in old else words() for n in new_numbers}
        for n in new_numbers:
            if n in old and rng.randrange(3)==0:
                new[n][5] = struct.unpack('<I',struct.pack('<f',case*.125))[0]
        baselines = {n:words() for n in rng.sample(pool,4)}
        output += struct.pack('<4B3H',distance,1,0,0,len(old),len(new),len(baselines))
        for records in [baselines,old,new]:
            for number,record in records.items():
                output += struct.pack('<H12I',number,*record)
    return bytes(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, default=ROOT.parent/'qsrc')
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--rust', type=Path, required=True)
    parser.add_argument('--protocol', choices=['q3','q2','qw'], default='q3')
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    q2 = args.protocol == 'q2'
    if args.protocol == 'qw':
        binary = compile_qw_reference(args.qsrc, args.evidence)
        data = qw_fixture()
    else:
        binary, tables = (compile_q2_reference if q2 else compile_reference)(args.qsrc, args.evidence)
        data = fixture(tables, q2)
    (args.evidence/'fixtures.bin').write_bytes(data)
    native = subprocess.check_output([binary], input=data)
    rust = subprocess.check_output([args.rust] + (['--'+args.protocol] if args.protocol != 'q3' else []), input=data)
    (args.evidence/'original.bin').write_bytes(native)
    (args.evidence/'rust.bin').write_bytes(rust)
    result = {'protocol': args.protocol, 'cases': 512, 'bytes': len(native), 'exact': native == rust,
              'scope': 'original snapshot writer/parser bodies with private native bindings; no signon/host/gameplay'}
    if args.protocol == 'qw':
        result['scope'] = 'original QW packet-entity writer/parser bodies; no playerinfo/signon/host/gameplay'
        result['retained_snapshot_slots'] = 32
        result['native_qw_backup'] = 64
        result['retention_policy'] = 'original full-response selection when the unified 32-slot window loses the requested base'
    if native != rust:
        result['first_difference'] = next((i for i,(a,b) in enumerate(zip(native,rust)) if a != b), min(len(native),len(rust)))
    (args.evidence/'comparison.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result))
    if not result['exact']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
