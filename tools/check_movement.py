#!/usr/bin/env python3
"""Compare the shared movement entry with original Q2/Q3 Pmove on analytic fixtures."""
import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_TARGET_DIR='target')
    started = time.monotonic()
    build = subprocess.run(['cargo','build','--release','-p','qa-movement','--example','native_fixture'], cwd=ROOT, env=env, capture_output=True, text=True, timeout=300)
    (args.output/'build.log').write_text(build.stdout+build.stderr)
    build.check_returncode()
    build_seconds = time.monotonic()-started
    results = {}
    for game, folder, sources in [
        ('q2','quake-2',['qcommon/pmove.c','game/q_shared.c']),
        ('q3','quake-iii-arena',['game/bg_pmove.c','game/bg_slidemove.c','game/bg_misc.c','game/q_shared.c','game/q_math.c']),
    ]:
        base=args.qsrc/folder
        if game=='q3': base=base/'code'
        executable=args.output/f'{game}-original'
        cc=['cc','-std=c99','-O2','-ffp-contract=off','-fno-strict-aliasing','-ffunction-sections','-fdata-sections','-include','strings.h','-I',str(base/'game'),str(ROOT/f'tools/probes/movement_{game}.c'),*[str(base/s) for s in sources],'-Wl,--gc-sections','-lm','-o',str(executable)]
        compiled=subprocess.run(cc,capture_output=True,text=True,timeout=300)
        (args.output/f'{game}-compile.log').write_text(compiled.stdout+compiled.stderr)
        compiled.check_returncode()
        expected=subprocess.check_output([str(executable)],text=True,timeout=300)
        actual=subprocess.check_output([str(ROOT/'target/release/examples/native_fixture'),*(['q3']if game=='q3'else[])],text=True,timeout=300)
        (args.output/f'{game}-original.txt').write_text(expected)
        (args.output/f'{game}-rust.txt').write_text(actual)
        a=expected.splitlines();b=actual.splitlines()
        if len(a)!=1152 or len(b)!=len(a):raise RuntimeError(f'{game}: row count mismatch')
        maximum_error=0.0;different_float_bits=0
        for index,(native,rust) in enumerate(zip(a,b)):
            x=native.split();y=rust.split()
            if len(x)!=11 or len(y)!=11:raise RuntimeError(f'{game}: invalid row {index}')
            if game=='q2':
                if x!=y:raise RuntimeError(f'{game} row {index}: {native} != {rust}')
            else:
                if x[:2]!=y[:2] or x[8:]!=y[8:]:raise RuntimeError(f'{game} flags/timer mismatch {index}: {native} != {rust}')
                for left,right in zip(x[2:8],y[2:8]):
                    error=abs(float(left)-float(right));maximum_error=max(maximum_error,error)
                    different_float_bits+=struct.pack('<f',float(left))!=struct.pack('<f',float(right))
                    if error>0.00003:raise RuntimeError(f'{game} movement mismatch {index}: {native} != {rust}')
        results[game]={'states':len(a),'scenarios':['flat walk','18-unit stair','wall/angled strafe','jump/landing','crouch/pitch','swimming'],'maximum_position_or_velocity_error':maximum_error,'different_float_components':different_float_bits,'integer_flags_and_timers':'exact','result':'PASS'}
    report={'scope':'original function comparison on analytic collision; not retail maps or gameplay','commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'source_tree_dirty':bool(subprocess.check_output(['git','status','--porcelain'],cwd=ROOT,text=True)),'build_seconds':build_seconds,'reference_flags':'-O2 -ffp-contract=off -fno-strict-aliasing','results':results,'result':'PASS'}
    (args.output/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))

if __name__=='__main__':main()
