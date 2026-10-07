"""Time the C port's actual native hull kernel on the Rust probe's segments."""
import json
from pathlib import Path
import subprocess
import time

PROGRAM = r'''
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <time.h>
#include "C_PORT_Q1"

typedef struct {float normal[3],distance;int32_t type;} disk_plane;
typedef struct {uint32_t hull;qa_vec3 start,end;} segment;
static uint64_t now(void) {
    struct timespec t;
    if(clock_gettime(CLOCK_MONOTONIC,&t)) exit(2);
    return (uint64_t)t.tv_sec*1000000000ull+(uint64_t)t.tv_nsec;
}
static int compare(const void *a,const void *b) {
    uint64_t x=*(const uint64_t *)a,y=*(const uint64_t *)b;
    return (x>y)-(x<y);
}
int main(int argc,char **argv) {
    if(argc!=2) return 2;
    FILE *input=fopen(argv[1],"rb");
    uint32_t counts[3];int32_t roots[3];
    if(!input || fread(counts,4,3,input)!=3 || fread(roots,4,3,input)!=3) return 2;
    qa_collision_plane *planes=calloc(counts[0],sizeof(*planes));
    q1node *drawing=calloc(counts[1],sizeof(*drawing)),*clips=calloc(counts[2],sizeof(*clips));
    segment *segments=calloc(30000,sizeof(*segments));
    uint64_t *samples=calloc(600000,sizeof(*samples));
    if(!planes || !drawing || !clips || !segments || !samples) return 2;
    for(uint32_t i=0;i<counts[0];i++) {
        disk_plane p;
        if(fread(&p,sizeof(p),1,input)!=1) return 2;
        planes[i]=(qa_collision_plane){.normal={p.normal[0],p.normal[1],p.normal[2]},.distance=p.distance,.type=p.type};
    }
    if(fread(drawing,sizeof(*drawing),counts[1],input)!=counts[1] ||
       fread(clips,sizeof(*clips),counts[2],input)!=counts[2] ||
       fread(segments,sizeof(*segments),30000,input)!=30000) return 2;
    fclose(input);
    qa_arena arena;qa_arena_init(&arena,16384);
    qa_error error={0};q1work work={.arena=&arena,.error=&error};
    const char *names[3]={"point","player","large"};
    printf("{\"scope\":\"actual C-port native kernel; excludes engine dispatch\",\"rows\":[");
    for(unsigned hull=0;hull<3;hull++) {
        q1hull view={hull?clips:drawing,counts[hull?2:1],planes,counts[0],roots[hull]};
        for(size_t i=0;i<600;i++) {
            segment q=segments[hull*10000+i];native_trace trace;
            if(q.hull!=hull || !trace_hull(&work,&view,q.start,q.end,NULL,&trace)) return 3;
            __asm__ volatile("" : : "m"(trace) : "memory");
            qa_arena_reset(&arena);
        }
        for(size_t i=0;i<600000;i++) {
            segment q=segments[hull*10000+i%10000];native_trace trace;
            uint64_t started=now();
            if(q.hull!=hull || !trace_hull(&work,&view,q.start,q.end,NULL,&trace)) return 3;
            __asm__ volatile("" : : "m"(trace) : "memory");
            samples[i]=now()-started;
            qa_arena_reset(&arena);
        }
        qsort(samples,600000,sizeof(*samples),compare);
        double median=((double)samples[299999]+(double)samples[300000])*0.5;
        printf("%s{\"hull\":\"%s\",\"median_ns\":%.1f,\"p99_ns\":%llu}",hull?",":"",names[hull],median,(unsigned long long)samples[593999]);
    }
    printf("],\"warmup_traces_per_hull\":600,\"timed_traces_per_hull\":600000}\n");
    qa_arena_destroy(&arena);free(planes);free(drawing);free(clips);free(segments);free(samples);
    return 0;
}
'''


def measure(c_port, data, output, cores):
    c_port = c_port.resolve(strict=True)
    commit = subprocess.check_output(["git", "-C", str(c_port), "rev-parse", "HEAD"], text=True).strip()
    dirty = bool(subprocess.check_output(["git", "-C", str(c_port), "status", "--porcelain"], text=True))
    dependencies = ["src/world/collision/q1/geometry.c", "src/world/collision/contents.c", "src/core/common.c", "src/core/arena.c"]
    # Retain the exact dirty C source used, without hashing or changing its tree.
    snapshot = output / "c-port-source"
    paths = [c_port / "src/world/collision/q1.c", *[c_port / path for path in dependencies],
             *sorted((c_port / "include").rglob("*.h")), *sorted((c_port / "src/world/collision").rglob("*.h"))]
    copied = [(path, path.read_bytes()) for path in paths]
    for path, value in copied:
        target = snapshot / path.relative_to(c_port)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(value)
    if any(path.read_bytes() != value for path, value in copied):
        raise ValueError("C source changed while capturing the probe")
    source = output / "c-port-probe.c"
    source.write_text(PROGRAM.replace("C_PORT_Q1", str(snapshot.resolve() / "src/world/collision/q1.c")))
    executable = output / "c-port-probe"
    flags = ["-O3", "-ffp-contract=off", "-ffunction-sections", "-fdata-sections", "-Wl,--gc-sections"]
    command = ["cc", *flags, "-I" + str(snapshot / "include"), str(source), *[str(snapshot / path) for path in dependencies], "-lm", "-o", str(executable)]
    started = time.monotonic()
    build = subprocess.run(command, capture_output=True, text=True, timeout=300)
    (output / "c-port-build.log").write_text(build.stdout + build.stderr)
    build.check_returncode()
    build_seconds = time.monotonic() - started
    run = subprocess.run(["taskset", "-c", cores, str(executable), str(data)], capture_output=True, text=True, timeout=300)
    (output / "c-port-run.log").write_text(run.stdout + run.stderr)
    run.check_returncode()
    return {"commit": commit, "source_tree_dirty": dirty, "source": str(c_port / "src/world/collision/q1.c"), "source_snapshot": str(snapshot), "compiler_flags": flags, "build_seconds": build_seconds, "cpu_affinity": cores, "debugger": False, "result": json.loads(run.stdout), "limits": "Native kernel only with NULL blocking policy; not an installed C engine timing. The C port uses float epsilon/backoff literals, unlike the original world.c reference; no exact cross-port speedup claim."}
