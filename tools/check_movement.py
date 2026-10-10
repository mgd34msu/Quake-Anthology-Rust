#!/usr/bin/env python3
"""Compare the shared movement entry with original Q2/Q3 Pmove on analytic fixtures."""
import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import time
from check_hull_trace import function

ROOT = Path(__file__).resolve().parents[1]

def q3_attribution(qsrc, output, rcc):
    """Diagnostic only: retain the native oracle and isolate one literal's width."""
    base = qsrc / 'quake-iii-arena/code'
    source = (base / 'game/bg_pmove.c').read_text()
    expression = 'scale = (float)pm->ps->speed * max / ( 127.0 * total );'
    if source.count(expression) != 1:
        raise RuntimeError('original PM_CmdScale expression changed')
    variant = output / 'bg_pmove-f32-scale.c'
    variant.write_text(source.replace(expression, expression.replace('127.0', '127.0f')))
    executable = output / 'q3-f32-scale'
    command = ['cc', '-std=c99', '-O2', '-ffp-contract=off', '-fno-strict-aliasing',
               '-ffunction-sections', '-fdata-sections', '-include', 'strings.h',
               '-I', str(base / 'game'), str(ROOT / 'tools/probes/movement_q3.c'),
               str(variant), *[str(base / s) for s in
                              ['game/bg_slidemove.c', 'game/bg_misc.c', 'game/q_shared.c', 'game/q_math.c']],
               '-Wl,--gc-sections', '-lm', '-o', str(executable)]
    compiled = subprocess.run(command, capture_output=True, text=True, timeout=300)
    (output / 'q3-f32-compile.log').write_text(compiled.stdout + compiled.stderr)
    (output / 'q3-f32-compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
    compiled.check_returncode()
    expected = subprocess.check_output([str(executable)], text=True, timeout=300)
    actual = (output / 'q3-rust.txt').read_text()
    (output / 'q3-f32-scale.txt').write_text(expected)
    left, right = expected.splitlines(), actual.splitlines()
    if len(left) != 1152 or len(right) != len(left):
        raise RuntimeError('Q3 diagnostic row count')
    for row, (native, rust) in enumerate(zip(left, right)):
        a, b = native.split(), rust.split()
        if a[:2] + a[8:] != b[:2] + b[8:] or any(
                struct.pack('<f', float(x)) != struct.pack('<f', float(y))
                for x, y in zip(a[2:8], b[2:8])):
            raise RuntimeError(f'Q3 binary32-scale diagnostic differs at row {row}')
    assembly = subprocess.check_output(['objdump', '-d', str(output / 'q3-original')], text=True)
    (output / 'q3-native-disassembly.txt').write_text(assembly)
    instructions = [line.split('\t')[-1].split()[0] for line in assembly.splitlines() if '\t' in line and line.split('\t')[-1].split()]
    report = {'states': len(left), 'float_components': len(left) * 6,
              'different_float_components': 0, 'integer_flags_and_timers': 'exact',
              'changed_reference_expression': '127.0 -> 127.0f in PM_CmdScale only',
              'compiler': subprocess.check_output(['cc', '--version'], text=True).splitlines()[0],
              'target': subprocess.check_output(['cc', '-dumpmachine'], text=True).strip(),
              'x87_instructions': sum(i.startswith(('fld', 'fst', 'fadd', 'fsub', 'fmul', 'fdiv')) for i in instructions),
              'fused_multiply_add_instructions': sum('fmadd' in i or 'fmsub' in i for i in instructions),
              'scope': 'single-expression diagnostic C build; original C oracle retained, not a QVM runtime or retail gameplay proof'}
    if rcc:
        code = output / 'cmd-scale-qvm.c'
        code.write_text('int abs(int); double sqrt(double);\n'
                        'typedef struct {signed char forwardmove,rightmove,upmove;} usercmd_t;\n'
                        'typedef struct {int speed;} playerState_t;\n'
                        'struct move {playerState_t *ps;}; struct move *pm;\n'
                        + function(source, 'PM_CmdScale')
                        + '\nfloat scale(usercmd_t *cmd) {return PM_CmdScale(cmd);}\n')
        asm = output / 'cmd-scale-qvm.asm'
        compiled = subprocess.run([str(rcc), '-target=bytecode', str(code), str(asm)],
                                  capture_output=True, text=True, timeout=300)
        (output / 'q3-rcc.log').write_text(compiled.stdout + compiled.stderr)
        compiled.check_returncode()
        text = asm.read_text()
        if 'CNSTF4 1123942400' not in text or 'MULF4\nDIVF4' not in text:
            raise RuntimeError('original lcc did not emit expected binary32 scale operations')
        report['original_lcc_scale'] = 'CNSTF4 127.0, MULF4, DIVF4; four-byte float/double target'
    return report

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--q3-attribution', action='store_true', help='retain native C and check the single binary32 scale-literal diagnostic')
    parser.add_argument('--q3-rcc', type=Path, help='optional original lcc rcc built for its 32-bit host to inspect scale bytecode')
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
    if args.q3_attribution:
        report['q3_attribution'] = q3_attribution(args.qsrc, args.output, args.q3_rcc)
    (args.output/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))

if __name__=='__main__':main()
