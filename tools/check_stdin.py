#!/usr/bin/env python3
"""Check real nonblocking pipe commands on a copied private candidate."""
import argparse
import json
from pathlib import Path
from frame_timings import pinned_cores
from private_run import run

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--owner-profile',type=Path,required=True)
    parser.add_argument('--evidence',type=Path,required=True)
    parser.add_argument('--tty',action='store_true',help='use an owned canonical PTY instead of an EOF pipe')
    args=parser.parse_args()
    # This is generated test command data delivered to actual stdin, not an
    # engine-side input player. An oversized command must never execute a prefix.
    payload=b'\r\nset sensitivity "echo stdin_ack"'+b' '*5000+b'\r\n' + b'echo dropped_prefix;'+b'x'*9000+b'\n' + b'echo bad_utf8_\xff\n' + b'vstr sensitivity'
    if args.tty:payload=b'set sensitivity "echo stdin_ack"\nvstr sensitivity\n'
    result=run(args.binary,args.owner_profile,args.evidence,
        ['--frames','600','--startup-hold-ms','1000','+set','developer','1'],
        cores=pinned_cores(),stdin_bytes=payload,stdin_tty=args.tty)
    events=result.get('events',[])
    rows=[r for r in events if r.get('event')=='stdin_frame']
    outputs=[r for r in events if r.get('event')=='output_frame']
    log=(args.evidence/'runtime.log').read_text()
    checks={'private_run':result['result']=='PASS','600_measured_frames':len(rows)==600,
        'two_complete_commands':bool(rows) and rows[-1]['lines']==2,
        'discard_count_matches':bool(rows) and rows[-1]['discarded']==(0 if args.tty else 2),
        'no_source_errors':bool(rows) and rows[-1]['errors']==0,
        'vstr_echo':any(line.strip()=='stdin_ack' for line in log.splitlines()),
        'no_partial_prefix_execution':'dropped_prefix' not in log,
        'one_output_drain':len(outputs)==600 and all(r['drains']==1 and r['remaining']==0 for r in outputs),
        'profile_and_candidate_preserved':result['owner_profile_unchanged'] and result['candidate_unchanged'],
        'owned_pids_cleaned':result['remaining_owned_pids']==[]}
    gates=[r for r in events if r.get('event')=='allocation_gate']
    if gates:checks['allocation_gate']=gates[-1]['passed'] and gates[-1]['frames']==600 and gates[-1]['requested_bytes']==0
    report={'result':'PASS' if all(checks.values()) else 'FAIL','checks':checks,
        'scope':'stdin -> system ConsoleLine -> shared cvar/vstr -> output ring, window shell only',
        'stdin_bytes':len(payload),'stdin_source':'owned canonical PTY' if args.tty else 'owned pipe with final EOF line',
        'final_stdin':rows[-1] if rows else None,
        'allocation_gate':gates[-1] if gates else None,'gameplay_reached':result['gameplay_reached']}
    (args.evidence/'verification.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))
    return report['result']!='PASS'

if __name__=='__main__':raise SystemExit(main())
