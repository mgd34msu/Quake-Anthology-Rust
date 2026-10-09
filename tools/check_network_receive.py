#!/usr/bin/env python3
"""THE-860/949: compare native receive transcripts with unchanged qsrc functions."""
import argparse
import json
import pathlib
import random
import struct
import subprocess
from check_network_headers import function

ROOT = pathlib.Path(__file__).resolve().parents[1]
COMMON = r'''
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef unsigned char byte;
typedef int qboolean;
typedef int netadr_t;
typedef struct { byte *data; unsigned maxsize,cursize,readcount,bit; } sizebuf_t;
typedef sizebuf_t msg_t;
typedef struct { int value,integer; } cvar_t;
#define qtrue 1
#define qfalse 0
#define NS_CLIENT 0
#define NS_SERVER 1
#define MAX_LATENT 32
#define OLD_AVG 0.99
#define FRAGMENT_SIZE 1300
#define FRAGMENT_BIT (1u<<31)
#define USE_CLIENT 1
#define REL_BIT (1u<<31)
#define FRG_BIT (1u<<30)
#define OLD_MASK (REL_BIT-1)
#define NEW_MASK (FRG_BIT-1)
#define PROTOCOL_VERSION_R1Q2 35
#define LittleLong(x) (x)
#define Com_Memcpy memcpy
#define Q_memcpy memcpy
#define SHOWPACKET(...) ((void)0)
#define SHOWDROP(...) ((void)0)
#define Con_Printf(...) ((void)0)
#define Con_DPrintf(...) ((void)0)
#define Com_Printf(...) ((void)0)
#define NET_AdrToString(x) "matched private peer"
static cvar_t disabled;
static cvar_t *showpackets=&disabled,*showdrop=&disabled;
static char *netsrcString[2]={"client","server"};
static double realtime,net_time;
static unsigned curtime,com_localTime,net_drop;
static struct { int demoplayback; } cls;
static int net_from;
static bool NET_CompareAdr(int a,int b) { (void)a;(void)b;return true; }
static byte wire[65536],body[65536],fragment_data[32768];
static byte controls[256];static unsigned control_length;
static unsigned wire_length;
static sizebuf_t message;
#define net_message message
#define msg_read message
#define msg_read_buffer wire
static void begin_read(void) { message.readcount=message.bit=0; }
static uint32_t read_word(unsigned n) {
 uint32_t value=0;
 for(unsigned i=0;i<n;i++) {
  if(message.readcount>=message.cursize) { message.readcount+=n-i;return UINT32_MAX; }
  value|=(uint32_t)message.data[message.readcount++]<<(8*i);
 }
 return value;
}
#define MSG_BeginReading(...) begin_read()
#define MSG_BeginReadingOOB(...) begin_read()
#define MSG_ReadLong(...) read_word(4)
#define MSG_ReadShort(...) ((int16_t)read_word(2))
#define MSG_ReadWord(...) read_word(2)
#define MSG_ReadByte(...) read_word(1)
static void append(sizebuf_t *b,const void *p,unsigned n) {
 if(n>b->maxsize-b->cursize)abort();
 memcpy(b->data+b->cursize,p,n);b->cursize+=n;
}
#define SZ_Write append
#define SZ_Clear(b) do { (b)->cursize=(b)->readcount=0; } while(0)
static void init_read(sizebuf_t *b,byte *p,unsigned n) { *b=(sizebuf_t){.data=p,.maxsize=n,.cursize=n}; }
#define SZ_InitRead init_read
typedef struct {
 int sock,qport,protocol,remote_address,remoteAddress;
 unsigned incoming_sequence,incoming_acknowledged,outgoing_sequence;
 unsigned incoming_reliable_sequence,incoming_reliable_acknowledged,reliable_sequence;
 unsigned reliable_length,dropped,fragment_sequence,total_dropped,total_received;
 unsigned last_received;bool reliable_ack_pending;
 unsigned drop_count,good_count;double frame_latency,frame_rate;
 sizebuf_t fragment_in;
 int incomingSequence,fragmentSequence,fragmentLength;
 byte fragmentBuffer[16384];
} netchan_t;
static netchan_t chan;
static unsigned char mode;static uint16_t count;
static bool transcript(void) {
 if(fread(&mode,1,1,stdin)!=1)return false;
 if(fread(&count,2,1,stdin)!=1)abort();
 memset(&chan,0,sizeof(chan));
 chan.sock=mode%2;chan.qport=(mode/2==4 || mode/2==6)?0:37;
 chan.protocol=mode<6?34:35;
 chan.fragment_in=(sizebuf_t){.data=fragment_data,.maxsize=32768};net_drop=0;
 return true;
}
static void packet(void) {
 uint16_t n;
 if(fread(&n,2,1,stdin)!=1 || fread(wire,1,n,stdin)!=n)abort();
 wire_length=n;control_length=0;
 message=(sizebuf_t){.data=wire,.cursize=n,.maxsize=65536};
}
static void result(unsigned ready,unsigned sequence,unsigned next_datagram,unsigned ack,unsigned rack,
 unsigned reliable,unsigned fragment_sequence,unsigned fragment_bytes,unsigned dropped) {
 unsigned length=ready?message.cursize-message.readcount:0;
 uint32_t fields[]={ready,sequence,next_datagram,ack,rack,reliable,fragment_sequence,fragment_bytes,dropped,control_length,length};
 fwrite(fields,4,11,stdout);
 fwrite(message.data+message.readcount,1,length,stdout);fwrite(controls,1,control_length,stdout);
}
'''
NQ = r'''
#define MAX_DATAGRAM 1024
#define NET_MAXMESSAGE 8192
#define NET_DATAGRAMSIZE 8200
#define NET_HEADERSIZE 8
#define NETFLAG_LENGTH_MASK 0xffff
#define NETFLAG_DATA 0x00010000
#define NETFLAG_ACK 0x00020000
#define NETFLAG_NAK 0x00040000
#define NETFLAG_EOM 0x00080000
#define NETFLAG_UNRELIABLE 0x00100000
#define NETFLAG_CTL 0x80000000
#define BigLong(x) __builtin_bswap32((uint32_t)(x))
struct qsockaddr { int unused; };
typedef struct {
 unsigned receiveSequence,unreliableReceiveSequence,sendSequence,ackSequence;
 int receiveMessageLength,sendMessageLength;
 byte receiveMessage[8192],sendMessage[8192];
 bool canSend,sendNext;int socket;double lastSendTime;struct qsockaddr addr;
} qsocket_t;
static qsocket_t sock;
static struct { unsigned length,sequence;byte data[8192]; } packetBuffer;
static unsigned droppedDatagrams,shortPacketCount,packetsReceived,receivedDuplicateCount;
static int read_packet(int s,byte *data,int n,struct qsockaddr *a) {
 (void)s;(void)a;if(wire_length>(unsigned)n)abort();
 memcpy(data,wire,wire_length);unsigned length=wire_length;wire_length=0;return length;
}
static int write_packet(int s,byte *data,int n,struct qsockaddr *a) {
 (void)s;(void)a;if(n>(int)sizeof(controls)-(int)control_length)abort();
 memcpy(controls+control_length,data,n);control_length+=n;return n;
}
static int compare_address(struct qsockaddr *a,struct qsockaddr *b) { (void)a;(void)b;return 0; }
static struct { int (*Read)(int,byte *,int,struct qsockaddr *);int (*Write)(int,byte *,int,struct qsockaddr *);int (*AddrCompare)(struct qsockaddr *,struct qsockaddr *); } sfunc={read_packet,write_packet,compare_address};
static void ReSendMessage(qsocket_t *s) { (void)s;abort(); }
static void SendMessageNext(qsocket_t *s) { (void)s;abort(); }
'''
DRIVER = r'''
int main(void) {
 while(transcript()) {
  INITIALIZE
  for(unsigned index=0;index<count;index++) {
   packet();curtime=com_localTime=index;realtime=index;
   PROCESS
  }
 }
 return ferror(stdin)?1:0;
}
'''


def compile_references(qsrc, evidence):
    paths = {
        'nq': 'quake/WinQuake/net_dgrm.c',
        'qw-client': 'quake/QW/client/net_chan.c',
        'qw-server': 'quake/QW/client/net_chan.c',
        'q2': 'quake-2/qcommon/net_chan.c',
        'q2pro': 'q2repro/src/common/net/chan.c',
        'q3': 'quake-iii-arena/code/qcommon/net_chan.c',
    }
    binaries = {}
    for name, path in paths.items():
        text = (qsrc / path).read_text()
        source = COMMON
        initialize = ''
        if name == 'nq':
            source += NQ + function(text, 'Datagram_GetMessage')
            initialize = 'memset(&sock,0,sizeof(sock));sock.canSend=true;droppedDatagrams=0;unsigned last_drop=0;'
            process = '''
             message=(sizebuf_t){.data=body,.maxsize=sizeof(body)};
             unsigned before=droppedDatagrams;unsigned ready=Datagram_GetMessage(&sock);
             if(ready==2)last_drop=droppedDatagrams-before;
             result(ready,sock.receiveSequence,sock.unreliableReceiveSequence,0,0,0,0,sock.receiveMessageLength,last_drop);
            '''
        elif name.startswith('qw'):
            source += '#define showpackets disabled\n#define showdrop disabled\n'
            if name == 'qw-server': source += '#define SERVERONLY 1\n'
            source += function(text, 'Netchan_Process')
            process = 'unsigned ready=Netchan_Process(&chan);result(ready,chan.incoming_sequence,0,chan.incoming_acknowledged,chan.incoming_reliable_acknowledged,chan.incoming_reliable_sequence,0,0,net_drop);'
        elif name == 'q2':
            source += function(text, 'Netchan_Process')
            process = 'unsigned ready=Netchan_Process(&chan,&message);result(ready,chan.incoming_sequence,0,chan.incoming_acknowledged,chan.incoming_reliable_acknowledged,chan.incoming_reliable_sequence,0,0,chan.dropped);'
        elif name == 'q2pro':
            source += function(text, 'NetchanOld_Process') + function(text, 'NetchanNew_Process')
            process = 'unsigned ready=mode<10?NetchanOld_Process(&chan):NetchanNew_Process(&chan);result(ready,chan.incoming_sequence,0,chan.incoming_acknowledged,chan.incoming_reliable_acknowledged,chan.incoming_reliable_sequence,chan.fragment_sequence,chan.fragment_in.cursize,chan.dropped);'
        else:
            source += function(text, 'Netchan_Process')
            process = 'unsigned ready=Netchan_Process(&chan,&message);result(ready,chan.incomingSequence,0,0,0,0,chan.fragmentSequence,chan.fragmentLength,chan.dropped);'
        source += DRIVER.replace('INITIALIZE', initialize).replace('PROCESS', process)
        file = evidence / f'original-{name}.c';file.write_text(source)
        binary = evidence / f'original-{name}'
        subprocess.run(['cc', '-O2', '-std=c11', str(file), '-o', str(binary)], check=True)
        binaries[name] = binary
    return binaries


def native_packet(mode, sequence, payload, ack=0, reliable=False, rack=False, flags=0, fragment=None):
    if mode < 2:
        return struct.pack('>II', flags | (8 + len(payload)), sequence) + payload
    q3 = mode >= 14
    new = 10 <= mode < 14
    first = sequence | (0x80000000 if reliable and not q3 else 0)
    if fragment is not None: first |= 0x80000000 if q3 else 0x40000000
    header = struct.pack('<I', first)
    if not q3: header += struct.pack('<I', ack | (0x80000000 if rack else 0))
    if mode % 2:
        if mode in (3, 5, 15): header += struct.pack('<H', 37)
        elif mode in (7, 11): header += bytes([37])
    if fragment is not None:
        offset, more = fragment
        header += struct.pack('<HH', offset, len(payload)) if q3 else struct.pack('<H', offset | (0x8000 if more else 0))
    if fragment is not None and not (new or q3): raise ValueError('unsupported fixture fragmentation')
    return header + payload


def transcripts():
    for mode in range(16):
        for seed in range(128):
            rng = random.Random(seed * 17 + mode)
            body = rng.randbytes(1300)
            ack = rng.randrange(0, 10000)
            def packet(sequence, payload=b'', **kwargs):
                return native_packet(mode, sequence, payload, **kwargs)
            if mode < 2:
                packets = [
                    packet(0, body[:100], flags=0x10000),
                    packet(0, body[:100], flags=0x10000),
                    packet(2, body[:20], flags=0x90000),
                    packet(0, body[:7], flags=0x100000),
                    packet(2, body[:17], flags=0x100000),
                    packet(1, body[:3], flags=0x100000),
                    packet(1, body[:200], flags=0x90000),
                    packet(1, body[:200], flags=0x90000),
                    packet(2, b'', flags=0x90000),
                    packet(0, flags=0x20000),
                    packet(3, body[:9], flags=0x120000),
                    packet(3, body[:19], flags=0x90000),
                ]
            elif mode < 10:
                packets = [packet(seq, body[:length], ack=ack+i, reliable=bool(i%2), rack=bool(i%3))
                    for i, (seq, length) in enumerate([(0, 0), (1, 100), (1, 200), (3, 0), (2, 17), (4, 255), (8, 1300), (9, 1), (8, 3), (10, 1300)])]
            else:
                packets = [
                    packet(1, body[:17], ack=ack, reliable=True, rack=True),
                    packet(2, body, ack=ack+1, reliable=True, rack=True, fragment=(0, True)),
                    packet(2, body, ack=ack+2, rack=False, fragment=(0, True)),
                    packet(2, body[:99], ack=ack+3, rack=True, fragment=(1400, False)),
                    packet(2, body[:99], ack=ack+4, reliable=True, fragment=(1300, False)),
                    packet(2, body[:17], ack=ack+5, rack=True),
                    packet(4, body, ack=ack+6, reliable=True, fragment=(0, True)),
                    packet(5, body[:100], ack=ack+7, rack=True, fragment=(100, False)),
                    packet(5, body, ack=ack+8, rack=True, fragment=(0, True)),
                    packet(5, b'', ack=ack+9, reliable=True, fragment=(1300, False)),
                    packet(6, body[:31], ack=ack+10, reliable=True, rack=True),
                    packet(7, body[:53], ack=ack+11, fragment=(0, False)),
                ]
            record = struct.pack('<BH', mode, len(packets)) + b''.join(struct.pack('<H', len(p)) + p for p in packets)
            group = 'nq' if mode < 2 else ('qw-client' if mode == 2 else 'qw-server') if mode < 4 else 'q2' if mode < 6 else 'q2pro' if mode < 14 else 'q3'
            yield group, record, len(packets)


def rows(data):
    while data:
        if len(data) < 44: raise ValueError('truncated native result')
        fields = struct.unpack_from('<11I', data)
        length = 44 + fields[9] + fields[10]
        if length > len(data): raise ValueError('truncated native body/control')
        yield data[:length]
        data = data[length:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=pathlib.Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=pathlib.Path, required=True)
    parser.add_argument('--probe', type=pathlib.Path, required=True)
    args = parser.parse_args();args.evidence.mkdir(parents=True, exist_ok=True)
    binaries = compile_references(args.qsrc, args.evidence)
    groups = {name: [] for name in binaries}
    count = 0
    for group, record, packets in transcripts():
        groups[group].append(record);count += packets
    compared = 0
    for group, records in groups.items():
        fixture = b''.join(records);(args.evidence / f'{group}-fixture.bin').write_bytes(fixture)
        original = subprocess.check_output([binaries[group]], input=fixture)
        rust = subprocess.check_output([args.probe, '--compare'], input=fixture)
        (args.evidence / f'{group}-original.bin').write_bytes(original)
        (args.evidence / f'{group}-rust.bin').write_bytes(rust)
        originals, actuals = list(rows(original)), list(rows(rust))
        if len(originals) != len(actuals): raise ValueError(f'{group} result count')
        for i, (a, b) in enumerate(zip(actuals, originals)):
            if a != b: raise ValueError(f'{group} row {i} differs: Rust {struct.unpack_from("<11I", a)}, original {struct.unpack_from("<11I", b)}')
        compared += len(originals)
    if compared != count: raise ValueError('native packet count')
    report = {'transcripts': sum(map(len, groups.values())), 'packet_rows': compared, 'policy_directions': 16, 'original_source_groups': len(groups), 'payload_state_and_ack_reply_bytes_exact': True, 'scope': 'connected receive state and ordered assembly; no transmit ACK validation, reliable retirement, handshake, host or live connection acceptance'}
    (args.evidence / 'comparison.json').write_text(json.dumps(report, indent=2) + '\n');print(json.dumps(report))


if __name__ == '__main__': main()
