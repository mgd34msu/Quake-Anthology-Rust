#!/usr/bin/env python3
"""THE-860 original Q3 snapshot stream oracle; extracted C stays developer-only."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess
from check_message import function
from check_state_delta import reference_source

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


def fixture(tables):
    rng = random.Random(8603232)
    def words(layout, player=False):
        out = []
        for _, width in layout:
            if width == 0:
                value = rng.choice([0., -0., -4096., 4095., rng.randrange(-100, 100)*.125])
                out.append(struct.unpack('<I', struct.pack('<f', value))[0])
            else:
                out.append(rng.randrange(1 << min(abs(width), 31)))
        if player:
            out += [rng.randrange(-32000, 32000) & 0xffffffff for _ in range(48)]
            out += [rng.randrange(1 << 31) for _ in range(16)]
        return out
    output = bytearray(struct.pack('<I', 512))
    for case in range(512):
        distance = [0, 1, 2, 28, 29, 31, 32][case % 7]
        prime = case % 9 != 0
        area = bytes(rng.randrange(256) for _ in range(case % 33))
        old_numbers = sorted(rng.sample(range(96), case % 25))
        new_numbers = sorted(set(n for n in old_numbers if rng.randrange(4) != 0) | set(rng.sample(range(96), case % 8)))
        old = {n: words(tables[0]) for n in old_numbers}
        new = {n: old[n].copy() if n in old else words(tables[0]) for n in new_numbers}
        for n in new_numbers:
            if n in old and rng.randrange(3) == 0:
                new[n][1] = struct.unpack('<I', struct.pack('<f', 0.5*case))[0]
        baselines = {n: words(tables[0]) for n in rng.sample(range(96), 4)}
        p, q = words(tables[1], True), words(tables[1], True)
        output += struct.pack('<4B3H', distance, prime, 4 | (case & 1), len(area), len(old), len(new), len(baselines))
        output += struct.pack('<224I', *(p+q)) + area
        for records in [baselines, old, new]:
            for number, record in records.items():
                output += struct.pack('<H51I', number, *record)
    return bytes(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, default=ROOT.parent/'qsrc')
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--rust', type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    binary, tables = compile_reference(args.qsrc, args.evidence)
    data = fixture(tables)
    (args.evidence/'fixtures.bin').write_bytes(data)
    native = subprocess.check_output([binary], input=data)
    rust = subprocess.check_output([args.rust], input=data)
    (args.evidence/'original.bin').write_bytes(native)
    (args.evidence/'rust.bin').write_bytes(rust)
    result = {'cases': 512, 'bytes': len(native), 'exact': native == rust,
              'scope': 'original snapshot writer/parser bodies with private native bindings; no signon/host/gameplay'}
    if native != rust:
        result['first_difference'] = next((i for i,(a,b) in enumerate(zip(native,rust)) if a != b), min(len(native),len(rust)))
    (args.evidence/'comparison.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result))
    if not result['exact']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
