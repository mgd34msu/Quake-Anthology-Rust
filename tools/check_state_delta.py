#!/usr/bin/env python3
"""THE-860 native entity/player deltas; unchanged original C is developer-only."""
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


def qw_layout():
    rust = (ROOT / 'crates/network/src/states.rs').read_text()
    block = re.search(r'pub const QW_ENTITY_LAYOUT.*?=\s*\[(.*?)\n\];', rust, re.S).group(1)
    return [(name, int(width)) for name, width in re.findall(r'\("([^"]+)",\s*(-?\d+)\)', block)]


def qw_reference(qsrc):
    protocol = (qsrc / 'quake/QW/client/protocol.h').read_text()
    common = (qsrc / 'quake/QW/client/common.c').read_text()
    end = protocol.index('} entity_state_t;') + len('} entity_state_t;')
    source = '\n#define entity_state_t qw_entity_state_t\n' + protocol[protocol.rfind('typedef struct',0,end):end] + '\n'
    source += '\n'.join(re.findall(r'^#define\s+U_[A-Z0-9_]+\s+.*$', protocol, re.M)) + '\n'
    source += r'''
static int bitcounts[16];
static void SV_Error(char *fmt,...) {(void)fmt;abort();}
static void Sys_Error(char *fmt,...) {(void)fmt;abort();}
#define MSG_WriteByte(m,v) q2_write(m,v,8)
#define MSG_WriteShort(m,v) q2_write(m,v,16)
#define MSG_ReadByte() q2_read(&net_message,8,0)
#define MSG_ReadChar() q2_read(&net_message,8,1)
#define MSG_ReadShort() q2_read(&net_message,16,1)
#define MSG_WriteCoord QW_WriteCoord
#define MSG_WriteAngle QW_WriteAngle
#define MSG_ReadCoord QW_ReadCoord
#define MSG_ReadAngle QW_ReadAngle
'''
    for name in ['MSG_WriteCoord','MSG_WriteAngle','MSG_ReadCoord','MSG_ReadAngle']:
        source += function(common,name)
    source += function((qsrc/'quake/QW/server/sv_ents.c').read_text(),'SV_WriteDelta')
    source += function((qsrc/'quake/QW/client/cl_ents.c').read_text(),'CL_ParseDelta')
    source += 'static void qw_put(qw_entity_state_t *p,uint32_t *words) {\n'
    for i,(name,_) in enumerate(qw_layout()):
        source += f' memcpy(&p->{name}, &words[{i}],4);\n' if 5 <= i <= 10 else f' p->{name}=words[{i}];\n'
    source += '}\nstatic void qw_get(qw_entity_state_t *p,uint32_t *words) {\n'
    for i,(name,_) in enumerate(qw_layout()):
        source += f' memcpy(&words[{i}], &p->{name},4);\n' if 5 <= i <= 10 else f' words[{i}]=p->{name};\n'
    source += r'''
}
static void qw_encode(msg_t *m,uint32_t *from,uint32_t *to,int number,int flags) {
 qw_entity_state_t a={.number=number},b={.number=number};qw_put(&a,from);qw_put(&b,to);
 // Native SV_EmitPacketEntities removal statement, sv_ents.c:307.
 if(flags&2)MSG_WriteShort(m,number|U_REMOVE);else SV_WriteDelta(&a,&b,m,flags&1);
}
static void qw_decode(msg_t *m,uint32_t *from,uint32_t *out,uint32_t *number,byte *removed) {
 qw_entity_state_t a={.number=*number},b={0};qw_put(&a,from);
 if(!m->cursize) {qw_get(&a,out);return;}
 net_message=*m;int header=MSG_ReadShort();*number=header&511;
 if(header&U_REMOVE) {*removed=1;return;}
 CL_ParseDelta(&a,&b,header);qw_get(&b,out);
}
#undef entity_state_t
#undef MSG_WriteByte
#undef MSG_WriteShort
#undef MSG_ReadByte
#undef MSG_ReadChar
#undef MSG_ReadShort
#undef MSG_WriteCoord
#undef MSG_WriteAngle
#undef MSG_ReadCoord
#undef MSG_ReadAngle
'''
    return source


def q2_entity_layout():
    rust = (ROOT / 'crates/network/src/states.rs').read_text()
    block = re.search(r'pub const Q2_ENTITY_LAYOUT.*?=\s*\[(.*?)\n\];', rust, re.S).group(1)
    return [(name, int(width)) for name, width in re.findall(r'\("([^"]+)",\s*(-?\d+)\)', block)]


def q2_entity_reference(qsrc):
    header = (qsrc / 'quake-2/game/q_shared.h').read_text()
    constants = (qsrc / 'quake-2/qcommon/qcommon.h').read_text()
    common = (qsrc / 'quake-2/qcommon/common.c').read_text()
    client = (qsrc / 'quake-2/client/cl_ents.c').read_text()
    end = header.index('} entity_state_t;') + len('} entity_state_t;')
    source = '\n' + header[header.rfind('typedef struct entity_state_s',0,end):end] + '\n'
    for line in re.findall(r'^#define\s+U_[A-Z0-9_]+\s+.*$', constants, re.M):
        name = line.split()[1]
        source += f'#undef {name}\n' + line + '\n'
    source += '\n'.join(re.findall(r'^#define\s+(?:MAX_EDICTS|RF_BEAM|VectorCopy).*$',header,re.M)) + '\n'
    source += r'''
#define bitcounts q2_entity_bitcounts
static int bitcounts[32];
#define MSG_WriteByte(m,v) q2_write(m,v,8)
#define MSG_WriteShort(m,v) q2_write(m,v,16)
#define MSG_WriteLong(m,v) q2_write(m,v,32)
#define MSG_ReadByte(m) q2_read(m,8,0)
#define MSG_ReadChar(m) q2_read(m,8,1)
#define MSG_ReadShort(m) q2_read(m,16,1)
#define MSG_ReadLong(m) q2_read(m,32,0)
#define MSG_WriteCoord Q2_WriteCoord
#define MSG_WriteAngle Q2_WriteAngle
#define MSG_ReadCoord Q2_ReadCoord
#define MSG_ReadAngle Q2_ReadAngle
#define MSG_ReadPos Q2_ReadPos
#define MSG_WriteDeltaEntity Q2_WriteDeltaEntity
#define CL_ParseDelta Q2_ParseDelta
'''
    for name in ['MSG_WriteCoord','MSG_WriteAngle','MSG_ReadCoord','MSG_ReadAngle','MSG_ReadPos','MSG_WriteDeltaEntity']:
        source += function(common,name)
    source += function(client,'CL_ParseEntityBits') + function(client,'CL_ParseDelta')
    source += 'static void q2_entity_put(entity_state_t *p,uint32_t *words) {\n'
    for i,(name,_) in enumerate(q2_entity_layout()):
        source += f' memcpy(&p->{name}, &words[{i}],4);\n' if 8 <= i <= 16 else f' p->{name}=words[{i}];\n'
    source += '}\nstatic void q2_entity_get(entity_state_t *p,uint32_t *words) {\n'
    for i,(name,_) in enumerate(q2_entity_layout()):
        source += f' memcpy(&words[{i}], &p->{name},4);\n' if 8 <= i <= 16 else f' words[{i}]=p->{name};\n'
    source += r'''
}
static void q2_entity_encode(msg_t *msg,uint32_t *from,uint32_t *to,int number,int flags) {
 entity_state_t a={.number=number},b={.number=number};q2_entity_put(&a,from);q2_entity_put(&b,to);
 if(!(flags&2)) {MSG_WriteDeltaEntity(&a,&b,msg,flags&1,flags&4);return;}
 int oldnum=number,bits;
'''
    emitter = (qsrc/'quake-2/server/sv_ents.c').read_text()
    start = emitter.index('bits = U_REMOVE;')
    source += emitter[start:emitter.index('oldindex++;',start)]
    source += r'''
}
static void q2_entity_decode(msg_t *m,uint32_t *from,uint32_t *out,uint32_t *number,byte *removed) {
 entity_state_t a={.number=*number},b={0};q2_entity_put(&a,from);
 if(!m->cursize) {q2_entity_get(&a,out);return;}
 net_message=*m;unsigned flags;*number=CL_ParseEntityBits(&flags);
 if(flags&U_REMOVE) {*removed=1;return;}
 CL_ParseDelta(&a,&b,*number,flags);q2_entity_get(&b,out);
}
#undef bitcounts
#undef MSG_WriteByte
#undef MSG_WriteShort
#undef MSG_WriteLong
#undef MSG_ReadByte
#undef MSG_ReadChar
#undef MSG_ReadShort
#undef MSG_ReadLong
#undef MSG_WriteCoord
#undef MSG_WriteAngle
#undef MSG_ReadCoord
#undef MSG_ReadAngle
#undef MSG_ReadPos
#undef MSG_WriteDeltaEntity
#undef CL_ParseDelta
'''
    return source


def nq_layout():
    rust = (ROOT / 'crates/network/src/states.rs').read_text()
    block = re.search(r'pub const NQ_ENTITY_LAYOUT.*?=\s*\[(.*?)\n\];', rust, re.S).group(1)
    return [(name, int(width)) for name, width in re.findall(r'\("([^"]+)",\s*(-?\d+)\)', block)]


def nq_player_layout():
    rust = (ROOT / 'crates/network/src/states.rs').read_text()
    block = re.search(r'pub const NQ_PLAYER_LAYOUT.*?=\s*\[(.*?)\n\];', rust, re.S).group(1)
    return [(name, int(width)) for name, width in re.findall(r'\("([^"]+)",\s*(-?\d+)\)', block)]


def nq_reference(qsrc):
    folder = qsrc / 'quake/WinQuake'
    protocol = (folder / 'protocol.h').read_text()
    common = (folder / 'common.c').read_text()
    header = (folder / 'quakedef.h').read_text()
    progs = (folder / 'progdefs.q1').read_text()
    source = r'''
#define entity_state_t nq_baseline_t
#define entvars_t nq_entvars_t
#define edict_t nq_edict_t
#define entity_t nq_entity_t
#define model_t nq_model_t
#define sv nq_sv
#define cl nq_cl
#define cls nq_cls
#define vid nq_vid
#define bitcounts nq_bitcounts
typedef int string_t,func_t;
'''
    end = header.index('} entity_state_t;') + len('} entity_state_t;')
    source += header[header.rfind('typedef struct',0,end):end] + '\n'
    end = progs.index('} entvars_t;') + len('} entvars_t;')
    source += progs[progs.rfind('typedef struct',0,end):end] + '\n'
    for line in re.findall(r'^#define\s+U_[A-Z0-9_]+\s+.*$', protocol, re.M):
        source += '#undef ' + line.split()[1] + '\n' + line + '\n'
    source += '\n'.join(re.findall(r'^#define\s+(?:SU_[A-Z0-9_]+|DEFAULT_VIEWHEIGHT|svc_clientdata|svc_damage|svc_setangle)\s+.*$', protocol, re.M)) + '\n'
    source += '\n'.join(re.findall(r'^#define\s+STAT_[A-Z0-9_]+\s+.*$', header, re.M)) + '\n'
    source += r'''
#define true 1
#define false 0
#define MAX_MODELS 256
#define SIGNONS 4
#define ST_RAND 1
#define MOVETYPE_STEP 4
typedef struct {entvars_t v;entity_state_t baseline;int num_leafs;short leafnums[16];} edict_t;
typedef struct {int synctype;} model_t;
typedef struct {entity_state_t baseline;model_t *model;double msgtime;
 vec3_t msg_origins[2],msg_angles[2],origin,angles;int frame,skinnum,effects,forcelink;
 float syncbase;byte *colormap;} entity_t;
static struct {edict_t *edicts;int num_edicts;} sv;
static struct {int signon;} cls;
static struct {double mtime[2];model_t *model_precache[256];int maxclients;
 float viewheight,idealpitch,punchangle[3],mvelocity[2][3];
 int items,onground,inwater,stats[32];double time,item_gettime[32];
 struct {byte translations[1];} scores[16];} cl;
static struct {byte *colormap;} vid;
static edict_t nq_edicts[32768];static entity_t nq_decoded;static model_t nq_models[256];
static byte nq_pvs[1]={255},nq_global_colormap[1];static char pr_strings[2]={0,1};
static int pr_edict_size=sizeof(edict_t);
static int bitcounts[16];
#define NEXT_EDICT(e) ((edict_t *)((byte *)(e)+pr_edict_size))
#define VectorAdd(a,b,c) {c[0]=a[0]+b[0];c[1]=a[1]+b[1];c[2]=a[2]+b[2];}
static byte *SV_FatPVS(vec3_t p) {(void)p;return nq_pvs;}
static int nq_parsed_number;
static entity_t *CL_EntityNum(int n) {if(n<0||n>=32768)abort();nq_parsed_number=n;return &nq_decoded;}
static void CL_SignonReply(void) {}
static void Host_Error(char *fmt,...) {(void)fmt;abort();}
static void Con_Printf(char *fmt,...) {(void)fmt;}
#define MSG_WriteByte(m,v) q2_write(m,v,8)
#define MSG_WriteChar(m,v) q2_write(m,v,8)
#define MSG_WriteShort(m,v) q2_write(m,v,16)
#define MSG_WriteLong(m,v) q2_write(m,v,32)
#define MSG_ReadByte() q2_read(&net_message,8,0)
#define MSG_ReadChar() q2_read(&net_message,8,1)
#define MSG_ReadShort() q2_read(&net_message,16,1)
#define MSG_ReadLong() q2_read(&net_message,32,0)
#define MSG_WriteCoord NQ_WriteCoord
#define MSG_WriteAngle NQ_WriteAngle
#define MSG_ReadCoord NQ_ReadCoord
#define MSG_ReadAngle NQ_ReadAngle
'''
    for name in ['MSG_WriteCoord','MSG_WriteAngle','MSG_ReadCoord','MSG_ReadAngle']:
        source += function(common,name)
    source += function((folder/'sv_main.c').read_text(),'SV_WriteEntitiesToClient')
    source += function((folder/'cl_parse.c').read_text(),'CL_ParseUpdate')
    source += r'''
typedef union {float _float;} eval_t;
static struct {float serverflags;} nq_globals;
#define pr_global_struct (&nq_globals)
static int standard_quake,nq_weaponindex;
#define FL_ONGROUND 512
#define PROG_TO_EDICT(n) (nq_edicts)
static eval_t *GetEdictFieldValue(edict_t *p,char *name) {(void)p;(void)name;return NULL;}
static int SV_ModelIndex(char *name) {(void)name;return nq_weaponindex;}
static void SV_SetIdealPitch(void) {}
static void Sbar_Changed(void) {}
'''
    source += function((folder/'sv_main.c').read_text(),'SV_WriteClientdataToMessage')
    source += function((folder/'cl_parse.c').read_text(),'CL_ParseClientdata')
    source += 'static void nq_baseline(entity_state_t *p,uint32_t *words) {\n'
    for i,(name,_) in enumerate(nq_layout()[:11]):
        source += f' memcpy(&p->{name}, &words[{i}],4);\n'
    source += '}\nstatic void nq_encode(msg_t *m,uint32_t *from,uint32_t *to,int number,int flags) {\n'
    source += ' edict_t *p=&nq_edicts[number];memset(p,0,sizeof(*p));nq_baseline(&p->baseline,from);\n'
    for i,(name,_) in enumerate(nq_layout()[:11]):
        source += f' memcpy(&p->v.{name}, &to[{i}],4);\n'
    source += r'''
 p->v.model=1;p->v.movetype=(flags&4)?MOVETYPE_STEP:0;
 sv.edicts=nq_edicts;sv.num_edicts=number+1;SV_WriteEntitiesToClient(p,m);
 // Clear the selected entity so future record helpers see no extra visible entity.
 p->v.modelindex=0;p->v.model=0;
}
static void nq_decode(msg_t *m,uint32_t *from,uint32_t *out,uint32_t *number) {
 memset(&nq_decoded,0,sizeof(nq_decoded));nq_baseline(&nq_decoded.baseline,from);
 for(int i=0;i<256;i++)cl.model_precache[i]=&nq_models[i];
 cl.maxclients=16;vid.colormap=nq_global_colormap;net_message=*m;
 int bits=MSG_ReadByte();CL_ParseUpdate(bits&127);*number=nq_parsed_number;
 out[0]=(int)(nq_decoded.model-nq_models);
 out[1]=nq_decoded.frame;out[3]=nq_decoded.skinnum;out[4]=nq_decoded.effects;
 if(nq_decoded.colormap!=vid.colormap)
  for(int i=0;i<cl.maxclients;i++)if(nq_decoded.colormap==cl.scores[i].translations)out[2]=i+1;
 for(int i=0;i<3;i++) {memcpy(out+5+2*i,nq_decoded.msg_origins[0]+i,4);
  memcpy(out+6+2*i,nq_decoded.msg_angles[0]+i,4);}
 out[11]=!!(bits&U_NOLERP);
}
static void nq_player_encode(msg_t *m,uint32_t *to,int flags) {
 edict_t p={0};standard_quake=!(flags&4);nq_weaponindex=to[11];
 memcpy(&p.v.view_ofs[2],to,4);memcpy(&p.v.idealpitch,to+1,4);
 for(int i=0;i<3;i++) {memcpy(p.v.punchangle+i,to+2+2*i,4);memcpy(p.v.velocity+i,to+3+2*i,4);}
 // This record fixture supplies already-reduced items; bind exact QC low bits
 // and native serverflags. items2/mod inventory conversion remains separate.
 p.v.items=(float)(to[8]&0x0fffffff);nq_globals.serverflags=(float)(to[8]>>28);
 memcpy(&p.v.weaponframe,to+9,4);memcpy(&p.v.armorvalue,to+10,4);memcpy(&p.v.health,to+12,4);
 memcpy(&p.v.currentammo,to+13,4);memcpy(&p.v.ammo_shells,to+14,4);memcpy(&p.v.ammo_nails,to+15,4);
 memcpy(&p.v.ammo_rockets,to+16,4);memcpy(&p.v.ammo_cells,to+17,4);
 p.v.weapon=standard_quake?(float)to[18]:(float)(int32_t)(1u<<to[18]);
 p.v.flags=to[19]?FL_ONGROUND:0;p.v.waterlevel=to[20]?3:0;
 SV_WriteClientdataToMessage(&p,m);
}
static void nq_player_decode(msg_t *m,uint32_t *out) {
 net_message=*m;if(MSG_ReadByte()!=svc_clientdata)abort();CL_ParseClientdata(MSG_ReadShort());
 memcpy(out,&cl.viewheight,4);memcpy(out+1,&cl.idealpitch,4);
 for(int i=0;i<3;i++) {memcpy(out+2+2*i,cl.punchangle+i,4);memcpy(out+3+2*i,cl.mvelocity[0]+i,4);}
 out[8]=cl.items;out[9]=cl.stats[STAT_WEAPONFRAME];out[10]=cl.stats[STAT_ARMOR];out[11]=cl.stats[STAT_WEAPON];
 out[12]=cl.stats[STAT_HEALTH];out[13]=cl.stats[STAT_AMMO];
 for(int i=0;i<4;i++)out[14+i]=cl.stats[STAT_SHELLS+i];
 out[18]=cl.stats[STAT_ACTIVEWEAPON];out[19]=cl.onground;out[20]=cl.inwater;
}
#undef entity_state_t
#undef entvars_t
#undef edict_t
#undef entity_t
#undef model_t
#undef sv
#undef cl
#undef cls
#undef vid
#undef bitcounts
#undef pr_global_struct
#undef MSG_WriteByte
#undef MSG_WriteChar
#undef MSG_WriteShort
#undef MSG_WriteLong
#undef MSG_ReadByte
#undef MSG_ReadChar
#undef MSG_ReadShort
#undef MSG_ReadLong
#undef MSG_WriteCoord
#undef MSG_WriteAngle
#undef MSG_ReadCoord
#undef MSG_ReadAngle
'''
    return source


def qw_player_layout():
    rust = (ROOT / 'crates/network/src/states.rs').read_text()
    block = re.search(r'pub const QW_PLAYER_LAYOUT.*?=\s*\[(.*?)\n\];', rust, re.S).group(1)
    return [(name, int(width)) for name, width in re.findall(r'\("([^"]+)",\s*(-?\d+)\)', block)]


def qw_player_reference(qsrc):
    folder = qsrc / 'quake/QW'
    protocol = (folder/'client/protocol.h').read_text()
    common = (folder/'client/common.c').read_text()
    header = (folder/'client/client.h').read_text()
    source = r'''
#define usercmd_t qw_player_command_t
#define player_state_t qw_player_state_t
#define player_info_t qw_player_info_t
#define cl qw_player_client
'''
    end = protocol.index('} usercmd_t;') + len('} usercmd_t;')
    source += protocol[protocol.rfind('typedef struct',0,end):end] + '\n'
    end = header.index('} player_state_t;') + len('} player_state_t;')
    source += header[header.rfind('typedef struct',0,end):end] + '\n'
    source += '\n#undef svc_playerinfo\n'
    source += '\n'.join(re.findall(r'^#define\s+(?:PF_[A-Z0-9_]+|CM_[A-Z0-9_]+|MAX_CLIENTS|svc_playerinfo)\s+.*$',protocol,re.M)) + '\n'
    source += r'''
typedef struct {int unused;} player_info_t;
static struct {player_info_t players[MAX_CLIENTS];
 struct {player_state_t playerstate[MAX_CLIENTS];} frames[1];int parsecount;} cl;
static int parsecountmod,cl_playerindex;static double parsecounttime=1.0;
static usercmd_t nullcmd;
#define MSG_WriteByte(m,v) q2_write(m,v,8)
#define MSG_WriteShort(m,v) q2_write(m,v,16)
#define MSG_ReadByte() q2_read(&net_message,8,0)
#define MSG_ReadShort() q2_read(&net_message,16,1)
#define MSG_WriteCoord QW_WriteCoord
#define MSG_ReadCoord QW_ReadCoord
#define MSG_WriteAngle16 QW_Player_WriteAngle16
#define MSG_ReadAngle16 QW_Player_ReadAngle16
#define MSG_WriteDeltaUsercmd QW_Player_WriteCommand
#define MSG_ReadDeltaUsercmd QW_Player_ReadCommand
'''
    for name in ['MSG_WriteAngle16','MSG_ReadAngle16','MSG_WriteDeltaUsercmd','MSG_ReadDeltaUsercmd']:
        source += function(common,name)
    source += function((folder/'client/cl_ents.c').read_text(),'CL_ParsePlayerinfo')
    original = function((folder/'server/sv_ents.c').read_text(),'SV_WritePlayersToClient')
    start = original.index('\t\tMSG_WriteByte (msg, svc_playerinfo);')
    end = original.rindex('\n\t}\n}')
    block = original[start:end]
    source += r'''
static usercmd_t qw_player_cmd(uint32_t *v) {
 usercmd_t c={0};memcpy(c.angles,v,12);c.forwardmove=v[3];c.sidemove=v[4];c.upmove=v[5];
 c.buttons=v[6];c.impulse=v[7];c.msec=v[8];return c;
}
static void qw_player_encode(msg_t *msg,uint32_t *to,int j) {
 typedef struct {vec3_t origin,velocity,angles;float frame,health,modelindex,skin,effects,weaponframe;} vars_t;
 struct {vars_t v;} object={0},*ent=&object;
 struct {double localtime;usercmd_t lastcmd;} binding={0},*cl=&binding;
 struct {double time;} sv={.time=1};
 int pflags=to[12],i,msec;usercmd_t cmd;
 memcpy(ent->v.origin,to,12);memcpy(&ent->v.frame,to+3,4);memcpy(ent->v.velocity,to+5,12);
 memcpy(&ent->v.modelindex,to+8,4);memcpy(&ent->v.skin,to+9,4);memcpy(&ent->v.effects,to+10,4);
 memcpy(&ent->v.weaponframe,to+11,4);memcpy(ent->v.angles+1,to+13,4);
 ent->v.health=(pflags&PF_DEAD)?-1:100;cl->lastcmd=qw_player_cmd(to+14);
 // Bind an interval inside the supplied native integer age, avoiding a second
 // copy of the native timestamp-to-integer expression under comparison.
 cl->localtime=1-((int32_t)to[4]+((int32_t)to[4]<0?-0.25:0.25))/1000.0;
'''
    source += block + '\n}\n'
    source += r'''
static void qw_player_decode(msg_t *m,uint32_t *from,uint32_t *out,uint32_t *number) {
 net_message=*m;if(MSG_ReadByte()!=svc_playerinfo)abort();*number=m->data[1];
 player_state_t *p=&cl.frames[0].playerstate[*number];memset(p,0,sizeof(*p));
 p->command=qw_player_cmd(from+14);cl_playerindex=from[8];CL_ParsePlayerinfo();
 memcpy(out,p->origin,12);out[3]=p->frame;
 if(p->flags&PF_MSEC)out[4]=m->data[11];
 memcpy(out+5,p->velocity,12);out[8]=p->modelindex;out[9]=p->skinnum;
 out[10]=p->effects;out[11]=p->weaponframe;out[12]=p->flags;
 memcpy(out+14,p->command.angles,12);out[17]=(int)p->command.forwardmove;
 out[18]=(int)p->command.sidemove;out[19]=(int)p->command.upmove;
 out[20]=p->command.buttons;out[21]=p->command.impulse;out[22]=p->command.msec;
}
#undef usercmd_t
#undef player_state_t
#undef player_info_t
#undef cl
#undef MSG_WriteByte
#undef MSG_WriteShort
#undef MSG_ReadByte
#undef MSG_ReadShort
#undef MSG_WriteCoord
#undef MSG_ReadCoord
#undef MSG_WriteAngle16
#undef MSG_ReadAngle16
#undef MSG_WriteDeltaUsercmd
#undef MSG_ReadDeltaUsercmd
'''
    return source


def rr_stats_reference(qsrc):
    original = (qsrc / 'q2repro/src/common/msg.c').read_text()
    source = r'''
typedef struct {
 int pm_type;float origin[3],velocity[3];uint16_t pm_time,pm_flags;
 int16_t gravity;float delta_angles[3];int8_t viewheight;
} rr_pmove_t;
typedef struct {
 rr_pmove_t pmove;int16_t viewangles[3],viewoffset[3],kick_angles[3],gunangles[3],gunoffset[3];
 uint16_t gunindex;uint8_t gunframe,screen_blend[4],damage_blend[4],fov,rdflags;
 int16_t stats[64];int8_t gunrate;
} rr_packed_t;
typedef struct {
 rr_pmove_t pmove;float viewangles[3],viewoffset[3],kick_angles[3],gunangles[3],gunoffset[3];
 int gunindex,gunskin,gunframe,gunrate;float screen_blend[4],damage_blend[4],fov;
 uint8_t rdflags;int16_t stats[64];
} rr_player_t;
static void rr_stat_write64(uint64_t v) {
 q2_write(&net_message,(uint32_t)v,32);q2_write(&net_message,(uint32_t)(v>>32),32);
}
static uint64_t rr_stat_read64(void) {
 uint64_t low=(uint32_t)q2_read(&net_message,32,0);
 return low|((uint64_t)(uint32_t)q2_read(&net_message,32,0)<<32);
}
#define player_packed_t rr_packed_t
#define player_state_t rr_player_t
#define msgPsFlags_t uint32_t
#define MSG_PS_RERELEASE (1u<<11)
#define MSG_PS_EXTENSIONS_2 (1u<<7)
#define MSG_PS_EXTENSIONS (1u<<6)
#define MAX_STATS_NEW 64
#define MAX_STATS_OLD 32
#define BIT_ULL(i) (UINT64_C(1)<<(i))
#define MSG_WriteStats RR_WriteStats
#define MSG_ReadStats RR_ReadStats
#define MSG_WriteVarInt64 RR_WriteVarInt64
#define MSG_ReadVarInt64 RR_ReadVarInt64
#define MSG_WriteLong64 rr_stat_write64
#define MSG_ReadLong64 rr_stat_read64
#define MSG_WriteLong(v) q2_write(&net_message,v,32)
#define MSG_WriteShort(v) q2_write(&net_message,v,16)
#define MSG_WriteByte(v) q2_write(&net_message,v,8)
#define MSG_ReadLong() q2_read(&net_message,32,0)
#define MSG_ReadShort() q2_read(&net_message,16,1)
#define MSG_ReadByte() q2_read(&net_message,8,0)
'''
    source += ''.join(function(original, name) for name in ['MSG_WriteVarInt64','MSG_ReadVarInt64','MSG_WriteStats','MSG_ReadStats'])
    source += r'''
static void rr_stats_encode(msg_t *m,uint32_t *from,uint32_t *to) {
 rr_packed_t a={0},b={0};uint64_t mask=0;
 for(int i=0;i<64;i++){a.stats[i]=from[i];b.stats[i]=to[i];if(a.stats[i]!=b.stats[i])mask|=BIT_ULL(i);}
 net_message=*m;RR_WriteStats(&b,mask,MSG_PS_RERELEASE);*m=net_message;
}
static void rr_stats_decode(msg_t *m,uint32_t *from,uint32_t *out) {
 rr_player_t a={0};for(int i=0;i<64;i++)a.stats[i]=from[i];
 net_message=*m;RR_ReadStats(&a,MSG_PS_RERELEASE);*m=net_message;
 for(int i=0;i<64;i++)out[i]=(int)a.stats[i];
}
'''
    protocol = (qsrc / 'q2repro/inc/common/protocol.h').read_text()
    source += '\n#define BIT(i) (1u<<(i))\n'
    definitions = re.findall(r'^#define\s+((?:PS_|EPS_)[A-Z_0-9]+)\s+(.*)$',protocol,re.M)
    source += ''.join(f'\n#undef {name}\n#define {name} {value}\n' for name,value in definitions)
    source += r'''
#define MSG_PS_IGNORE_PREDICTION (1u<<0)
#define MSG_PS_IGNORE_DELTAANGLES (1u<<1)
#define MSG_PS_IGNORE_VIEWANGLES (1u<<2)
#define MSG_PS_IGNORE_BLEND (1u<<3)
#define MSG_PS_IGNORE_GUNINDEX (1u<<4)
#define MSG_PS_IGNORE_GUNFRAMES (1u<<5)
#define GUNINDEX_BITS 13
#define GUNINDEX_MASK ((1u<<13)-1)
#define Q_assert assert
#define VectorCompare(a,b) ((a)[0]==(b)[0]&&(a)[1]==(b)[1]&&(a)[2]==(b)[2])
#define Vector4Compare(a,b) (VectorCompare(a,b)&&(a)[3]==(b)[3])
#undef VectorCopy
#define VectorCopy(a,b) memcpy(b,a,12)
#define Vector4Copy(a,b) memcpy(b,a,16)
#define pmtype_to_game3(v) (v)
#define pmtype_from_game3(v) (v)
#define pmflags_to_game3(v,x) (v)
#define pmflags_from_game3(v,x) (v)
#define MSG_WriteData(v,n) do {memcpy(net_message.data+net_message.cursize,v,n);net_message.cursize+=n;net_message.bit=net_message.cursize*8;} while(0)
#define MSG_WriteChar(v) q2_write(&net_message,v,8)
#define MSG_ReadChar() q2_read(&net_message,8,1)
#define MSG_ReadWord() q2_read(&net_message,16,0)
#define MSG_WriteFloat RR_WriteFloat
#define MSG_ReadFloat RR_ReadFloat
#define MSG_WriteCoord RR_WriteCoord
#define MSG_ReadCoordP RR_ReadCoordP
#define MSG_WriteAngle16 RR_WriteAngle16
#define MSG_ReadAngle16 RR_ReadAngle16
#define MSG_ReadDeltaCoord(v) abort()
#define SHORT2COORD(v) ((v)*(1.0f/8))
#define MSG_WriteDeltaBlend RR_WriteDeltaBlend
#define MSG_ReadBlend RR_ReadBlend
#define MSG_WriteDeltaPlayerstate_Enhanced RR_WritePlayer
#define MSG_ParseDeltaPlayerstate_Enhanced RR_ReadPlayer
#define MSG_CalcStatBits RR_CalcStatBits
#define MSG_ReadFog(v) abort()
typedef int player_fogchange_t;
static const rr_packed_t nullPlayerState;
static void RR_WriteFloat(float v){uint32_t bits;memcpy(&bits,&v,4);q2_write(&net_message,bits,32);}
static float RR_ReadFloat(void){uint32_t bits=q2_read(&net_message,32,0);float v;memcpy(&v,&bits,4);return v;}
'''
    source += ''.join(function(original,n) for n in ['MSG_WriteCoord','MSG_ReadCoordP','MSG_WriteAngle16','MSG_ReadAngle16','MSG_CalcStatBits','MSG_WriteDeltaBlend','MSG_ReadBlend','MSG_WriteDeltaPlayerstate_Enhanced','MSG_ParseDeltaPlayerstate_Enhanced'])
    source += 'static void rr_player_put(rr_packed_t *p,uint32_t *w) {\n'
    for i,(name,_) in enumerate(q2_layout()):
        name=name.replace('blend[','screen_blend[')
        source += f' memcpy(&p->{name},w+{i},4);\n' if 1<=i<=6 or 10<=i<=12 else f' p->{name}=w[{i}];\n'
    source += ' for(int i=0;i<4;i++)p->damage_blend[i]=w[36+i];p->gunrate=w[40];p->pmove.viewheight=w[41];for(int i=0;i<64;i++)p->stats[i]=w[43+i];\n}\n'
    # Only the cold fixture binds packed words to the expanded native parser
    # ABI. It does not implement a player delta reader or writer.
    source += 'static void rr_player_expanded(rr_player_t *p,uint32_t *w) {\n'
    for i,(name,_) in enumerate(q2_layout()):
        name=name.replace('blend[','screen_blend[')
        if 1<=i<=6 or 10<=i<=12: expr=None
        elif 13<=i<=15: expr=f'(int32_t)w[{i}]/16.f'
        elif 16<=i<=18: expr=f'(int16_t)w[{i}]*(360.0f/65536)'
        elif 19<=i<=21: expr=f'(int32_t)w[{i}]/1024.f'
        elif 24<=i<=26: expr=f'(int32_t)w[{i}]/512.f'
        elif 27<=i<=29: expr=f'(int32_t)w[{i}]/4096.f'
        elif 30<=i<=33: expr=f'w[{i}]/255.f'
        elif i==22: expr=f'w[{i}]&GUNINDEX_MASK'
        else: expr=f'w[{i}]'
        source += f' memcpy(&p->{name},w+{i},4);\n' if expr is None else f' p->{name}={expr};\n'
    source += ' p->gunskin=w[22]>>GUNINDEX_BITS;for(int i=0;i<4;i++)p->damage_blend[i]=w[36+i]/255.f;p->gunrate=w[40];p->pmove.viewheight=w[41];for(int i=0;i<64;i++)p->stats[i]=w[43+i];\n}\n'
    source += 'static void rr_player_get(rr_player_t *p,uint32_t *w) {\n'
    for i,(name,_) in enumerate(q2_layout()):
        name=name.replace('blend[','screen_blend[')
        if 1<=i<=6 or 10<=i<=12: expr=None
        elif 13<=i<=15: expr=f'(int)(p->{name}*16)'
        elif 16<=i<=18: expr=f'(int16_t)ANGLE2SHORT(p->{name})'
        elif 19<=i<=21: expr=f'(int)(p->{name}*1024)'
        elif 24<=i<=26: expr=f'(int)(p->{name}*512)'
        elif 27<=i<=29: expr=f'(int)(p->{name}*4096)'
        elif 30<=i<=33: expr=f'(int)(p->{name}*255+0.5f)'
        elif i==22: expr='p->gunindex|(p->gunskin<<GUNINDEX_BITS)'
        else: expr=f'p->{name}'
        source += f' memcpy(w+{i},&p->{name},4);\n' if expr is None else f' w[{i}]={expr};\n'
    source += ' for(int i=0;i<4;i++)w[36+i]=(int)(p->damage_blend[i]*255+0.5f);w[40]=p->gunrate;w[41]=(int)p->pmove.viewheight;for(int i=0;i<64;i++)w[43+i]=(int)p->stats[i];\n}\n'
    q2proto = (qsrc / 'q2repro/q2proto/src/q2proto_proto_q2repro.c').read_text()
    tails = [
        re.search(r'if \(\*extraflags & EPS_CLIENTNUM\)\s*WRITE_CHECKED\(server_write, io_arg, i16, playerstate->clientnum\);',q2proto).group(),
        re.search(r'if \(delta_bits_check\(extraflags, EPS_CLIENTNUM, &playerstate->delta_bits, Q2P_PSD_CLIENTNUM\)\)\s*READ_CHECKED\(client_read, io_arg, playerstate->clientnum, i16\);',q2proto).group(),
    ]
    source += r'''
typedef struct {int16_t clientnum;uint32_t delta_bits;} rr_clientnum_t;
#define WRITE_CHECKED(owner,io,width,value) q2_write((msg_t*)io,value,16)
#define READ_CHECKED(owner,io,value,width) ((value)=q2_read((msg_t*)io,16,1))
#define Q2P_PSD_CLIENTNUM 1
#define delta_bits_check(flags,bit,out,field) (((flags)&(bit))!=0)
static void rr_write_clientnum(msg_t *m,uint8_t flags,int16_t value) {
 uintptr_t io_arg=(uintptr_t)m;uint8_t *extraflags=&flags;
 rr_clientnum_t record={.clientnum=value},*playerstate=&record;
'''
    source += tails[0] + '\n}\n'
    source += 'static int16_t rr_read_clientnum(msg_t *m,uint8_t extraflags,int16_t value) {\n uintptr_t io_arg=(uintptr_t)m;rr_clientnum_t record={.clientnum=value},*playerstate=&record;\n'
    source += tails[1] + '\nreturn record.clientnum;\n}\n'
    source += '\n#undef WRITE_CHECKED\n#undef READ_CHECKED\n#undef Q2P_PSD_CLIENTNUM\n#undef delta_bits_check\n'
    source += r'''
static void rr_player_encode(msg_t *m,uint32_t *from,uint32_t *to) {
 rr_packed_t a={0},b={0};rr_player_put(&a,from);rr_player_put(&b,to);
 /* Extra flags are a cold fixture envelope, not part of the player body. */
 q2_write(m,0,8);net_message=*m;int extra=RR_WritePlayer(&a,&b,MSG_PS_RERELEASE|MSG_PS_EXTENSIONS);
 /* q2proto's EPS_CLIENTNUM tail follows gunrate and viewheight. */
 if(from[42]!=to[42])extra|=EPS_CLIENTNUM;
 rr_write_clientnum(&net_message,extra,to[42]);
 *m=net_message;m->data[0]=extra;
}
static void rr_player_decode(msg_t *m,uint32_t *from,uint32_t *out) {
 rr_player_t a={0},b={0};rr_player_expanded(&a,from);net_message=*m;
 int extra=MSG_ReadByte(),flags=MSG_ReadWord();
 RR_ReadPlayer(&a,&b,NULL,flags,extra,MSG_PS_RERELEASE|MSG_PS_EXTENSIONS);rr_player_get(&b,out);
 out[42]=(int)rr_read_clientnum(&net_message,extra,from[42]);*m=net_message;
}
'''
    extra_names=['BIT','MSG_PS_EXTENSIONS','MSG_PS_IGNORE_PREDICTION','MSG_PS_IGNORE_DELTAANGLES','MSG_PS_IGNORE_VIEWANGLES','MSG_PS_IGNORE_BLEND','MSG_PS_IGNORE_GUNINDEX','MSG_PS_IGNORE_GUNFRAMES','GUNINDEX_BITS','GUNINDEX_MASK','Q_assert','VectorCompare','Vector4Compare','VectorCopy','Vector4Copy','pmtype_to_game3','pmtype_from_game3','pmflags_to_game3','pmflags_from_game3','MSG_WriteData','MSG_WriteChar','MSG_ReadChar','MSG_ReadWord','MSG_WriteFloat','MSG_ReadFloat','MSG_WriteCoord','MSG_ReadCoordP','MSG_WriteAngle16','MSG_ReadAngle16','MSG_ReadDeltaCoord','SHORT2COORD','MSG_WriteDeltaBlend','MSG_ReadBlend','MSG_WriteDeltaPlayerstate_Enhanced','MSG_ParseDeltaPlayerstate_Enhanced','MSG_CalcStatBits','MSG_ReadFog']
    extra_names += [name for name,_ in definitions]
    source += ''.join(f'\n#undef {name}\n' for name in extra_names)
    names=['player_packed_t','player_state_t','msgPsFlags_t','MSG_PS_RERELEASE','MSG_PS_EXTENSIONS_2','MAX_STATS_NEW','MAX_STATS_OLD','BIT_ULL','MSG_WriteStats','MSG_ReadStats','MSG_WriteVarInt64','MSG_ReadVarInt64','MSG_WriteLong64','MSG_ReadLong64','MSG_WriteLong','MSG_WriteShort','MSG_WriteByte','MSG_ReadLong','MSG_ReadShort','MSG_ReadByte']
    return source + ''.join(f'\n#undef {name}\n' for name in names)


def kex_stats_reference(qsrc):
    original = (qsrc / 'q2repro/q2proto/src/q2proto_proto_kex.c').read_text()
    writer = re.search(r'uint32_t statbits1 = playerstate->statbits & 0xffffffff;.*?WRITE_CHECKED\(server_write, io_arg, i16, playerstate->stats\[i \+ 32\]\);',original,re.S).group()
    reader = re.search(r'uint32_t statbits1, statbits2;.*?playerstate->statbits = statbits1 \| \(\(uint64_t\)statbits2\) << 32;',original,re.S).group()
    source = r'''
typedef struct {uint64_t statbits;int16_t stats[64];} kex_stats_t;
#define BIT(i) (1u<<(i))
#define KEX_BITS_u32 32
#define KEX_BITS_i16 16
#define KEX_SIGN_u32 0
#define KEX_SIGN_i16 1
#define WRITE_CHECKED(owner,io,width,value) q2_write((msg_t*)io,value,KEX_BITS_##width)
#define READ_CHECKED(owner,io,value,width) ((value)=q2_read((msg_t*)io,KEX_BITS_##width,KEX_SIGN_##width))
static void kex_stats_encode(msg_t *m,uint32_t *from,uint32_t *to) {
 kex_stats_t record={0},*playerstate=&record;uintptr_t io_arg=(uintptr_t)m;
 for(int i=0;i<64;i++){record.stats[i]=to[i];if((int16_t)from[i]!=record.stats[i])record.statbits|=UINT64_C(1)<<i;}
'''
    source += writer + '\n}\n'
    source += 'static void kex_stats_decode(msg_t *m,uint32_t *from,uint32_t *out) {\n kex_stats_t record={0},*playerstate=&record;uintptr_t io_arg=(uintptr_t)m;\n for(int i=0;i<64;i++)record.stats[i]=from[i];\n'
    source += reader + '\nfor(int i=0;i<64;i++)out[i]=(int)record.stats[i];\n}\n'
    names=['BIT','KEX_BITS_u32','KEX_BITS_i16','KEX_SIGN_u32','KEX_SIGN_i16','WRITE_CHECKED','READ_CHECKED']
    return source + ''.join(f'\n#undef {name}\n' for name in names)


def kex_player_reference(qsrc):
    """Use the real native structs/helpers in an isolated cold translation unit."""
    original = (qsrc / 'q2repro/q2proto/src/q2proto_proto_kex.c').read_text()
    source = r'''
#define Q2PROTO_BUILD
#include "q2proto_internal.h"
#include <assert.h>
#include <string.h>
#include <stdlib.h>
typedef struct {uint8_t *bytes;uint32_t size,pos;} kex_io_t;
uint8_t q2protoio_read_u8(uintptr_t arg) {
 kex_io_t *io=(void*)arg;assert(io->pos<io->size);return io->bytes[io->pos++];
}
uint16_t q2protoio_read_u16(uintptr_t arg) {
 uint16_t v=q2protoio_read_u8(arg);return v|((uint16_t)q2protoio_read_u8(arg)<<8);
}
uint32_t q2protoio_read_u32(uintptr_t arg) {
 uint32_t v=q2protoio_read_u16(arg);return v|((uint32_t)q2protoio_read_u16(arg)<<16);
}
void q2protoio_write_u8(uintptr_t arg,uint8_t v) {
 kex_io_t *io=(void*)arg;assert(io->size<1400);io->bytes[io->size++]=v;
}
void q2protoio_write_u16(uintptr_t arg,uint16_t v) {
 q2protoio_write_u8(arg,v);q2protoio_write_u8(arg,v>>8);
}
void q2protoio_write_u32(uintptr_t arg,uint32_t v) {
 q2protoio_write_u16(arg,v);q2protoio_write_u16(arg,v>>16);
}
q2proto_error_t q2protoio_get_error(uintptr_t arg) {(void)arg;return Q2P_ERR_SUCCESS;}
'''
    source += '\n'.join(re.findall(r'^#define GUNBIT_.*$', original, re.M)) + '\n'
    start = original.index('#define READ_CHECKED_GUNOFFSET_COMP_FLOAT')
    end = original.index('static q2proto_error_t kex_client_read_playerstate', start)
    source += original[start:end]
    source += function(original, 'kex_server_write_playerstate')
    source += function(original, 'kex_client_read_playerstate')
    # The fixture binds already projected words, not a module/game ABI.
    source += 'static float kex_float(uint32_t word) {float v;memcpy(&v,&word,4);return v;}\n'
    source += 'static void kex_put(q2proto_svc_playerstate_t *p,uint32_t *w,bool reading) {\n'
    scalars = {0:'pm_type',7:'pm_time',8:'pm_flags',9:'pm_gravity',23:'gunframe',34:'fov',35:'rdflags',40:'gunrate',41:'pm_viewheight'}
    for i, name in scalars.items():
        source += f' p->{name}=w[{i}];\n'
    source += ' p->gunindex=w[22]&Q2PRO_GUNINDEX_MASK;p->gunskin=w[22]>>Q2PRO_GUNINDEX_BITS;\n'
    vectors = [(1,'pm_origin','coords',True),(4,'pm_velocity','coords',True),
               (10,'pm_delta_angles','angles',False),(16,'viewangles.values','angles',False),
               (24,'gunoffset.values','small_offsets',False),(27,'gunangles.values','small_angles',False)]
    for start, name, kind, coords in vectors:
        target = f'(reading?&p->{name}.read.value.values:&p->{name}.write.current)' if coords else f'&p->{name}'
        source += f' for(int i=0;i<3;i++)q2proto_var_{kind}_set_float_comp({target},i,kex_float(w[{start}+i]));\n'
    for start,name,kind,scale in [(13,'viewoffset','small_offsets',16),(19,'kick_angles','small_angles',1024)]:
        source += f' for(int i=0;i<3;i++)q2proto_var_{kind}_set_float_comp(&p->{name},i,(int32_t)w[{start}+i]/{scale}.f);\n'
    for start,name in [(30,'blend.values'),(36,'damage_blend.values')]:
        source += f' for(int i=0;i<4;i++)q2proto_var_color_set_byte_comp(&p->{name},i,w[{start}+i]);\n'
    source += ' for(int i=0;i<64;i++)p->stats[i]=w[42+i];\n}\n'
    source += 'static void kex_get(q2proto_svc_playerstate_t *p,uint32_t *w) {\n'
    for i,name in scalars.items():
        source += f' w[{i}]=(int)p->{name};\n'
    source += ' w[22]=p->gunindex|(p->gunskin<<Q2PRO_GUNINDEX_BITS);\n'
    for start,name,kind,coords in vectors:
        target = f'&p->{name}.read.value.values' if coords else f'&p->{name}'
        source += f' for(int i=0;i<3;i++){{float v=q2proto_var_{kind}_get_float_comp({target},i);memcpy(w+{start}+i,&v,4);}}\n'
    for start,name,kind in [(13,'viewoffset','small_offsets'),(19,'kick_angles','small_angles')]:
        suffix = 'viewoffset' if start==13 else 'kick_angles'
        source += f' for(int i=0;i<3;i++)w[{start}+i]=(int)q2proto_var_{kind}_get_q2repro_{suffix}_comp(&p->{name},i);\n'
    for start,name in [(30,'blend.values'),(36,'damage_blend.values')]:
        source += f' for(int i=0;i<4;i++)w[{start}+i]=q2proto_var_color_get_byte_comp(&p->{name},i);\n'
    source += ' for(int i=0;i<64;i++)w[42+i]=(int)p->stats[i];\n}\n'
    source += r'''
static q2proto_svc_playerstate_t kex_player_delta(uint32_t *from,uint32_t *to) {
 q2proto_svc_playerstate_t p={0};kex_put(&p,to,false);
 for(int i=0;i<3;i++) {
  q2proto_var_coords_set_float_comp(&p.pm_origin.write.prev,i,kex_float(from[1+i]));
  q2proto_var_coords_set_float_comp(&p.pm_velocity.write.prev,i,kex_float(from[4+i]));
 }
'''
    flags = {0:'PM_TYPE',7:'PM_TIME',8:'PM_FLAGS',9:'PM_GRAVITY',22:'GUNINDEX',23:'GUNFRAME',34:'FOV',35:'RDFLAGS',40:'GUNRATE',41:'PM_VIEWHEIGHT'}
    for i,name in flags.items():
        source += f' if(from[{i}]!=to[{i}])p.delta_bits|=Q2P_PSD_{name};\n'
    for start,name,count,floating in [(10,'PM_DELTA_ANGLES',3,True),(13,'VIEWOFFSET',3,False),(19,'KICKANGLES',3,False)]:
        compare = f'kex_float(from[{start}+i])!=kex_float(to[{start}+i])' if floating else f'from[{start}+i]!=to[{start}+i]'
        source += f' for(int i=0;i<{count};i++)if({compare})p.delta_bits|=Q2P_PSD_{name};\n'
    for start,name,count,floating in [(16,'viewangles',3,True),(24,'gunoffset',3,True),(27,'gunangles',3,True),(30,'blend',4,False),(36,'damage_blend',4,False)]:
        compare = f'kex_float(from[{start}+i])!=kex_float(to[{start}+i])' if floating else f'from[{start}+i]!=to[{start}+i]'
        source += f' for(int i=0;i<{count};i++)if({compare})p.{name}.delta_bits|=1u<<i;\n'
    source += r'''
 for(int i=0;i<64;i++)if(from[42+i]!=to[42+i])p.statbits|=UINT64_C(1)<<i;
 return p;
}
uint32_t kex_player_encode(uint8_t *bytes,uint32_t *from,uint32_t *to) {
 kex_io_t io={.bytes=bytes};q2proto_svc_playerstate_t p=kex_player_delta(from,to);
 assert(kex_server_write_playerstate(NULL,(uintptr_t)&io,&p)==Q2P_ERR_SUCCESS);
 return io.size;
}
void kex_player_decode(uint8_t *bytes,uint32_t size,uint32_t *from,uint32_t *out) {
 kex_io_t io={.bytes=bytes,.size=size};q2proto_svc_playerstate_t p={0};kex_put(&p,from,true);
 assert(q2protoio_read_u8((uintptr_t)&io)==svc_playerinfo);
 assert(kex_client_read_playerstate(NULL,(uintptr_t)&io,&p)==Q2P_ERR_SUCCESS);
 assert(io.pos==size);kex_get(&p,out);
}
'''
    return source


def repro_entity_reference(qsrc):
    """Native metadata, writer and reader, with cold packed-word bindings."""
    base = qsrc / 'q2repro/q2proto/src'
    original = (base / 'q2proto_proto_q2repro.c').read_text()
    common = (base / 'q2proto_internal_common.c').read_text()
    source = '\n'
    for name in ['q2proto_common_server_write_entity_bits', 'q2proto_common_client_read_entity_bits']:
        source += function(common, name)
    for name in ['q2repro_server_make_entity_state_delta', 'q2proto_q2repro_server_write_entity_state_delta', 'q2repro_client_read_entity_delta']:
        source += function(original, name)
    kex = (base / 'q2proto_proto_kex.c').read_text()
    for name in ['kex_server_make_entity_state_delta', 'kex_server_write_entity_state_delta', 'kex_client_read_entity_delta']:
        source += function(kex, name)
    scalars = {0:('modelindex','MODELINDEX'),1:('modelindex2','MODELINDEX2'),
               2:('modelindex3','MODELINDEX3'),3:('modelindex4','MODELINDEX4'),
               4:('frame','FRAME'),5:('skinnum','SKINNUM'),6:('effects','EFFECTS'),
               7:('renderfx','RENDERFX'),17:('sound','SOUND'),18:('event','EVENT'),
               19:('solid','SOLID'),20:('effects_more','EFFECTS_MORE'),
               21:('alpha','ALPHA'),22:('scale','SCALE'),
               23:('loop_volume','LOOP_VOLUME'),24:('loop_attenuation','LOOP_ATTENUATION')}
    source += 'static void repro_entity_put(q2proto_packed_entity_state_t *p,uint32_t *w) {\n'
    for i,(name,_) in scalars.items():
        if i not in (6,20): source += f' p->{name}=w[{i}];\n'
    source += ' p->effects=w[6]|((uint64_t)w[20]<<32);\n'
    source += ' for(int i=0;i<3;i++){p->origin[i]=w[8+i];p->angles[i]=w[11+i];p->old_origin[i]=w[14+i];}\n}\n'
    source += r'''
static void enhanced_entity_get(q2proto_entity_state_delta_t*,uint32_t*,uint32_t*,bool);
uint32_t repro_entity_encode(uint8_t *bytes,uint32_t *from,uint32_t *to,uint16_t number,uint8_t flags) {
 kex_io_t io={.bytes=bytes};q2proto_packed_entity_state_t a={0},b={0};
 repro_entity_put(&a,from);repro_entity_put(&b,to);
 if(flags&2)assert(q2proto_common_server_write_entity_bits((uintptr_t)&io,U_REMOVE,number)==Q2P_ERR_SUCCESS);
 else {
  q2proto_entity_state_delta_t d;
  q2repro_server_make_entity_state_delta(NULL,&a,&b,(flags&4)!=0,&d);
  assert(q2proto_q2repro_server_write_entity_state_delta(NULL,(uintptr_t)&io,number,&d)==Q2P_ERR_SUCCESS);
 }
 return io.size;
}
void repro_entity_decode(uint8_t *bytes,uint32_t size,uint32_t *from,uint32_t *out,uint32_t *number,uint8_t *removed) {
 kex_io_t io={.bytes=bytes,.size=size};uint64_t bits;uint16_t entnum;
 assert(q2proto_common_client_read_entity_bits((uintptr_t)&io,&bits,&entnum)==Q2P_ERR_SUCCESS);
 *number=entnum;*removed=(bits&U_REMOVE)!=0;
 if(*removed){assert(io.pos==size);return;}
 q2proto_entity_state_delta_t d={0};
 assert(q2repro_client_read_entity_delta(NULL,(uintptr_t)&io,bits,&d)==Q2P_ERR_SUCCESS);
 assert(io.pos==size);enhanced_entity_get(&d,from,out,false);
}
uint32_t kex_entity_encode(uint8_t *bytes,uint32_t *from,uint32_t *to,uint16_t number,uint8_t flags,bool demo) {
 kex_io_t io={.bytes=bytes};q2proto_packed_entity_state_t a={0},b={0};
 repro_entity_put(&a,from);repro_entity_put(&b,to);
 q2proto_servercontext_t context={0};context.protocol=demo?Q2P_PROTOCOL_KEX_DEMOS:Q2P_PROTOCOL_KEX;
 if(flags&2)assert(q2proto_common_server_write_entity_bits((uintptr_t)&io,U_REMOVE,number)==Q2P_ERR_SUCCESS);
 else {
  q2proto_entity_state_delta_t d;
  kex_server_make_entity_state_delta(&context,&a,&b,(flags&4)!=0,&d);
  assert(kex_server_write_entity_state_delta(&context,(uintptr_t)&io,number,&d,(flags&8)?true:(flags&16)?false:from[19]!=0)==Q2P_ERR_SUCCESS);
 }
 return io.size;
}
void kex_entity_decode(uint8_t *bytes,uint32_t size,uint32_t *from,uint32_t *out,uint32_t *number,uint8_t *removed,bool demo,uint8_t flags) {
 kex_io_t io={.bytes=bytes,.size=size};uint64_t bits;uint16_t entnum;
 assert(q2proto_common_client_read_entity_bits((uintptr_t)&io,&bits,&entnum)==Q2P_ERR_SUCCESS);
 *number=entnum;*removed=(bits&U_REMOVE)!=0;
 if(*removed){assert(io.pos==size);return;}
 q2proto_clientcontext_t context={0};context.server_protocol=demo?Q2P_PROTOCOL_KEX_DEMOS:Q2P_PROTOCOL_KEX;
 q2proto_entity_state_delta_t d={0};
 assert(kex_client_read_entity_delta(&context,(uintptr_t)&io,bits,entnum,&d,(flags&8)?true:(flags&16)?false:from[19]!=0)==Q2P_ERR_SUCCESS);
 assert(io.pos==size);enhanced_entity_get(&d,from,out,true);
}
static void enhanced_entity_get(q2proto_entity_state_delta_t *p,uint32_t *from,uint32_t *out,bool floating_angles) {
 q2proto_entity_state_delta_t d=*p;
 memcpy(out,from,25*4);out[18]=0;
'''
    for i,(name,flag) in scalars.items():
        source += f' if(d.delta_bits&Q2P_ESD_{flag})out[{i}]=d.{name};\n'
    source += r'''
 for(int i=0;i<3;i++) {
  if(d.origin.read.value.delta_bits&(1u<<i))out[8+i]=_q2proto_valenc_float2bits(q2proto_var_coords_get_float_comp(&d.origin.read.value.values,i));
  if(d.angle.delta_bits&(1u<<i))out[11+i]=floating_angles?_q2proto_valenc_float2bits(q2proto_var_angles_get_float_comp(&d.angle.values,i)):(uint32_t)(int32_t)q2proto_var_angles_get_short_comp(&d.angle.values,i);
  if(d.delta_bits&Q2P_ESD_OLD_ORIGIN)out[14+i]=_q2proto_valenc_float2bits(q2proto_var_coords_get_float_comp(&d.old_origin,i));
  else if(!(out[7]&128))out[14+i]=from[8+i];
 }
}
'''
    return source


def layouts(msg):
    result = []
    for name, macro in [('entityStateFields', 'NETF'), ('playerStateFields', 'PSF')]:
        block = re.search(r'netField_t\s+' + name + r'\[\]\s*=\s*\{.*?\n\};', msg, re.S).group()
        rows = re.findall(r'\{\s*' + macro + r'\((.*?)\),\s*(-?\d+|GENTITYNUM_BITS)\s*\}', block)
        result.append([(field, 10 if bits == 'GENTITYNUM_BITS' else int(bits)) for field, bits in rows])
    return result


def reference_source(qsrc):
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
    source += qw_reference(qsrc)
    source += q2_entity_reference(qsrc)
    source += nq_reference(qsrc)
    source += qw_player_reference(qsrc)
    source += rr_stats_reference(qsrc)
    source += kex_stats_reference(qsrc)
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
 {byte wire[]={0x90,0x80,2,1,4,0xbf,0xfe};msg_t check={.data=wire,.cursize=sizeof(wire)};
  uint32_t from[112]={0},out[112]={0},number=1;byte removed=0;
  q2_entity_decode(&check,from,out,&number,&removed);
  if(out[4]!=(uint32_t)-321||number!=1||removed||net_message.readcount!=7)return 9;
 }
 Huff_Init(&msgHuff);for(int i=0;i<256;i++)for(int j=0;j<msg_hData[i];j++)Huff_addRef(&msgHuff.compressor,(byte)i);
 msgHuff.decompressor=msgHuff.compressor;msgHuff.decompressor.tree=msgHuff.compressor.tree;
 byte mode,flags;uint16_t number;uint32_t from[112],to[112];
 while(fread(&mode,1,1,stdin)==1) {
  if(mode>14||fread(&flags,1,1,stdin)!=1||fread(&number,2,1,stdin)!=1||fread(from,4,112,stdin)!=112||fread(to,4,112,stdin)!=112)return 2;
  byte data[1400]={0};msg_t m={.data=data,.maxsize=sizeof(data)};
  entityState_t a={.number=number},b={.number=number},c={0};playerState_t p={0},q={0},r={0};
  if(mode==0) {
   put(&a,entityStateFields,51,from);put(&b,entityStateFields,51,to);
   MSG_WriteDeltaEntity(&m,&a,(flags&2)?NULL:&b,flags&1);
  } else if(mode==1) {
   put(&p,playerStateFields,48,from);arrays(&p,from,0);put(&q,playerStateFields,48,to);arrays(&q,to,0);
   MSG_WriteDeltaPlayerstate(&m,&p,&q);
  } else if(mode==2) {
   client_frame_t x={0},y={0};q2_put(&x.ps,from);q2_put(&y.ps,to);SV_WritePlayerstateToClient(&x,&y,&m);
  } else if(mode==3) {qw_encode(&m,from,to,number,flags);
  } else if(mode==4) {q2_entity_encode(&m,from,to,number,flags);
  } else if(mode==5) {nq_encode(&m,from,to,number,flags);
  } else if(mode==6) {nq_player_encode(&m,to,flags);
  } else if(mode==7) {qw_player_encode(&m,to,number);
  } else if(mode==8) {rr_stats_encode(&m,from,to);
  } else if(mode==9) {rr_player_encode(&m,from,to);
  } else if(mode==10) {kex_stats_encode(&m,from,to);
  } else if(mode==11) {m.cursize=kex_player_encode(data,from,to);m.bit=m.cursize*8;
  } else if(mode==12) {m.cursize=repro_entity_encode(data,from,to,number,flags);m.bit=m.cursize*8;
  } else {m.cursize=kex_entity_encode(data,from,to,number,flags,mode==14);m.bit=m.cursize*8;
  }
  uint32_t header[2]={m.bit,m.cursize};fwrite(header,4,2,stdout);fwrite(data,1,m.cursize,stdout);
  uint32_t decoded[112]={0},wire_number=(mode==0||mode>=3)?number:0;byte removed=0;m.bit=m.readcount=0;
  if(mode==0) {
   if(header[0]) {wire_number=MSG_ReadBits(&m,10);MSG_ReadDeltaEntity(&m,&a,&c,wire_number);removed=(flags&2)!=0;}
   else c=a;
   get(&c,entityStateFields,51,decoded);
  } else if(mode==1) {MSG_ReadDeltaPlayerstate(&m,&p,&r);get(&r,playerStateFields,48,decoded);arrays(&r,decoded,1);}
  else if(mode==2) {player_state_t x={0},y={0};q2_put(&x,from);q2_decode(&m,&x,&y);q2_get(&y,decoded);}
  else if(mode==3) {qw_decode(&m,from,decoded,&wire_number,&removed);}
  else if(mode==4) {q2_entity_decode(&m,from,decoded,&wire_number,&removed);}
  else if(mode==5) {nq_decode(&m,from,decoded,&wire_number);}
  else if(mode==6) {nq_player_decode(&m,decoded);}
  else if(mode==7) {qw_player_decode(&m,from,decoded,&wire_number);}
  else if(mode==8) {rr_stats_decode(&m,from,decoded);}
  else if(mode==9) {rr_player_decode(&m,from,decoded);}
  else if(mode==10) {kex_stats_decode(&m,from,decoded);}
  else if(mode==11) {kex_player_decode(data,m.cursize,from,decoded);}
  else if(mode==12) {repro_entity_decode(data,m.cursize,from,decoded,&wire_number,&removed);}
  else {kex_entity_decode(data,m.cursize,from,decoded,&wire_number,&removed,mode==14,flags);}
  fwrite(decoded,4,112,stdout);fwrite(&wire_number,4,1,stdout);fwrite(&removed,1,1,stdout);
 }
 return ferror(stdin)?3:0;
}
'''
    stat_layout = [(f'stats[{i}]',16) for i in range(64)]
    entity_layout = q2_entity_layout()+[('effects_high',32),('alpha',8),('scale',8),('loop_volume',8),('loop_attenuation',8)]
    return source, layouts(msg) + [q2_layout(), qw_layout(), q2_entity_layout(), nq_layout(), nq_player_layout(), qw_player_layout(), stat_layout, q2_layout()+[(f'damage_blend[{i}]',8) for i in range(4)]+[('gunrate',8),('pmove.viewheight',-8),('clientnum',-16)]+stat_layout, stat_layout, q2_layout()+[(f'damage_blend[{i}]',8) for i in range(4)]+[('gunrate',8),('pmove.viewheight',-8)]+stat_layout, entity_layout, entity_layout, entity_layout]


def compile_reference(qsrc, evidence):
    source, tables = reference_source(qsrc)
    declarations = 'extern uint32_t kex_player_encode(uint8_t*,uint32_t*,uint32_t*);\nextern void kex_player_decode(uint8_t*,uint32_t,uint32_t*,uint32_t*);\n'
    declarations += 'extern uint32_t repro_entity_encode(uint8_t*,uint32_t*,uint32_t*,uint16_t,uint8_t);\nextern void repro_entity_decode(uint8_t*,uint32_t,uint32_t*,uint32_t*,uint32_t*,uint8_t*);\n'
    declarations += 'extern uint32_t kex_entity_encode(uint8_t*,uint32_t*,uint32_t*,uint16_t,uint8_t,_Bool);\nextern void kex_entity_decode(uint8_t*,uint32_t,uint32_t*,uint32_t*,uint32_t*,uint8_t*,_Bool,uint8_t);\n'
    source = source.replace('int main(void)',declarations+'int main(void)',1)
    code = evidence / 'original-state-delta.c'
    code.write_text(source)
    binary = evidence / 'original-state-delta'
    kex_code = evidence / 'original-kex-player.c'
    kex_code.write_text(kex_player_reference(qsrc)+repro_entity_reference(qsrc))
    base = qsrc / 'q2repro/q2proto'
    command = ['cc', '-O2', '-std=c11', '-fno-strict-aliasing', '-ffp-contract=off',
               '-ffunction-sections','-fdata-sections','-DQ2PROTO_CONFIG_PROVIDED=1',
               '-DQ2PROTO_PLAYER_STATE_FEATURES=Q2PROTO_FEATURES_RERELEASE',
               '-DQ2PROTO_ENTITY_STATE_FEATURES=Q2PROTO_FEATURES_RERELEASE',
               '-I',str(base/'inc'),'-I',str(base/'src'),str(code),str(kex_code),str(base/'src/q2proto_coords.c'),
               '-Wl,--gc-sections','-lm','-o',str(binary)]
    (evidence/'compile-command.json').write_text(json.dumps(command,indent=2)+'\n')
    subprocess.run(command, check=True)
    return binary, tables


def fixture(tables):
    rng = random.Random(8606851)
    output = bytearray()
    floats = [-0.0, 0.0, -4096.0, -4097.0, 4095.0, 4096.0, 0.125, -0.125, 123456.75]
    for mode, table in enumerate(tables):
        for case in range(2048):
            if mode in (13,14):
                old,new=[0]*112,[0]*112
                for i in range(25):
                    if 8<=i<=16:
                        a,b=rng.uniform(-4096,4096),rng.uniform(-4096,4096)
                        if case<32:a,b=(-0.0,0.0) if i%2 else (0.0,-0.0)
                        old[i]=struct.unpack('<I',struct.pack('<f',a))[0]
                        new[i]=struct.unpack('<I',struct.pack('<f',b))[0]
                    elif i<5:
                        old[i],new[i]=rng.randrange(65536),rng.randrange(65536)
                    elif i==17:
                        old[i],new[i]=rng.randrange(16384),rng.randrange(16384)
                    elif i==18 or i>=21:
                        old[i],new[i]=rng.randrange(256),rng.randrange(256)
                    else:
                        old[i],new[i]=rng.getrandbits(32),rng.getrandbits(32)
                    if (case+i)%3:new[i]=old[i]
                if case<25:
                    word=new[case];new=old.copy();new[case]=word
                if case%8==0:new=old.copy();new[18]=0
                old[19]=1 if case%2 else 0
                new[19]=1 if case%3 else 0
                if 25<=case<41:
                    old[6]=0x12345678;old[20]=0x100
                    new[6]=[0,255,256,32767,32768,65535,65536,0xffffffff][case%8]
                    new[20]=new[6] if case<33 else 0
                if 41<=case<49:
                    new=old.copy();new[22]=(old[22]+1)&255
                if 49<=case<57:
                    new[8]=struct.unpack('<I',struct.pack('<f',1.26))[0]
                    old[8]=struct.unpack('<I',struct.pack('<f',1.25))[0]
                flags=(2 if case%17==0 else 0)|(4 if case%3==0 else 0)
                number=[1,255,256,1023,8191][case%5]
                output+=struct.pack('<BBH224I',mode,flags,number,*old,*new)
                continue
            if mode == 12:
                old,new=[0]*112,[0]*112
                for i in range(25):
                    if 8<=i<=10 or 14<=i<=16:
                        a,b=rng.uniform(-4096,4096),rng.uniform(-4096,4096)
                        old[i]=struct.unpack('<I',struct.pack('<f',a))[0]
                        new[i]=struct.unpack('<I',struct.pack('<f',b))[0]
                    elif 11<=i<=13:
                        old[i],new[i]=rng.randrange(-32768,32768)&0xffffffff,rng.randrange(-32768,32768)&0xffffffff
                    elif i<5:
                        old[i],new[i]=rng.randrange(65536),rng.randrange(65536)
                    elif i==17:
                        old[i],new[i]=rng.randrange(16384),rng.randrange(16384)
                    elif i==18 or i>=21:
                        old[i],new[i]=rng.randrange(256),rng.randrange(256)
                    else:
                        old[i],new[i]=rng.getrandbits(32),rng.getrandbits(32)
                    if (case+i)%3:new[i]=old[i]
                if case<25:
                    word=new[case];new=old.copy();new[case]=word
                if case%8==0:new=old.copy();new[18]=0
                if 25<=case<40:
                    old=[0]*112;new=old.copy()
                    new[5]=[255,256,32767,32768,65535,65536,0x80000000,0xffffffff][case%8]
                    new[20]=new[5];new[22]=255
                if 40<=case<56:
                    old[8]=struct.unpack('<I',struct.pack('<f',1.25))[0]
                    new[8]=struct.unpack('<I',struct.pack('<f',1.25+(0.01 if case%2 else 0.125)))[0]
                if 56<=case<64:
                    old[8]=0x80000000;new[8]=0
                flags=(2 if case%17==0 else 0)|(4 if case%3==0 else 0)
                number=[1,255,256,8191,65535][case%5]
                output+=struct.pack('<BBH224I',mode,flags,number,*old,*new)
                continue
            if mode in (9,11):
                old,new=[0]*112,[0]*112
                count = 107 if mode==9 else 106
                for i in range(count):
                    if 1<=i<=6 or 10<=i<=12 or mode==11 and (16<=i<=18 or 24<=i<=29):
                        a,b=rng.uniform(-32768,32768),rng.uniform(-32768,32768)
                        if case<32:a,b=(-0.0,0.0) if i%2 else (0.0,-0.0)
                        old[i]=struct.unpack('<I',struct.pack('<f',a))[0]
                        new[i]=struct.unpack('<I',struct.pack('<f',b))[0]
                    elif i in (7,8,22):
                        old[i],new[i]=rng.randrange(65536),rng.randrange(65536)
                    elif i==23 and mode==11:
                        old[i],new[i]=rng.randrange(512),rng.randrange(512)
                    elif i in (0,23,30,31,32,33,34,35,36,37,38,39,40):
                        old[i],new[i]=rng.randrange(256),rng.randrange(256)
                    elif i==41:
                        old[i],new[i]=rng.randrange(-128,128)&0xffffffff,rng.randrange(-128,128)&0xffffffff
                    else:
                        old[i],new[i]=rng.randrange(-32768,32768)&0xffffffff,rng.randrange(-32768,32768)&0xffffffff
                    if (case+i)%3:new[i]=old[i]
                if case<count:
                    field=new[case]
                    new=old.copy();new[case]=field
                if case==count:new=old.copy()
                if count+1<=case<count+9:
                    old=[0]*112;new=old.copy()
                    new[1]=[0,0x80000000,0x7f800000,0xff800000,0x7fc00001,0x7f800001,1,0x80000001][case-count-1]
                output+=struct.pack('<BBH224I',mode,0,0,*old,*new)
                continue
            if mode in (8,10):
                old,new=[0]*112,[0]*112
                for i in range(64):
                    old[i]=rng.randrange(-32768,32768)&0xffffffff
                    new[i]=(rng.randrange(-32768,32768)&0xffffffff) if (case+i)%3==0 else old[i]
                if case<64:
                    new=old.copy();new[case]=(old[case]+1)&0xffff
                    if new[case]&0x8000:new[case]|=0xffff0000
                if case==64:new=old.copy()
                if case==65:new=[((old[i]^1)&0xffffffff) if i<64 else 0 for i in range(112)]
                output+=struct.pack('<BBH224I',mode,0,0,*old,*new)
                continue
            if mode == 7:
                old,new=[0]*112,[0]*112;old[8]=case%256
                for i in list(range(4))+list(range(5,12))+[13,14,15,16]:
                    new[i]=struct.unpack('<I',struct.pack('<f',rng.uniform(-4096,4096)))[0]
                new[4]=rng.randrange(-128,1024)&0xffffffff
                # Native local/spectator rows have no command/msec; remote rows
                # carry both. Other flags include all defined prediction bits.
                new[12]=(case|((case&1)<<11))&~3
                if case%3==0: new[12]|=3
                if case%3==1: new[12]&=0x1c
                for i in range(14,17):old[i]=struct.unpack('<I',struct.pack('<f',rng.uniform(-4096,4096)))[0]
                for i in range(17,20):
                    old[i]=rng.randrange(-32768,32768)&0xffffffff;new[i]=rng.randrange(-32768,32768)&0xffffffff
                for i in range(20,23):old[i]=rng.randrange(256);new[i]=rng.randrange(256)
                output += struct.pack('<BBH224I',mode,0,case%32,*old,*new)
                continue
            if mode == 6:
                old,new=[0]*112,[0]*112
                for i in list(range(8))+[9,10]+list(range(12,18)):
                    value=rng.uniform(-2048,2048) if i<8 else rng.uniform(-65536,65536) if i==12 else rng.uniform(-256,512)
                    if case<256:
                        value=[0.0,-0.0,0.25,-0.25,127.9,128.0,-128.9,-129.0][(case+i)%8]
                    new[i]=struct.unpack('<I',struct.pack('<f',value))[0]
                if case%4==0: new[0]=struct.unpack('<I',struct.pack('<f',22.0))[0]
                new[8]=rng.randrange(1<<24)|(rng.randrange(16)<<28)
                new[11]=rng.randrange(256)
                flags=4 if case%2 else 0
                new[18]=case%32 if flags&4 else case%256
                new[19]=int(case%3!=0);new[20]=int(case%5!=0)
                if case>=256 and case%8==0:
                    new=[0]*112;new[0]=struct.unpack('<I',struct.pack('<f',22.0))[0]
                output += struct.pack('<BBH224I',mode,flags,0,*old,*new)
                continue
            if mode == 5:
                old,new=[0]*112,[0]*112
                for i in range(5):
                    old[i]=rng.randrange(17 if i==2 else 256)
                    value=old[i]+rng.choice([0.0,0.0,0.25,-0.25,1.0])
                    if i==2: value=max(0,min(16,value))
                    new[i]=struct.unpack('<I',struct.pack('<f',value))[0]
                for i in range(5,11):
                    old[i]=struct.unpack('<I',struct.pack('<f',rng.uniform(-4096,4096)))[0]
                    value=struct.unpack('<f',struct.pack('<I',old[i]))[0]
                    if case%4: value+=rng.uniform(-8,8)
                    new[i]=struct.unpack('<I',struct.pack('<f',value))[0]
                if case<128:
                    old=[0]*112;new=old.copy();old[1]=4
                    new[1]=struct.unpack('<I',struct.pack('<f',4.5 if case%2 else 4.0))[0]
                    new[6]=struct.unpack('<I',struct.pack('<f',[1.40625,-1.40625,1.999,-1.999,180.25,-180.25][case%6]))[0]
                    new[5]=[0x3dcccccc,0x3dcccccd,0x3dccccce,0xbdcccccc,0xbdcccccd,0xbdccccce][case%6]
                flags=4 if case%2 else 0
                number=[1,255,256,599,32767][case%5]
                output += struct.pack('<BBH224I',mode,flags,number,*old,*new)
                continue
            old, new = [0] * 112, [0] * 112
            for i, (_, width) in enumerate(table):
                if mode == 3 and 5 <= i <= 10:
                    value = lambda: struct.unpack('<I',struct.pack('<f',rng.uniform(-4096,4096)))[0]
                elif mode == 4:
                    if 8 <= i <= 16:
                        value = lambda: struct.unpack('<I',struct.pack('<f',rng.uniform(-4096,4096)))[0]
                    else:
                        value = lambda: rng.getrandbits(32)
                elif mode == 2:
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
                    floating = width == 0 and mode != 4 or mode == 2 and (13 <= i <= 21 or 24 <= i <= 34) or mode == 3 and 5 <= i <= 10 or mode == 4 and 8 <= i <= 16
                    new[i] = struct.unpack('<I', struct.pack('<f', 0.25))[0] if floating else old[i] ^ 1
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
            if mode == 3 and case < 128:
                old = [0]*112;new = old.copy()
                edge = [0x3dcccccc,0x3dcccccd,0x3dccccce,0xbdcccccc,0xbdcccccd,0xbdccccce][case%6]
                if case < 96: new[5]=edge
                new[11] = 64 if case%2 else 0
                if case >= 96: new[0]=3 if case%4 else 0
            flags = case % 2 | (2 if mode in (0,3,4) and case % 17 == 0 else 0)
            if mode == 4:
                flags |= 4 if case%3==0 else 0
                if case < 128:
                    old=[0]*112;new=old.copy()
                    edges=[0,255,256,32767,32768,65535,65536,0x7fffffff,0x80000000,0xffffffff]
                    for i in (4,5,6,7): new[i]=edges[(case+i)%len(edges)]
                    old[18]=7;new[18]=0 if case%2 else 7
                    old[8]=struct.unpack('<I',struct.pack('<f',1.25))[0]
                    old[14]=struct.unpack('<I',struct.pack('<f',-4.0))[0]
                    new[8]=old[8];new[14]=old[14]
                    if case>=100: new=old.copy();new[18]=0
            number = 1 + case%511 if mode==3 else 1+case%1023 if mode==4 else case%1023
            output += struct.pack('<BBH224I', mode, flags, number, *old, *new)
    for case in range(512):
        old, new = [0] * 112, [0] * 112
        old[19] = new[19] = 0 if case % 2 else 1
        old[8] = struct.unpack('<I', struct.pack('<f', 3.125))[0]
        new[8] = struct.unpack('<I', struct.pack('<f', -1.01 - case * 0.125))[0]
        new[14] = struct.unpack('<I', struct.pack('<f', 5.03 + case * 0.125))[0]
        flags = 4 | (8 if case % 2 else 16)
        output += struct.pack('<BBH224I', 14, flags, 1 + case % 8191, *old, *new)

    return output


def compare_entity_headers(qsrc, evidence, probe):
    """Cold IO bindings around the unchanged native entity-header functions."""
    common = (qsrc / 'q2repro/q2proto/src/q2proto_internal_common.c').read_text()
    protocol = (qsrc / 'q2repro/inc/common/protocol.h').read_text()
    source = r'''
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
typedef int q2proto_error_t;
#define BIT_ULL(n) (UINT64_C(1) << (n))
#define Q2P_ERR_SUCCESS 0
typedef struct {uint8_t bytes[7];uint32_t size,pos;} header_io_t;
static void header_u8_write(uintptr_t arg,uint8_t value) {
 header_io_t *io=(void*)arg;assert(io->size<7);io->bytes[io->size++]=value;
}
static void header_u16_write(uintptr_t arg,uint16_t value) {
 header_u8_write(arg,value);header_u8_write(arg,value>>8);
}
static uint8_t header_u8_read(uintptr_t arg) {
 header_io_t *io=(void*)arg;assert(io->pos<io->size);return io->bytes[io->pos++];
}
static uint16_t header_u16_read(uintptr_t arg) {
 uint16_t lo=header_u8_read(arg);return lo|((uint16_t)header_u8_read(arg)<<8);
}
#define READ_CHECKED(scope,arg,dest,type) ((dest)=header_##type##_read(arg))
#define WRITE_CHECKED(scope,arg,type,value) header_##type##_write(arg,value)
static int bitcounts[32];
#define MSG_ReadByte(m) header_u8_read((uintptr_t)m)
#define MSG_ReadShort(m) ((int16_t)header_u16_read((uintptr_t)m))
static header_io_t net_message;
'''
    source += '\n'.join(re.findall(r'^#define\s+U_[A-Z0-9_]+\s+.*$', protocol, re.M)) + '\n'
    source += function(common, 'q2proto_common_server_write_entity_bits')
    source += function(common, 'q2proto_common_client_read_entity_bits')
    source += function((qsrc / 'quake-2/client/cl_ents.c').read_text(), 'CL_ParseEntityBits')
    source += r'''
int main(void) {
 uint8_t mode;uint16_t number;uint64_t flags;
 while(fread(&mode,1,1,stdin)==1) {
  if(mode>1||fread(&number,2,1,stdin)!=1||fread(&flags,8,1,stdin)!=1)return 2;
  header_io_t io={0};
  assert(q2proto_common_server_write_entity_bits((uintptr_t)&io,flags,number)==0);
  uint64_t decoded;uint16_t decoded_number;
  if(mode)assert(q2proto_common_client_read_entity_bits((uintptr_t)&io,&decoded,&decoded_number)==0);
  else {net_message=io;unsigned legacy_flags;decoded_number=CL_ParseEntityBits(&legacy_flags);decoded=legacy_flags;io.pos=net_message.pos;}
  assert(io.pos==io.size);
  fwrite(&io.size,4,1,stdout);fwrite(io.bytes,1,io.size,stdout);
  fwrite(&decoded,8,1,stdout);fwrite(&decoded_number,2,1,stdout);
 }
 return ferror(stdin)?3:0;
}
'''
    code = evidence / 'original-entity-headers.c'
    code.write_text(source)
    binary = evidence / 'original-entity-headers'
    subprocess.run(['cc', '-O2', '-std=c11', str(code), '-o', str(binary)], check=True)
    rng = random.Random(8601640)
    data = bytearray()
    continuations = sum(1 << bit for bit in (7, 15, 23, 31))
    for mode in range(2):
        for case in range(2048):
            number = [0, 1, 255, 256, 1023, 8191, 65535][case % 7]
            width = 40 if mode else 31
            flags = ((1 << (case % width)) if case < width else rng.getrandbits(width)) & ~continuations
            if case == 0:
                flags = 0
            data += struct.pack('<BHQ', mode, number, flags)
    expected = subprocess.check_output([binary], input=data)
    actual = subprocess.check_output([probe], input=data)
    for name, contents in [('fixture.bin', data), ('original.bin', expected), ('rust.bin', actual)]:
        (evidence / name).write_bytes(contents)
    if actual != expected:
        at = next((i for i, (a,b) in enumerate(zip(actual,expected)) if a != b), min(len(actual),len(expected)))
        raise AssertionError(f'entity-prefix bytes/flags/numbers differ at output byte {at}')
    result = dict(result='PASS', cases=4096, byte_exact=True, flags_and_numbers_exact=True,
                  original='Unchanged q2proto common entity-bits writer/reader; unchanged original Q2 CL_ParseEntityBits on legacy masks; private bounded IO bindings and native constants',
                  limits='Entity prefixes only; legacy masks omit its undefined bit31/fifth-byte extension. No entity bodies, frames, channel, protocol selection, module ABI or live/installed acceptance')
    (evidence / 'comparison.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--entity-headers', action='store_true', help='compare the shared four/five-byte entity prefix only')
    args = parser.parse_args(); args.evidence.mkdir(parents=True, exist_ok=True)
    if args.entity_headers:
        compare_entity_headers(args.qsrc, args.evidence, args.probe)
        return
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
    result = dict(result='PASS', cases=len(data)//900, entity_fields=51, player_fields=48, player_arrays=64, q2_player_fields=36, q2_stats=32, q2_repro_stats=64, q2_repro_player_words=107, q2_repro_entity_words=25, kex_stats=64, kex_player_words=106, kex_entity_words=25, kex_entity_protocols=[2022,2023], qw_entity_words=12, q2_entity_words=20, q2_dual_frame_flag_parser=True, nq_entity_words=12, nq_player_words=21, qw_player_words=14, bytes=len(actual), byte_exact=True, decoded_words_exact=True,
                  original='Q3 MSG entity/player, Q2 server player writer/client parser, QW SV_WriteDelta/CL_ParseDelta, Q2 entity writer/bits/parser, NQ entity/client-data functions unchanged; QW player writing block, CL_ParsePlayerinfo, usercmd helpers and Q2repro stats/enhanced-player/coordinate/angle/blend functions unchanged; q2proto clientnum statements, KEX stat blocks and full KEX player writer/parser unchanged; Q2repro and KEX entity metadata builders, writers and readers unchanged with native common headers, coordinate code and IO helpers; original removal statements, offsetof and private packed-word/struct bindings',
                  limits='Seeded native delta records; Q2repro comparison uses packed view/weapon/color words and original MSG packed gunframe range 0..255; entity records bind native packed short angles, finite map-range float coordinates and literal metadata including sound-only loop presence; KEX entity variants exercise solid-dependent demo coordinate precision, paired effect halves and native fifth-byte padding with ignored reserved fields; KEX binds already projected scalar words and native delta metadata; no rerelease snapshot/channel framing, common-state/module ABI packing, sign-on, captures, live or installed acceptance')
    (args.evidence / 'comparison.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
