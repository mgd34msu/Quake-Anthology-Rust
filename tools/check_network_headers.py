#!/usr/bin/env python3
"""THE-860/949 native header oracle. Original C runs only in this developer tool."""
import argparse
import json
import pathlib
import re
import struct
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
COMMON = r'''
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
typedef unsigned char byte;
typedef int qboolean;
typedef struct { byte *data; size_t maxsize,cursize,readcount; int overflowed; } sizebuf_t;
typedef sizebuf_t msg_t;
typedef struct { int value,integer; } cvar_t;
typedef int netadr_t;
#define qtrue 1
#define qfalse 0
#define NS_CLIENT 0
#define NS_SERVER 1
#define MAX_LATENT 32
#define PACKET_HEADER 10
#define MAX_PACKETLEN 4096
#define FRAGMENT_SIZE 1300
#define FRAGMENT_BIT (1u<<31)
#define MAX_MSGLEN 1450
static char *netsrcString[2]={"client","server"};
#define USE_CLIENT 1
#define REL_BIT (1u<<31)
#define FRG_BIT (1u<<30)
#define OLD_MASK (REL_BIT-1)
#define NEW_MASK (FRG_BIT-1)
#define PROTOCOL_VERSION_R1Q2 35
#define Q_assert(x) do { if(!(x))abort(); } while(0)
#define Com_Memcpy memcpy
#define Com_Memset memset
#define Q_memcpy memcpy
#define ERR_DROP 1
#define SHOWPACKET(...) ((void)0)
#define SHOWDROP(...) ((void)0)
#define Con_Printf(...) ((void)0)
#define Com_Printf(...) ((void)0)
#define Com_WPrintf(...) ((void)0)
#define NET_AdrToString(x) "private fixture"
static cvar_t disabled,port;
static cvar_t *showpackets=&disabled,*showdrop=&disabled,*qport=&port;
static double realtime,net_time;
static unsigned curtime,com_localTime;
static struct { int qport,demoplayback; } cls;
static byte capture[65536];static size_t captured;
static void save(const void *data,size_t length) { if(length>sizeof(capture))abort();memcpy(capture,data,length);captured=length; }
static void bytes(sizebuf_t *b,const void *p,size_t n) { if(b->cursize+n>b->maxsize)abort();memcpy(b->data+b->cursize,p,n);b->cursize+=n; }
static void word(sizebuf_t *b,uint32_t value,int n) { byte a[4];for(int i=0;i<n;i++)a[i]=(byte)(value>>(8*i));bytes(b,a,n); }
#define MSG_WriteLong(b,x) word(b,(uint32_t)(x),4)
#define MSG_WriteShort(b,x) word(b,(uint32_t)(x),2)
#define MSG_WriteData(b,p,n) bytes(b,p,n)
#define SZ_WriteLong MSG_WriteLong
#define SZ_WriteShort MSG_WriteShort
#define SZ_WriteByte(b,x) word(b,(uint32_t)(x),1)
#define SZ_Write(b,p,n) bytes(b,p,n)
#define SZ_Clear(b) do { (b)->cursize=0;(b)->readcount=0; } while(0)
static void init(sizebuf_t *b,void *p,size_t n) { *b=(sizebuf_t){.data=p,.maxsize=n}; }
#define SZ_Init(b,p,n,...) init(b,p,n)
#define MSG_InitOOB SZ_Init
static void Com_Error(int code,const char *fmt,...) { (void)code;(void)fmt;abort(); }
static bool ServerPaused(void) { return false; }
typedef struct {
 int sock,qport,protocol,type,remote_address,remoteAddress;
 unsigned outgoing_sequence,incoming_sequence,incoming_acknowledged;
 unsigned incoming_reliable_sequence,incoming_reliable_acknowledged;
 unsigned last_reliable_sequence,reliable_sequence;
 size_t reliable_length;byte reliable_buf[65536],message_buf[65536];sizebuf_t message;
 int fatal_error;double cleartime,rate,outgoing_time[MAX_LATENT];unsigned outgoing_size[MAX_LATENT];
 unsigned last_sent;bool fragment_pending,reliable_ack_pending;size_t maxpacketlen;
 sizebuf_t fragment_out;
 int outgoingSequence,incomingSequence,unsentFragmentStart,unsentLength,unsentFragments;
 byte unsentBuffer[65536];
} netchan_t;
static byte payload[65536],fragment_data[65536];
static byte mode,role,reliable,rack,more;static uint32_t sequence,ack;
static uint16_t port_value,offset,length;
static bool fixture(void) {
 if(fread(&mode,1,1,stdin)!=1)return false;
 if(fread(&role,1,1,stdin)!=1 || fread(&sequence,4,1,stdin)!=1 || fread(&ack,4,1,stdin)!=1 ||
 fread(&reliable,1,1,stdin)!=1 || fread(&rack,1,1,stdin)!=1 || fread(&port_value,2,1,stdin)!=1 ||
 fread(&offset,2,1,stdin)!=1 || fread(&more,1,1,stdin)!=1 || fread(&length,2,1,stdin)!=1 ||
 fread(payload,1,length,stdin)!=length)abort();
 captured=0;return true;
}
static void result(void) { uint32_t n=(uint32_t)captured;fwrite(&n,4,1,stdout);fwrite(capture,1,captured,stdout); }
static void state(netchan_t *c) {
 memset(c,0,sizeof(*c));c->sock=role;c->qport=port_value;c->protocol=34;c->type=1;
 c->outgoing_sequence=sequence;c->incoming_sequence=ack;c->incoming_reliable_sequence=rack;
 c->last_reliable_sequence=ack;c->outgoingSequence=(int)sequence;c->maxpacketlen=1390;
 c->message.data=c->message_buf;c->message.maxsize=sizeof(c->message_buf);
 if(reliable) { c->message.cursize=1;c->message_buf[0]=0xfd; }
 port.integer=port.value=cls.qport=port_value;
}
'''
NQ = r'''
#define MAX_DATAGRAM 1024
#define NET_MAXMESSAGE 8192
#define NET_HEADERSIZE 8
#define NETFLAG_LENGTH_MASK 0x0000ffff
#define NETFLAG_DATA 0x00010000
#define NETFLAG_ACK 0x00020000
#define NETFLAG_NAK 0x00040000
#define NETFLAG_EOM 0x00080000
#define NETFLAG_UNRELIABLE 0x00100000
#define NETFLAG_CTL 0x80000000
#define BigLong(x) __builtin_bswap32((uint32_t)(x))
struct qsockaddr { int unused; };
typedef struct { unsigned sendSequence,unreliableSendSequence;int sendMessageLength;
 byte sendMessage[NET_MAXMESSAGE];bool canSend,sendNext;int socket;struct qsockaddr addr;double lastSendTime; } qsocket_t;
static struct { uint32_t length,sequence;byte data[NET_MAXMESSAGE]; } packetBuffer;
static int write_packet(int s,byte *data,int n,struct qsockaddr *addr) { (void)s;(void)addr;save(data,n);return n; }
static struct { int (*Write)(int,byte *,int,struct qsockaddr *); } sfunc={write_packet};
static unsigned packetsSent,packetsReSent;
'''
NQ_DRIVER = r'''
int main(void) {
 while(fixture()) {
  qsocket_t storage={.sendSequence=sequence,.unreliableSendSequence=sequence};qsocket_t *sock=&storage;
  sizebuf_t data={.data=payload,.cursize=length};
  if(mode==0)Datagram_SendUnreliableMessage(sock,&data);
  else if(mode==1)Datagram_SendMessage(sock,&data);
  else if(mode==2) { data.cursize=1025;payload[1024]=0;Datagram_SendMessage(sock,&data); }
  else { ACK_SOURCE }
  result();
 }
 return ferror(stdin)?1:0;
}
'''
DRIVER = r'''
int main(void) {
 while(fixture()) {
  netchan_t chan;state(&chan);
  TRANSMIT_SOURCE
  result();
 }
 return ferror(stdin)?1:0;
}
'''


def function(source, name):
    match = re.search(r'(?:static\s+)?(?:void|int|qboolean|bool)\s+' + name + r'\s*\(', source)
    if match is None:
        raise ValueError(f'original function {name} absent')
    # These original functions have their closing brace at column zero.
    end = source.index('\n}\n', match.start()) + 3
    return source[match.start():end]


def compile_references(qsrc, evidence):
    nq = (qsrc / 'quake/WinQuake/net_dgrm.c').read_text()
    qw = (qsrc / 'quake/QW/client/net_chan.c').read_text()
    q2 = (qsrc / 'quake-2/qcommon/net_chan.c').read_text()
    q2pro = (qsrc / 'q2repro/src/common/net/chan.c').read_text()
    q3 = (qsrc / 'quake-iii-arena/code/qcommon/net_chan.c').read_text()
    ack = re.search(r'packetBuffer.length = BigLong\(NET_HEADERSIZE \| NETFLAG_ACK\);.*?sfunc.Write[^;]+;', nq, re.S).group()
    common = COMMON
    bodies = {
        'nq': common + NQ + function(nq,'Datagram_SendMessage') + function(nq,'Datagram_SendUnreliableMessage') + NQ_DRIVER.replace('ACK_SOURCE',ack.replace('&readaddr','&sock->addr')),
        'qw-client': common + '#define showpackets disabled\nstatic void NET_SendPacket(int n,void *p,int a) { (void)a;save(p,n); }\n' + function(qw,'Netchan_Transmit') + DRIVER.replace('TRANSMIT_SOURCE','Netchan_Transmit(&chan,length,payload);'),
        'qw-server': common + '#define showpackets disabled\n#define SERVERONLY 1\nstatic void NET_SendPacket(int n,void *p,int a) { (void)a;save(p,n); }\n' + function(qw,'Netchan_Transmit') + DRIVER.replace('TRANSMIT_SOURCE','Netchan_Transmit(&chan,length,payload);'),
        'q2': common.replace('#define MAX_MSGLEN 1450','#define MAX_MSGLEN 1400') + 'static void NET_SendPacket(int s,int n,void *p,int a) { (void)s;(void)a;save(p,n); }\n' + function(q2,'Netchan_NeedReliable') + function(q2,'Netchan_Transmit') + DRIVER.replace('TRANSMIT_SOURCE','Netchan_Transmit(&chan,length,payload);'),
        'q2pro': common.replace('#define MAX_MSGLEN 1450','#define MAX_MSGLEN 32768') + 'static void NET_SendPacket(int s,void *p,size_t n,const int *a) { (void)s;(void)a;save(p,n); }\n' + function(q2pro,'Netchan_TransmitNextFragment') + function(q2pro,'NetchanOld_Transmit') + function(q2pro,'NetchanNew_Transmit') + DRIVER.replace('TRANSMIT_SOURCE',r'''
          if(mode==8 || mode==9) { chan.protocol=35;if(mode==9)chan.qport=0;NetchanOld_Transmit(&chan,length,payload,1); }
          else if(mode==10 || mode==11)NetchanNew_Transmit(&chan,length,payload,1);
          else { chan.reliable_length=reliable;chan.fragment_out=(sizebuf_t){.data=fragment_data,.maxsize=sizeof(fragment_data),.readcount=offset,.cursize=offset+length+more};
           chan.maxpacketlen=length<512?512:length;memcpy(fragment_data+offset,payload,length);Netchan_TransmitNextFragment(&chan); }
        '''),
        'q3': common.replace('#define MAX_MSGLEN 1450','#define MAX_MSGLEN 16384').replace('#define MAX_PACKETLEN 4096','#define MAX_PACKETLEN 1400') + 'static void NET_SendPacket(int s,int n,void *p,int a) { (void)s;(void)a;save(p,n); }\n' + function(q3,'Netchan_TransmitNextFragment') + function(q3,'Netchan_Transmit') + DRIVER.replace('TRANSMIT_SOURCE',r'''
          if(mode==14 || mode==15)Netchan_Transmit(&chan,length,payload);
          else { chan.unsentFragmentStart=offset;chan.unsentLength=offset+length;memcpy(chan.unsentBuffer+offset,payload,length);Netchan_TransmitNextFragment(&chan); }
        '''),
    }
    binaries={}
    for name, code in bodies.items():
        source=evidence/f'original-{name}.c';source.write_text(code)
        binary=evidence/f'original-{name}'
        subprocess.run(['cc','-O2','-std=c11',str(source),'-o',str(binary)],check=True)
        binaries[name]=binary
    return binaries


def records(data):
    # Fixed fixture fields precede its explicitly sized payload.
    header=struct.Struct('<BBIIBBHHBH')
    while data:
        if len(data)<header.size:raise ValueError('truncated fixture')
        fields=header.unpack_from(data);n=header.size+fields[-1]
        if len(data)<n:raise ValueError('truncated fixture payload')
        yield fields,data[:n]
        data=data[n:]


def results(data):
    while data:
        n=struct.unpack_from('<I',data)[0]
        if len(data)<4+n:raise ValueError('truncated oracle result')
        yield data[4:4+n]
        data=data[4+n:]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc',type=pathlib.Path,default=ROOT.parent/'qsrc')
    parser.add_argument('--evidence',type=pathlib.Path,required=True)
    parser.add_argument('--probe',type=pathlib.Path,required=True)
    args=parser.parse_args();args.evidence.mkdir(parents=True,exist_ok=True)
    binaries=compile_references(args.qsrc,args.evidence)
    fixture=subprocess.check_output([args.probe,'--fixture'])
    (args.evidence/'fixture.bin').write_bytes(fixture)
    rows=list(records(fixture));reference=[None]*len(rows)
    groups={name:[] for name in binaries}
    for i,(fields,row) in enumerate(rows):
        mode=fields[0]
        group='nq' if mode<4 else ('qw-client' if mode==4 else 'qw-server') if mode<6 else 'q2' if mode<8 else 'q2pro' if mode<14 else 'q3'
        groups[group].append((i,row))
    for group,entries in groups.items():
        original=subprocess.check_output([binaries[group]],input=b''.join(row for _,row in entries))
        (args.evidence/f'{group}-packets.bin').write_bytes(original)
        outputs=list(results(original))
        if len(outputs)!=len(entries):raise ValueError('original record count')
        for (index,_),output in zip(entries,outputs):reference[index]=output
    rust=subprocess.check_output([args.probe,'--encode'],input=fixture)
    (args.evidence/'rust-packets.bin').write_bytes(rust)
    outputs=list(results(rust))
    if len(outputs)!=len(reference):raise ValueError('Rust record count')
    for i,(actual,expected) in enumerate(zip(outputs,reference)):
        if actual!=expected:raise ValueError(f'native packet mismatch case{i}, fields{rows[i][0]}, lengths{len(actual)}/{len(expected)}')
    report={'cases':len(rows),'layout_modes':18,'original_source_groups':len(groups),'native_bytes_exact':True,'original_functions':list(binaries),'scope':'header/body bytes from original transmit functions; no channel ACK/reliability/host/live acceptance'}
    (args.evidence/'comparison.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))


if __name__=='__main__':main()
