#!/usr/bin/env python3
"""Run the normal SDL3 host path on three owned private display backends."""
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
    args=parser.parse_args();args.evidence.mkdir(exist_ok=False,parents=True)
    records=[];core=pinned_cores()
    for backend in ['x11','sway','weston']:
        folder=args.evidence/backend
        r=run(args.binary,args.owner_profile,folder,
            ['--frames','600','--warmup','60','--startup-hold-ms','1000','--frame-timings','+set','developer','1','--commands','bind w "+forward;echo sdl3_hold"'],
            backend=backend,cores=core,stdin_bytes=b'echo sdl3_stdin\n',input_after_first_frame=True,
            actions=None if backend=='weston' else [{'key':'w','hold_seconds':1.25}])
        events=r.get('events',[]);rows=[e for e in events if e.get('event')=='system_event_frame' and e['frame']>=60]
        gates=[e for e in events if e.get('event')=='allocation_gate'];ready=next((e for e in events if e.get('event')=='window_ready'),{})
        exit=next((e for e in events if e.get('event')=='normal_exit'),{});times=[e['time_ns'] for e in rows]
        log=(folder/'runtime.log').read_text() if (folder/'runtime.log').exists() else ''
        outputs=[e for e in events if e.get('event')=='output_frame'];stdin=[e for e in events if e.get('event')=='stdin_frame']
        checks={'private_run':r['result']=='PASS','600_measured_frames':len(rows)==600,
            'monotonic_time':len(times)==600 and all(a<b for a,b in zip(times,times[1:])),
            'two_physical_host_drains':all(e['drains']==2 and e['queue_remaining']==0 and e['rejected']==0 for e in rows),
            'one_output_drain':len(outputs)==600 and all(e['drains']==1 and e['remaining']==0 for e in outputs),
            'stdin_ingress':bool(stdin) and stdin[-1]['lines']==1 and stdin[-1]['errors']==0 and any(s.strip()=='sdl3_stdin' for s in log.splitlines()),
            'selected_driver':ready.get('video_driver')==('x11' if backend=='x11' else 'wayland'),
            'preserved':r['owner_profile_unchanged'] and r['candidate_unchanged'],
            'owned_cleanup':r['remaining_owned_pids']==[]}
        if backend!='weston':
            checks.update(real_repeat=exit.get('key_repeats',0)>0,sustained_hold=sum(e['seat0_movement'][0]>0 for e in rows)>=30,
                released=bool(rows) and rows[-1]['seat0_movement']==[0,0,0],bound_echo=any(s.strip()=='sdl3_hold' for s in log.splitlines()))
        if gates:checks['zero_rust_allocations']=gates[-1]['frames']==600 and gates[-1]['passed'] and gates[-1]['requested_bytes']==0
        timing=next((e for e in events if e.get('event')=='frame_timings'),None)
        if timing:
            from frame_timings import summarize
            timing={name:summarize([row[i] for row in timing['samples_ns']]) for i,name in enumerate(['input','present','total'])}
        record={'backend':backend,'result':'PASS' if all(checks.values()) else 'FAIL','checks':checks,'normal_exit':exit,'allocation_gate':gates[-1] if gates else None,'shell_timings':timing,'error':r.get('error'),'scope':'SDL3 window shell; Weston input is ConsoleLine (no virtual keyboard protocol)'}
        records.append(record);(folder/'verification.json').write_text(json.dumps(record,indent=2)+'\n')
    report={'result':'PASS' if all(r['result']=='PASS' for r in records) else 'FAIL','core':core,'records':records,'gameplay_qualified':False}
    (args.evidence/'verification.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));return report['result']!='PASS'

if __name__=='__main__':raise SystemExit(main())
