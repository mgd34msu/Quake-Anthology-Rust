#!/usr/bin/env python3
"""Compare native SERVER address/qport routing blocks; no socket/gameplay proof."""
import argparse
import json
from pathlib import Path
import random
import struct
import subprocess
from check_hull_trace import function

ROOT = Path(__file__).resolve().parents[1]


def loop(source, marker):
    start = source.index(marker)
    brace = source.index('{', start)
    end, depth = brace + 1, 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


def compile_reference(qsrc, evidence, family):
    refs = [
        ('quake/QW/server/sv_main.c', 'SV_ReadPackets', 'quake/QW/client/net_udp.c'),
        ('quake-2/server/sv_main.c', 'SV_ReadPackets', 'quake-2/linux/net_udp.c'),
        ('quake-iii-arena/code/server/sv_main.c', 'SV_PacketEvent', 'quake-iii-arena/code/qcommon/net_chan.c'),
        ('q2repro/src/server/main.c', 'SV_PacketEvent', 'q2repro/inc/common/net/net.h'),
    ]
    path, name, net = refs[family]
    native = function((qsrc / path).read_text(), name)
    ip = 'union {uint8_t u8[16]; uint32_t u32[4];} ip' if family == 3 else 'uint8_t ip[4]'
    source = r'''
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>
typedef uint8_t byte;
typedef int qboolean;
#define qfalse false
#define qtrue true
enum {NA_LOOPBACK, NA_IP, NA_IPX, NA_IP6};
'''
    source += 'typedef struct {int type;' + ip + ';byte ipx[10];uint16_t port;} netadr_t;\n'
    source += r'''
typedef struct {netadr_t remote_address,remoteAddress;int qport,dropped;} netchan_t;
typedef struct {netchan_t netchan;int state,lastmessage,lastPacketTime,send_message,frameflags;char name[1];} client_t;
static struct {client_t clients[2];int realtime,time;struct {int packets;} stats;} svs;
typedef struct {byte *data;int cursize,readcount;} msg_t;
static msg_t net_message,msg_read;
static netadr_t net_from;
static struct {int value,integer;} limit={2,2};
static int selected;
#define MAX_CLIENTS 2
#define maxclients (&limit)
#define sv_maxclients (&limit)
#define cs_free 0
#define CS_FREE 0
#define cs_zombie 3
#define CS_ZOMBIE 3
#define PACKET_HEADER 10
#define FF_CLIENTDROP 1
#define FOR_EACH_CLIENT(c) for(c=svs.clients;c<svs.clients+2;c++)
#define Com_Printf(...) ((void)0)
#define Com_DPrintf(...) ((void)0)
#define Con_DPrintf(...) ((void)0)
#define NET_IsLocalAddress(a) ((a)->type == NA_LOOPBACK)
static void Sys_Error(char *s,...) {(void)s;abort();}
static void Com_Error(int n,char *s,...) {(void)n;(void)s;abort();}
#define ERR_FATAL 0
static int read_word(int n) {int v=0;for(int i=0;i<n;i++)v|=(int)net_message.data[net_message.readcount++]<<(i*8);return v;}
#define MSG_BeginReading(...) (net_message.readcount=0)
#define MSG_BeginReadingOOB(...) (net_message.readcount=0)
#define MSG_ReadLong(...) read_word(4)
#define MSG_ReadShort(...) read_word(2)
static int Netchan_Process(netchan_t *n,...) {selected=(int)(((client_t *)((byte*)n - offsetof(client_t,netchan)))-svs.clients);return 0;}
static int SV_Netchan_Process(client_t *c,msg_t *m) {(void)m;selected=(int)(c-svs.clients);return 0;}
#define SV_ExecuteClientMessage(...) ((void)0)
'''
    if family == 3:
        source += function((qsrc / net).read_text(), 'NET_IsEqualBaseAdr') + '\n'
        body = loop(native, 'FOR_EACH_CLIENT(client)')
        source += 'static void route(void) {client_t *client;netchan_t *netchan;int qport;\n' + body + '\n}\n'
    else:
        source += function((qsrc / net).read_text(), 'NET_CompareBaseAdr') + '\n'
        start = native.index('// read the qport')
        end_loop = loop(native[start:], 'for (i=0, cl=svs.clients')
        end = native.index(end_loop, start) + len(end_loop)
        body = native[start:end]
        source += 'static void route(void) {int i,qport,good=0;client_t *cl;netadr_t from=net_from;msg_t *msg=&net_message;\n' + body + '\n}\n'
    ipfield = '.ip.u8' if family == 3 else '.ip'
    source += r'''
int main(void) {
 uint8_t mode,ip[3][4];uint16_t ports[2],qports[2],from_port,n;uint32_t packet[350];
 while(fread(&mode,1,1,stdin)==1) {
  if(fread(ip[0],1,4,stdin)!=4||fread(ip[1],1,4,stdin)!=4||fread(ports,2,2,stdin)!=2||fread(qports,2,2,stdin)!=2||fread(ip[2],1,4,stdin)!=4||fread(&from_port,2,1,stdin)!=1||fread(&n,2,1,stdin)!=1||n>1400||fread(packet,1,n,stdin)!=n)return 2;
  memset(&svs,0,sizeof(svs));memset(&net_from,0,sizeof(net_from));
'''
    source += f'  net_from.type=NA_IP;memcpy(net_from{ipfield},ip[2],4);net_from.port=from_port;\n'
    source += f'  for(int i=0;i<2;i++){{svs.clients[i].state=1;svs.clients[i].netchan.qport=qports[i];svs.clients[i].netchan.remote_address.type=NA_IP;memcpy(svs.clients[i].netchan.remote_address{ipfield},ip[i],4);svs.clients[i].netchan.remote_address.port=ports[i];svs.clients[i].netchan.remoteAddress=svs.clients[i].netchan.remote_address;}}\n'
    source += r'''
  net_message=(msg_t){(byte*)packet,n,0};msg_read=net_message;selected=-1;route();
  uint32_t value=(uint32_t)selected;fwrite(&value,4,1,stdout);
'''
    field = 'remoteAddress' if family == 2 else 'remote_address'
    source += f'  for(int i=0;i<2;i++)fwrite(&svs.clients[i].netchan.{field}.port,2,1,stdout);\n' + ' }return ferror(stdin)?3:0;\n}\n'
    code = evidence / f'original-peer-routes-{family}.c'
    code.write_text(source)
    binary = evidence / f'original-peer-routes-{family}'
    subprocess.run(['cc', '-O2', '-std=c11', str(code), '-o', str(binary)], check=True)
    return binary


def fixture(mode):
    rand = random.Random(949860 + mode)
    data = bytearray()
    for case in range(512):
        addresses = [bytes([10, 23, 17, 1]), bytes([10, 23, 17, 1 + int(case % 7 == 0)])]
        ports = [20000, 20001]
        qports = [0, 0] if mode in (4, 6) else ([17, 23] if mode in (3, 5) else [rand.randrange(256,65536), 23])
        if case < 4 and mode in (0, 1, 2): qports[0] = [0,255,32768,65535][case]
        if case < 2 and mode in (3, 5): qports[0] = [1,255][case]
        chosen = case % 2
        source_ip = addresses[chosen] if case % 5 else bytes([10, 23, 19, 1])
        source_port = ports[chosen] if case % 3 == 0 or mode in (4, 6) else 30000 + case
        if mode in (4, 6) and case % 5 == 1: source_port += 1000
        sent_qport = qports[chosen] if case % 11 else 29
        if mode == 2:
            packet = struct.pack('<IH', 1, sent_qport)
        else:
            packet = struct.pack('<II', 1, 0)
            if mode in (0, 1): packet += struct.pack('<H', sent_qport)
            elif mode in (3, 5): packet += struct.pack('<B', sent_qport)
        packet += b'native'
        data += bytes([mode]) + b''.join(addresses) + struct.pack('<4H', *ports, *qports) + source_ip + struct.pack('<HH', source_port, len(packet)) + packet
    return bytes(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--probe', type=Path, default=ROOT / 'target/release/examples/peer_routes')
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=False)
    refs = [compile_reference(args.qsrc,args.evidence,family) for family in range(4)]
    data, expected = bytearray(), bytearray()
    for mode in range(7):
        rows = fixture(mode); data += rows
        expected += subprocess.check_output([str(refs[min(mode,3)])],input=rows)
    actual = subprocess.run([str(args.probe)],input=data,stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=True)
    (args.evidence/'fixture.bin').write_bytes(data)
    (args.evidence/'original.bin').write_bytes(expected)
    (args.evidence/'rust.bin').write_bytes(actual.stdout)
    (args.evidence/'probe.json').write_bytes(actual.stderr)
    if expected != actual.stdout:
        at=next((i for i,(a,b) in enumerate(zip(expected,actual.stdout)) if a!=b),min(len(expected),len(actual.stdout)))
        raise ValueError(f'native peer routing differs at row {at//8}, byte {at}; lengths {len(expected)}/{len(actual.stdout)}')
    result=dict(result='PASS',cases=3584,formats=7,selected_peer_and_ports_exact=True,
        scope='unchanged QW/Q2/Q3/q2repro SERVER routing blocks and native base-address predicates, IPv4; cold structs/IO and a channel-admission stop; no handshake, OS sockets, module ABI or gameplay',
        limits='valid native headers; Rust truncated/connectionless rejection and binding ambiguity are separate bounded fixtures',timing_run=False)
    (args.evidence/'comparison.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
