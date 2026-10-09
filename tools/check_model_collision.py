#!/usr/bin/env python3
"""Compare production transformed models with unchanged original Q1/Q2/Q3 C.

Build qa-platform's model_collision example separately. --prepare-only extracts
and compiles C references without executing either reference or Rust binaries.
This synthetic developer fixture does not qualify gameplay or installation.
"""
import argparse
import json
from pathlib import Path
import random
import struct
import sys

sys.dont_write_bytecode = True
import check_brush_tree as tree
import check_hull_trace as hull

ROOT = Path(__file__).resolve().parents[1]
POSE_MAGIC = 0x45534f50
HULL_MAGIC = 0x4c48514d
HULL_COLUMNS = tree.COLUMNS[:8] + ['solid_and_environment_flags']


def replace_one(source, old, new):
    if source.count(old) != 1:
        raise ValueError('native fixture adapter anchor missing or duplicated')
    return source.replace(old, new)


def pack_row(row):
    map_id, model, mask, *vectors = row
    return struct.pack('<3I', map_id, model, mask) + b''.join(
        struct.pack('<3I', *(tree.bits(value) for value in vector)) for vector in vectors)


def pose_payload(poses):
    return struct.pack('<3I', POSE_MAGIC, 1, len(poses)) + b''.join(
        struct.pack('<6I', *(tree.bits(value) for vector in pose for value in vector))
        for pose in poses)


def brush_workload(seeded_rows):
    payload, metadata = tree.workload(seeded_rows)
    rotations = [[0, 0, 0], [-0.0, 0.0, -0.0], [17, 31, -23],
                 [-61, 127, 39], [0, 360, 0], [360, -360, 360]]
    origins = [[0, 0, 0], [23, -11, 7], [-5.25, 18.5, -3.75],
               [16777216, -16777216, 16777216]]
    poses = [(origins[(index // len(rotations)) % len(origins)],
              rotations[index % len(rotations)]) for index in range(metadata['rows'])]
    focused = metadata['focused_rows']

    def add(row, origin, angles, label):
        index = len(poses)
        payload.extend(pack_row(row))
        poses.append((origin, angles))
        metadata['labels'].append(label)
        focused[label] = index

    for angles in rotations:
        for model in [0, 2]:
            for case, (start, end, point) in enumerate([
                    ([-48, 0, 0], [48, 0, 0], [0, 0, 0]),
                    ([0, 0, 0], [0, 0, 0], [8, 0, 0]),
                    ([48, 48, 48], [96, 96, 96], [96, 96, 96])]):
                origin = [23, -11, 7]
                row = (0, model, tree.ALL,
                       [a + b for a, b in zip(start, origin)],
                       [a + b for a, b in zip(end, origin)],
                       [-.75, -2, -7], [1.25, 4, 9],
                       [a + b for a, b in zip(point, origin)])
                add(row, origin, angles, f'compound/asymmetric model={model},angles={angles},case={case}')
    add((0, 0, 1, [0, 0, 0], [10, 0, 0], [16777216, 0, 0],
         [16777218, 0, 0], [16777216, 0, 0]), [16777216, 0, 0], [0, 0, 0],
        'Q3 wrapper plus kernel double recenter: first size0=0,size1=2; second=-1,+1')
    for model in [0, 2]:
        for angles in rotations:
            add((0, model, tree.ALL, [-0.0, 0, -0.0], [0, -0.0, 0],
                 [0, 0, 0], [0, 0, 0], [-0.0, 0, -0.0]), [0, 0, 0], angles,
                f'signed-zero stationary model={model},angles={angles}')
    for angles in rotations:
        add((7, 2, 1, [-40, 2, 1], [40, -3, -2], [0, 0, 0], [0, 0, 0], [0, 0, 0]),
            [3, -5, 7], angles, f'nonaxial contact/local plane distance angles={angles}')
    metadata['rows'] = len(poses)
    struct.pack_into('<I', payload, 12, len(poses))
    metadata['pose_seed'] = None
    return payload, pose_payload(poses), metadata


def hull_workload(seeded_rows):
    rng = random.Random(0x4d4f444c)
    planes = [([1, 0, 0], distance, 0) for distance in [8, 24, 40, 72, 96, 112]]
    drawing = [(0, -1, -2), (3, -1, -2)]
    clips = [(1, -1, -2), (2, -1, -2), (4, -1, -2), (5, -1, -2)]
    models = [([0, 0, 1], [-256] * 3, [256] * 3),
              ([1, 2, 3], [-256] * 3, [256] * 3)]
    below3 = struct.unpack('<f', struct.pack('<I', tree.bits(3) - 1))[0]
    above32 = struct.unpack('<f', struct.pack('<I', tree.bits(32) + 1))[0]
    sizes = [([0, 0, 0], [0, 0, 0]), ([0, -3, -11], [below3, 5, 9]),
             ([0, -3, -11], [3, 5, 9]), ([-16, -16, -24], [16, 16, 32]),
             ([0, -3, -11], [above32, 5, 9]), ([-.75, -2.5, -7], [1.25, 4, 9])]
    angles = [[0, 0, 0], [17, 31, -23], [0, 360, 0], [-0.0, 0, -0.0]]
    origins = [[0, 0, 0], [23, -11, 7], [16777216, -16777216, 16777216]]
    rows, poses, labels, focused = [], [], [], {}

    def add(model, start, end, size, origin, angle, label):
        rows.append((0, model, tree.ALL, start, end, *size, [0, 0, 0]))
        poses.append((origin, angle))
        labels.append(label)

    for index in range(seeded_rows):
        origin = origins[index % len(origins)]
        start = [value + rng.uniform(-192, 192) for value in origin]
        end = start[:] if index % 13 == 0 else [value + rng.uniform(-192, 192) for value in origin]
        add(index % 2, start, end, sizes[(index // 2) % len(sizes)], origin,
            angles[index % len(angles)], 'seeded original hull/model/offset')
    for model in range(2):
        for origin in origins:
            for size_index, size in enumerate(sizes):
                for angle in angles:
                    for case, (start_x, end_x) in enumerate([(192, -192), (-192, -192), (192, 256)]):
                        label = f'hull model={model},origin={origin},size={size_index},angle={angle},case={case}'
                        focused[label] = len(rows)
                        add(model, [origin[0] + start_x, origin[1], origin[2]],
                            [origin[0] + end_x, origin[1], origin[2]], size, origin, angle, label)
    payload = bytearray(struct.pack('<7I', HULL_MAGIC, 1, len(planes), len(drawing),
                                    len(clips), len(models), len(rows)))
    for normal, distance, axis in planes:
        payload.extend(struct.pack('<5I', *(tree.bits(v) for v in normal), tree.bits(distance), axis))
    for plane, front, back in drawing + clips:
        payload.extend(struct.pack('<3I', plane, front & 0xffffffff, back & 0xffffffff))
    for roots, mins, maxs in models:
        payload.extend(struct.pack('<3I6I', *roots, *(tree.bits(v) for v in mins + maxs)))
    for row in rows:
        payload.extend(pack_row(row))
    return payload, pose_payload(poses), {'rows': len(rows), 'seeded_rows': seeded_rows,
        'seed': 0x4d4f444c, 'labels': labels, 'focused_rows': focused, 'models': 2,
        'geometry': 'six native axial halfspaces; distinct point/player/large roots per model'}


def extract_extra(qsrc, folder, path, names, output, rule):
    source = (folder / path).read_text()
    bodies, spans = [], []
    for name in names:
        body, span = tree.extract_function(source, name)
        span['path'] = str((folder / path).relative_to(qsrc))
        (output / f'{rule}-{name}-original.inc').write_text(body + '\n')
        bodies.append(body)
        spans.append(span)
    return bodies, spans


def brush_source(qsrc, rule, output):
    path, folder, spans = tree.source_for(qsrc, rule, output)
    source = path.read_text()
    groups = [('game/q_shared.c' if rule == 'q2' else 'game/q_math.c', ['AngleVectors']),
              ('qcommon/cmodel.c' if rule == 'q2' else 'qcommon/cm_trace.c',
               ['CM_TransformedBoxTrace'] if rule == 'q2' else
               ['RotatePoint', 'TransposeMatrix', 'CreateRotationMatrix', 'CM_TransformedBoxTrace']),
              ('qcommon/cmodel.c' if rule == 'q2' else 'qcommon/cm_test.c', ['CM_TransformedPointContents'])]
    bodies = []
    for relative, names in groups:
        extra, evidence = extract_extra(qsrc, folder, relative, names, output, rule)
        bodies += extra
        spans += evidence
    declarations = 'static int box_headnode = -2147483647;\n' if rule == 'q2' else ''
    source = replace_one(source, 'static int word(FILE *in, uint32_t *value) {',
                         declarations + '\n\n'.join(bodies) + '\n\nstatic int word(FILE *in, uint32_t *value) {')
    source = replace_one(source, 'if (argc != 3 ||', 'if (argc != 4 ||')
    source = replace_one(source, 'FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");',
                         'FILE *in = fopen(argv[1], "rb"), *poses = fopen(argv[2], "rb"), *out = fopen(argv[3], "wb");')
    source = replace_one(source, 'for (uint32_t q = 0; q < queries; ++q) {',
                         'uint32_t pmagic, pversion, pcount;\n'
                         '    if (!poses || !word(poses, &pmagic) || pmagic != 0x45534f50 ||\n'
                         '        !word(poses, &pversion) || pversion != 1 || !word(poses, &pcount) || pcount != queries) return 25;\n'
                         '    for (uint32_t q = 0; q < queries; ++q) {')
    source = replace_one(source, 'vec3_t start, end, mins, maxs, point;',
                         'vec3_t start, end, mins, maxs, point, origin, angles;\n'
                         '        if (!vector(poses, origin) || !vector(poses, angles)) return 25;')
    if rule == 'q2':
        source = replace_one(source, 'CM_BoxTrace(start, end, mins, maxs, f->roots[model], (int)mask)',
                             'CM_TransformedBoxTrace(start, end, mins, maxs, f->roots[model], (int)mask, origin, angles)')
        source = replace_one(source, 'CM_PointContents(point, f->roots[model])',
                             'CM_TransformedPointContents(point, f->roots[model], origin, angles)')
    else:
        source = replace_one(source, 'CM_Trace(&trace, start, end, mins, maxs, (clipHandle_t)model, vec3_origin, (int)mask, 0, NULL);',
                             'CM_TransformedBoxTrace(&trace, start, end, mins, maxs, (clipHandle_t)model, (int)mask, origin, angles, 0);')
        source = replace_one(source, 'CM_PointContents(point, (clipHandle_t)model)',
                             'CM_TransformedPointContents(point, (clipHandle_t)model, origin, angles)')
    source = replace_one(source, 'if (fgetc(in) != EOF || ferror(in)) return 22;',
                         'if (fgetc(in) != EOF || ferror(in) || fgetc(poses) != EOF || ferror(poses)) return 22;\n    fclose(poses);')
    if any(source.count(body) != 1 for body in bodies):
        raise ValueError('transformed native body changed or duplicated')
    path = output / f'{rule}-model-native.c'
    path.write_text(source)
    return path, folder, spans


Q1_ADAPTER = r'''
static int word(FILE *in, uint32_t *value) {
    unsigned char b[4]; if(fread(b,1,4,in)!=4) return 0;
    *value=(uint32_t)b[0]|(uint32_t)b[1]<<8|(uint32_t)b[2]<<16|(uint32_t)b[3]<<24; return 1;
}
static int scalar(FILE *in,float *value) { uint32_t bits; if(!word(in,&bits)) return 0; memcpy(value,&bits,4); return 1; }
static int vector(FILE *in,vec3_t value) { return scalar(in,value)&&scalar(in,value+1)&&scalar(in,value+2); }
int main(int argc,char **argv) {
    if(argc!=4||sizeof(float)!=4||sizeof(int)!=4) return 2;
    FILE *in=fopen(argv[1],"rb"),*poses=fopen(argv[2],"rb"),*out=fopen(argv[3],"wb");
    uint32_t magic,version,np,nd,nc,nm,nq,pm,pv,pn;
    if(!in||!poses||!out||!word(in,&magic)||magic!=0x4c48514d||!word(in,&version)||version!=1||
       !word(in,&np)||np<1||np>64||!word(in,&nd)||nd<1||nd>128||!word(in,&nc)||nc<1||nc>128||
       !word(in,&nm)||nm<2||nm>8||!word(in,&nq)||nq<1||nq>50000||
       !word(poses,&pm)||pm!=0x45534f50||!word(poses,&pv)||pv!=1||!word(poses,&pn)||pn!=nq) return 3;
    mplane_t planes[64]={0}; dclipnode_t drawing[128]={0},clips[128]={0}; model_t models[8]={0};
    for(uint32_t p=0;p<np;p++) { uint32_t type; if(!vector(in,planes[p].normal)||!scalar(in,&planes[p].dist)||!word(in,&type)||type>3) return 4; planes[p].type=(int)type; }
    for(uint32_t n=0;n<nd+nc;n++) { uint32_t plane,a,b; dclipnode_t *node=n<nd?drawing+n:clips+n-nd;
        if(!word(in,&plane)||plane>=np||!word(in,&a)||!word(in,&b)) return 5; node->planenum=plane; node->children[0]=(int32_t)a; node->children[1]=(int32_t)b; }
    for(uint32_t m=0;m<nm;m++) { uint32_t roots[3]; vec3_t boundmin,boundmax;
        for(int h=0;h<3;h++) if(!word(in,roots+h)||roots[h]>=(h?nc:nd)) return 6;
        if(!vector(in,boundmin)||!vector(in,boundmax)) return 6; models[m].type=mod_brush;
        for(int h=0;h<3;h++) { hull_t *target=models[m].hulls+h; target->clipnodes=h?clips:drawing; target->planes=planes;
            target->firstclipnode=(int)roots[h]; target->lastclipnode=(int)(h?nc:nd)-1;
            float mins[3][3]={{0,0,0},{-16,-16,-24},{-32,-32,-24}};
            float maxs[3][3]={{0,0,0},{16,16,32},{32,32,64}};
            VectorCopy(mins[h],target->clip_mins); VectorCopy(maxs[h],target->clip_maxs); }
        sv.models[m]=models+m; }
    for(uint32_t q=0;q<nq;q++) { uint32_t map,model,mask; vec3_t start,end,mins,maxs,point,origin,angles;
        if(!word(in,&map)||map!=0||!word(in,&model)||model>=nm||!word(in,&mask)||!vector(in,start)||!vector(in,end)||
           !vector(in,mins)||!vector(in,maxs)||!vector(in,point)||!vector(poses,origin)||!vector(poses,angles)) return 7;
        edict_t entity={0}; entity.v.solid=SOLID_BSP; entity.v.movetype=MOVETYPE_PUSH; entity.v.modelindex=(float)model;
        VectorCopy(origin,entity.v.origin); VectorCopy(angles,entity.v.angles);
        trace_t trace=SV_ClipMoveToEntity(&entity,start,mins,maxs,end);
        uint32_t flags=!!trace.startsolid|!!trace.allsolid<<1|!!trace.inopen<<2|!!trace.inwater<<3;
        if(fwrite(&trace.fraction,4,1,out)!=1||fwrite(trace.endpos,4,3,out)!=3||fwrite(trace.plane.normal,4,3,out)!=3||
           fwrite(&trace.plane.dist,4,1,out)!=1||fwrite(&flags,4,1,out)!=1) return 8; }
    if(fgetc(in)!=EOF||ferror(in)||fgetc(poses)!=EOF||ferror(poses)) return 9;
    fclose(in);fclose(poses);return fclose(out)?10:0;
}
'''


def hull_source(qsrc, output):
    path = qsrc / 'quake/WinQuake/world.c'
    source = path.read_text()
    bodies, spans = [], []
    for name in ['SV_HullPointContents', 'SV_RecursiveHullCheck', 'SV_HullForEntity', 'SV_ClipMoveToEntity']:
        body = hull.function(source, name)
        offset = source.index(body)
        if source[offset:offset + len(body)] != body:
            raise ValueError('Q1 extraction is not an exact contiguous slice')
        line = source[:offset].count('\n') + 1
        spans.append({'path': str(path.relative_to(qsrc)), 'function': name,
                      'start_line': line, 'end_line': line + body.count('\n'),
                      'source_bytes': len(body.encode()), 'exact_contiguous_source_slice': True})
        (output / f'q1-{name}-original.inc').write_text(body + '\n')
        bodies.append(body)
    prefix = replace_one(hull.PREFIX,
        'int firstclipnode, lastclipnode; } hull_t;',
        'int firstclipnode, lastclipnode; vec3_t clip_mins, clip_maxs; } hull_t;')
    prefix = replace_one(prefix, 'typedef struct { int allsolid,',
                         'typedef struct edict_s edict_t;\ntypedef struct { int allsolid,')
    prefix = replace_one(prefix, 'mplane_t plane; } trace_t;', 'mplane_t plane; edict_t *ent; } trace_t;')
    definitions = '\n'.join(tree.extract_define((qsrc / 'quake/WinQuake/server.h').read_text(), name)[0]
                            for name in ['SOLID_BSP', 'MOVETYPE_PUSH'])
    prefix += '\n' + definitions + r'''
#define VectorAdd(a,b,c) do { for(int v=0;v<3;v++) (c)[v]=(a)[v]+(b)[v]; } while(0)
enum { mod_brush=0 };
typedef struct { int type; hull_t hulls[3]; } model_t;
struct edict_s { struct { float solid,movetype,modelindex; vec3_t origin,mins,maxs,angles; } v; };
static struct { model_t *models[8]; } sv;
static hull_t *SV_HullForBox(vec3_t mins,vec3_t maxs) { (void)mins;(void)maxs; fputs("excluded Q1 temporary box reached\n",stderr);exit(24); }
'''
    generated = prefix + '\n\n'.join(bodies) + Q1_ADAPTER
    if any(generated.count(body) != 1 for body in bodies):
        raise ValueError('Q1 transformed native body changed or duplicated')
    target = output / 'q1-model-native.c'
    target.write_text(generated)
    return target, qsrc / 'quake/WinQuake', spans


def hull_compare(expected, actual, metadata):
    count = metadata['rows']
    if len(expected) != count * 36 or len(actual) != count * 36:
        raise ValueError('Q1 native/Rust model rows differ in length')
    differences, first, matches = dict.fromkeys(HULL_COLUMNS, 0), [], 0
    for index, (left, right) in enumerate(zip(struct.iter_unpack('<9I', expected), struct.iter_unpack('<9I', actual))):
        if left == right:
            matches += 1
            continue
        fields = []
        for column, native, rust in zip(HULL_COLUMNS, left, right):
            if native != rust:
                differences[column] += 1
                fields.append({'field': column, 'native_raw': f'{native:08x}', 'rust_raw': f'{rust:08x}'})
        if len(first) < 20:
            first.append({'row': index, 'case': metadata['labels'][index], 'fields': fields})
    return {'rows_compared': count, 'rows_matched_bit_exact': matches,
            'different_fields': {k: v for k, v in differences.items() if v},
            'first_mismatches': first, 'result': 'PASS' if matches == count else 'FAIL'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--rust-binary', type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    parser.add_argument('--cc', default='cc')
    parser.add_argument('--seeded-rows', type=int, default=10000)
    parser.add_argument('--rules', nargs='+', choices=['q1', 'q2', 'q3'], default=['q1', 'q2', 'q3'])
    parser.add_argument('--timings', action='store_true')
    parser.add_argument('--cores')
    args = parser.parse_args()
    if not 10000 <= args.seeded_rows <= 40000:
        parser.error('--seeded-rows must be 10000..40000')
    if not args.prepare_only and args.rust_binary is None:
        parser.error('--rust-binary is required unless --prepare-only')
    if args.timings and not args.cores:
        parser.error('--timings requires explicit --cores')
    args.qsrc = args.qsrc.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    if args.rust_binary is not None:
        args.rust_binary = args.rust_binary.resolve(strict=True)
    results = {}
    for rule in args.rules:
        payload, poses, metadata = hull_workload(args.seeded_rows) if rule == 'q1' else brush_workload(args.seeded_rows)
        fixture = args.output / f'{rule}-models.bin'
        pose_file = args.output / f'{rule}-poses.bin'
        fixture.write_bytes(payload)
        pose_file.write_bytes(poses)
        if fixture.read_bytes() != payload or pose_file.read_bytes() != poses:
            raise ValueError('model fixture/pose readback differs')
        (args.output / f'{rule}-workload.json').write_text(json.dumps(metadata, indent=2) + '\n')
        source, folder, spans = hull_source(args.qsrc, args.output) if rule == 'q1' else brush_source(args.qsrc, rule, args.output)
        binary = args.output / f'{rule}-model-native'
        includes = [] if rule == 'q1' else ['-I' + str(folder / 'game'), '-I' + str(folder / 'qcommon')]
        tree.run([args.cc, *tree.C_FLAGS, *includes, str(source), '-lm', '-o', str(binary)], args.output, f'{rule}-compile')
        if args.prepare_only:
            results[rule] = {'result': 'PREPARED', 'source_extraction': spans, 'rows': metadata['rows']}
            continue
        expected, actual = args.output / f'{rule}-native.bin', args.output / f'{rule}-rust.bin'
        tree.run([str(binary), str(fixture), str(pose_file), str(expected)], args.output, f'{rule}-native')
        command = [str(args.rust_binary), rule, str(fixture), str(pose_file), str(actual)]
        if args.timings:
            command = ['taskset', '-c', args.cores, *command, '--timings', str(expected), str(args.output / f'{rule}-timing.json')]
        run = tree.run(command, args.output, f'{rule}-rust')
        allocation = json.loads(run.stdout)
        if allocation['rows'] != metadata['rows'] or any(allocation[key] != 0 for key in
                ['rust_allocations', 'rust_reallocations', 'rust_requested_bytes']) or allocation['allocation_positive_control'] != 1:
            raise ValueError('model row/allocation gate or positive control differs')
        native = expected.read_bytes()
        comparator = hull_compare if rule == 'q1' else tree.compare
        outcome = comparator(native, actual.read_bytes(), metadata)
        controls = []
        for column in [0, 8 if rule == 'q1' else 12]:
            row_bytes = 36 if rule == 'q1' else tree.BYTES
            row = metadata['rows'] // 2
            mutated = bytearray(native)
            mutated[row * row_bytes + column * 4] ^= 1
            control = comparator(native, mutated, metadata)
            columns = HULL_COLUMNS if rule == 'q1' else tree.COLUMNS
            if control['result'] != 'FAIL' or control['rows_matched_bit_exact'] != metadata['rows'] - 1 or control['different_fields'] != {columns[column]: 1}:
                raise ValueError('model comparator one-bit negative control differs')
            (args.output / f'{rule}-{columns[column]}-mutation.bin').write_bytes(mutated)
            controls.append({'row': row, 'field': columns[column], 'one_bit_rejected': True})
        if comparator(native, native, metadata)['result'] != 'PASS':
            raise ValueError('model comparator identical positive control failed')
        columns = HULL_COLUMNS if rule == 'q1' else tree.COLUMNS
        row_bytes = len(columns) * 4
        focused = {name: {'row': index, 'native_raw': list(struct.unpack_from(
            '<' + str(len(columns)) + 'I', native, index * row_bytes))}
            for name, index in metadata['focused_rows'].items()}
        outcome.update({'source_extraction': spans, 'allocation': allocation,
                        'identical_positive_control': 'PASS', 'negative_controls': controls,
                        'focused_results': focused})
        results[rule] = outcome
    report = {'scope': 'synthetic transformed inline hull/brush comparisons with unchanged qsrc wrappers',
              'gameplay': False, 'installation_qualified': False,
              'performance_measured': bool(args.timings and not args.prepare_only),
              'native_flags': tree.C_FLAGS, 'results': results,
              'limits': ['no retail BSP load, linked scene/filter/order or VM/wire ABI proof',
                         'Q1 synthetic hull traces only; no Q1 point contents or temporary boxes',
                         'Q2/Q3 brush-only; no patches, capsules or special box/capsule handles',
                         'zero-normal invalid plane axis normalized to no-axis for brush rows only',
                         'Rust calling thread allocation scope; no C/native heap or worker scope',
                         'timing samples include 64 queries and comparisons; Q1 trace only, Q2/Q3 trace plus point contents; p99 is batch p99 divided by 64',
                         'no comparable pre-transform baseline exists; timings are not a regression qualification'],
              'result': 'PREPARED' if args.prepare_only else ('PASS' if all(v['result'] == 'PASS' for v in results.values()) else 'FAIL')}
    path = args.output / 'report.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'report': str(path), 'result': report['result']}))
    if report['result'] == 'FAIL':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
