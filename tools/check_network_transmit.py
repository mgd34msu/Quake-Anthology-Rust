#!/usr/bin/env python3
"""THE-860/949 original send/ACK/fragment transcripts, compiled only as tooling."""
import argparse
import json
import pathlib
import random
import re
import struct
import subprocess
from check_network_headers import COMMON as SEND_BINDINGS, function
from check_network_receive import NQ as RECEIVE_DATAGRAM_BINDINGS, native_packet

ROOT = pathlib.Path(__file__).resolve().parents[1]
RECEIVE_BINDINGS = r'''
static byte wire[65536],body[65536],incoming_fragment[32768];
static unsigned wire_length;
static sizebuf_t incoming;
#define net_message incoming
#define msg_read incoming
#define msg_read_buffer wire
static unsigned net_drop;
static int net_from;
#define OLD_AVG 0.99
#define LittleLong(x) (x)
#define Con_DPrintf(...) ((void)0)
static bool NET_CompareAdr(int a,int b) { (void)a;(void)b;return true; }
static void begin_read(void) { incoming.readcount=incoming.bit=0; }
static uint32_t read_word(unsigned n) {
 uint32_t value=0;
 for(unsigned i=0;i<n;i++) {
  if(incoming.readcount>=incoming.cursize) { incoming.readcount+=n-i;return UINT32_MAX; }
  value|=(uint32_t)incoming.data[incoming.readcount++]<<(8*i);
 }
 return value;
}
#define MSG_BeginReading(...) begin_read()
#define MSG_BeginReadingOOB(...) begin_read()
#define MSG_ReadLong(...) read_word(4)
#define MSG_ReadShort(...) ((int16_t)read_word(2))
#define MSG_ReadWord(...) read_word(2)
#define MSG_ReadByte(...) read_word(1)
static void init_read(sizebuf_t *b,byte *p,size_t n) { *b=(sizebuf_t){.data=p,.maxsize=n,.cursize=n}; }
#define SZ_InitRead init_read
'''
DRIVER = r'''
static netchan_t chan;
static unsigned sent_packets;
static byte input[32768],pending_data[8192];
static byte mode;
static sizebuf_t queued_datagram;
static uint16_t count;
static bool transcript(void) {
 if(fread(&mode,1,1,stdin)!=1)return false;
 if(fread(&count,2,1,stdin)!=1)abort();
 memset(&chan,0,sizeof(chan));
 chan.sock=mode%2;chan.qport=(mode/2==4 || mode/2==6)?0:37;
 chan.protocol=mode<6?34:35;chan.type=1;
 chan.outgoing_sequence=chan.outgoingSequence=1;chan.maxpacketlen=1300;
 chan.message=(sizebuf_t){.data=chan.message_buf,.maxsize=sizeof(chan.message_buf)};
 chan.fragment_out=(sizebuf_t){.data=fragment_data,.maxsize=32768};
 chan.fragment_in=(sizebuf_t){.data=incoming_fragment,.maxsize=32768};
 queued_datagram=(sizebuf_t){.data=pending_data,.maxsize=8192};
 port.integer=port.value=cls.qport=37;captured=sent_packets=0;
 INITIALIZE
 return true;
}
static void result(uint32_t sequence,uint32_t datagram,uint32_t ack,uint32_t reliable,
 uint32_t last_reliable,uint32_t reliable_bytes,uint32_t fragment_bytes,uint32_t fragment_offset) {
 uint32_t fields[]={sequence,datagram,ack,reliable,last_reliable,reliable_bytes,fragment_bytes,fragment_offset,sent_packets,(uint32_t)captured};
 fwrite(fields,4,10,stdout);fwrite(capture,1,captured,stdout);
}
int main(void) {
 while(transcript()) {
  for(unsigned index=0;index<count;index++) {
   byte operation,present;uint32_t milliseconds;uint16_t n;
   if(fread(&operation,1,1,stdin)!=1 || fread(&milliseconds,4,1,stdin)!=1 || fread(&present,1,1,stdin)!=1 || fread(&n,2,1,stdin)!=1 || fread(input,1,n,stdin)!=n)abort();
   curtime=com_localTime=milliseconds;realtime=net_time=milliseconds/1000.0;captured=0;
   incoming=(sizebuf_t){.data=wire,.maxsize=sizeof(wire),.cursize=n};memcpy(wire,input,n);wire_length=n;
   PROCESS
   RESULT
  }
 }
 return ferror(stdin)?1:0;
}
'''


def bindings():
    code = SEND_BINDINGS[:SEND_BINDINGS.index('static byte mode,role')]
    code = code.replace('int overflowed;', 'int overflowed,bit;')
    code = code.replace(' int outgoingSequence,incomingSequence,unsentFragmentStart,unsentLength,unsentFragments;', '''
 unsigned dropped,total_dropped,total_received,fragment_sequence,drop_count,good_count;
 double last_received,frame_rate,frame_latency;
 sizebuf_t fragment_in;
 int outgoingSequence,incomingSequence,unsentFragmentStart,unsentLength,unsentFragments,fragmentSequence,fragmentLength;
 byte fragmentBuffer[16384];''')
    code = re.sub(r'static void save\([^\n]*\n', '''static unsigned sent_packets;
static void save(const void *data,size_t length) {
 if(captured+length+4>sizeof(capture))abort();
 uint32_t n=(uint32_t)length;memcpy(capture+captured,&n,4);memcpy(capture+captured+4,data,length);captured+=4+length;sent_packets++;
}
''', code)
    return code + RECEIVE_BINDINGS


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
        original = (qsrc / path).read_text();code = bindings();initialize = ''
        if name == 'nq':
            nq = RECEIVE_DATAGRAM_BINDINGS.replace('sendSequence,ackSequence;', 'sendSequence,ackSequence,unreliableSendSequence;')
            nq = nq.replace('static void ReSendMessage(qsocket_t *s) { (void)s;abort(); }\n', '')
            nq = nq.replace('static void SendMessageNext(qsocket_t *s) { (void)s;abort(); }\n', '')
            nq = re.sub(r'static int write_packet\([^\n]*\n.*?\n}\n', '''static int write_packet(int s,byte *data,int n,struct qsockaddr *a) {
 (void)s;(void)a;save(data,n);return n;
}
''', nq, flags=re.S)
            code += nq + '\nstatic unsigned packetsSent,packetsReSent;\n'
            for fn in ['Datagram_SendMessage', 'SendMessageNext', 'ReSendMessage', 'Datagram_SendUnreliableMessage', 'Datagram_GetMessage']:
                code += function(original, fn)
            initialize = 'memset(&sock,0,sizeof(sock));sock.canSend=true;'
            process = '''
             if(operation==0)SZ_Write(&queued_datagram,input,n);
             else if(operation==1) {
              sizebuf_t data={.data=input,.cursize=n};
              if(sock.canSend && queued_datagram.cursize) { Datagram_SendMessage(&sock,&queued_datagram);SZ_Clear(&queued_datagram); }
              else if(sock.sendNext)SendMessageNext(&sock);
              else if(!sock.canSend && net_time-sock.lastSendTime>1.0)ReSendMessage(&sock);
              else if(present)Datagram_SendUnreliableMessage(&sock,&data);
             } else { incoming=(sizebuf_t){.data=body,.maxsize=sizeof(body)};Datagram_GetMessage(&sock); }
            '''
            result = 'result(sock.sendSequence,sock.unreliableSendSequence,sock.ackSequence,0,0,sock.sendMessageLength,0,0);'
        elif name.startswith('qw'):
            code += '#define showpackets disabled\n#define showdrop disabled\n'
            if name == 'qw-server': code += '#define SERVERONLY 1\n'
            code += 'static void NET_SendPacket(int n,void *p,int a) { (void)a;save(p,n); }\n'
            code += function(original, 'Netchan_Transmit') + function(original, 'Netchan_Process')
            process = 'if(operation==0)SZ_Write(&chan.message,input,n);else if(operation==1)Netchan_Transmit(&chan,n,input);else Netchan_Process(&chan);'
            result = 'result(chan.outgoing_sequence,0,0,chan.reliable_sequence,chan.last_reliable_sequence,chan.reliable_length,0,0);'
        elif name == 'q2':
            code = code.replace('#define MAX_MSGLEN 1450', '#define MAX_MSGLEN 1400')
            code += 'static void NET_SendPacket(int s,int n,void *p,int a) { (void)s;(void)a;save(p,n); }\n'
            code += function(original, 'Netchan_NeedReliable') + function(original, 'Netchan_Transmit') + function(original, 'Netchan_Process')
            process = 'if(operation==0)SZ_Write(&chan.message,input,n);else if(operation==1)Netchan_Transmit(&chan,n,input);else Netchan_Process(&chan,&incoming);'
            result = 'result(chan.outgoing_sequence,0,0,chan.reliable_sequence,chan.last_reliable_sequence,chan.reliable_length,0,0);'
        elif name == 'q2pro':
            code = code.replace('#define MAX_MSGLEN 1450', '#define MAX_MSGLEN 32768')
            code += 'static void NET_SendPacket(int s,void *p,size_t n,const int *a) { (void)s;(void)a;save(p,n); }\n'
            for fn in ['Netchan_TransmitNextFragment', 'NetchanOld_Transmit', 'NetchanNew_Transmit', 'NetchanOld_Process', 'NetchanNew_Process']:
                code += function(original, fn)
            process = '''
             if(operation==0)SZ_Write(&chan.message,input,n);
             else if(operation==1) { if(mode<10)NetchanOld_Transmit(&chan,n,input,1);else NetchanNew_Transmit(&chan,n,input,1); }
             else { if(mode<10)NetchanOld_Process(&chan);else NetchanNew_Process(&chan); }
            '''
            result = 'result(chan.outgoing_sequence,0,0,chan.reliable_sequence,chan.last_reliable_sequence,chan.reliable_length,chan.fragment_out.cursize,chan.fragment_out.readcount);'
        else:
            code = code.replace('#define MAX_MSGLEN 1450', '#define MAX_MSGLEN 16384').replace('#define MAX_PACKETLEN 4096', '#define MAX_PACKETLEN 1400')
            code += 'static void NET_SendPacket(int s,int n,void *p,int a) { (void)s;(void)a;save(p,n); }\n'
            code += function(original, 'Netchan_TransmitNextFragment') + function(original, 'Netchan_Transmit') + function(original, 'Netchan_Process')
            process = 'if(operation==1) { if(chan.unsentFragments)Netchan_TransmitNextFragment(&chan);else Netchan_Transmit(&chan,n,input); } else if(operation==2)Netchan_Process(&chan,&incoming);else abort();'
            result = 'result(chan.outgoingSequence,0,0,0,0,0,chan.unsentFragments?chan.unsentLength:0,chan.unsentFragmentStart);'
        code += DRIVER.replace('INITIALIZE', initialize).replace('PROCESS', process).replace('RESULT', result)
        file = evidence / f'original-{name}.c';file.write_text(code)
        binary = evidence / f'original-{name}'
        subprocess.run(['cc', '-O2', '-std=c11', str(file), '-o', str(binary)], check=True)
        binaries[name] = binary
    return binaries


def transcripts():
    for mode in range(16):
        for seed in range(64):
            rng = random.Random(mode * 100 + seed);body = rng.randbytes(3900)
            operations = []
            def op(kind, time, data=b'', present=False):
                operations.append(struct.pack('<BIBH', kind, time, present, len(data)) + data)
            def ack(seq, acknowledged=0, rack=False, time=0):
                data = native_packet(mode, seq, b'', ack=acknowledged, rack=rack,
                    flags=0x20000 if mode < 2 else 0)
                op(2, time, data)
            if mode < 2:
                op(0, 0, body[:100]);op(0, 0, body[100:150]);op(1, 0);ack(0, time=100)
                op(0, 200, body[:2600]);op(1, 200);ack(1, time=300);ack(1, time=310)
                ack(2, time=400);ack(3, time=500)
                op(1, 600, body[:17], True)
                op(0, 700, body[:127]);op(1, 700)
                op(1, 1600, body[:31], True);op(1, 1701);ack(4, time=1800);ack(3, time=1900)
                op(1, 2000, b'', True)
            elif mode < 14:
                op(0, 0, body[:256]);op(1, 0, body[:17], True)
                ack(1, 2, False, 100);op(1, 200)
                ack(2, 9, False, 300);op(1, 400)
                ack(2, 999, True, 410);ack(3, 3, True, 500)
                op(0, 600, body[:23]);op(1, 600, body[:2600] if mode >= 10 else body[:127], True)
                if mode >= 10: op(1, 610);op(1, 620)
                ack(4, 4, False, 700);op(1, 800)
            else:
                op(1, 0, body[:1299], True)
                op(1, 100, body[:2600], True);op(1, 200);op(1, 300)
                op(1, 400, body[:17], True)
                op(1, 500, body[:3900], True);op(1, 600);op(1, 700);op(1, 800)
                op(1, 900, b'', True)
            record = struct.pack('<BH', mode, len(operations)) + b''.join(operations)
            group = 'nq' if mode < 2 else ('qw-client' if mode == 2 else 'qw-server') if mode < 4 else 'q2' if mode < 6 else 'q2pro' if mode < 14 else 'q3'
            yield group, record, len(operations)


def rows(data):
    while data:
        if len(data) < 40: raise ValueError('truncated native state')
        fields = struct.unpack_from('<10I', data);length = 40 + fields[-1]
        if length > len(data): raise ValueError('truncated native packets')
        yield data[:length]
        data = data[length:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=pathlib.Path, default=ROOT.parent / 'qsrc')
    parser.add_argument('--evidence', type=pathlib.Path, required=True)
    parser.add_argument('--probe', type=pathlib.Path, required=True)
    args = parser.parse_args();args.evidence.mkdir(parents=True, exist_ok=True)
    binaries = compile_references(args.qsrc, args.evidence)
    groups = {name: [] for name in binaries};count = 0
    for group, record, operations in transcripts(): groups[group].append(record);count += operations
    compared = 0
    for group, records in groups.items():
        fixture = b''.join(records);(args.evidence / f'{group}-fixture.bin').write_bytes(fixture)
        original = subprocess.check_output([binaries[group]], input=fixture)
        rust = subprocess.check_output([args.probe, '--compare'], input=fixture)
        (args.evidence / f'{group}-original.bin').write_bytes(original)
        (args.evidence / f'{group}-rust.bin').write_bytes(rust)
        expected, actual = list(rows(original)), list(rows(rust))
        if len(expected) != len(actual): raise ValueError(f'{group} row count')
        for i, (a, b) in enumerate(zip(actual, expected)):
            if a != b: raise ValueError(f'{group} row{i}: Rust {struct.unpack_from("<10I",a)}, original {struct.unpack_from("<10I",b)}')
        compared += len(expected)
    if compared != count: raise ValueError('native transcript count')
    report = {'transcripts': sum(map(len, groups.values())), 'operation_rows': compared, 'policy_directions': 16, 'original_source_groups': len(groups), 'outgoing_packet_bytes_and_native_flight_state_exact': True, 'scope': 'queued native message stream, send/receive ACK, resend and fragment sequences; no Q3 command ACK/XOR, transport rejection original proof, handshake, host or live interoperability'}
    (args.evidence / 'comparison.json').write_text(json.dumps(report, indent=2) + '\n');print(json.dumps(report))


if __name__ == '__main__': main()
