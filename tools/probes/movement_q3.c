/* Developer-only native Q2 Pmove fixture; never linked into the engine. */
#include "q_shared.h"
#include "bg_public.h"
#include <stdint.h>

static int use_step, use_water;
static trace_t fixture_trace(vec3_t start, vec3_t mins, vec3_t maxs, vec3_t end) {
    trace_t result; memset(&result,0,sizeof result); result.fraction=1;result.entityNum=ENTITYNUM_NONE;
    const float normals[4][3]={{0,0,1},{-1,0,0},{0,1,0},{0,-1,0}};
    const float distances[4]={0,-240,-120,-120};
    for(int p=0;p<4;p++) {
        float a=-distances[p],b=-distances[p];
        for(int i=0;i<3;i++) {float offset=normals[p][i]*(normals[p][i]<0?maxs[i]:mins[i]);a+=start[i]*normals[p][i]+offset;b+=end[i]*normals[p][i]+offset;}
        if(a<0) {result.startsolid=1;result.allsolid=b<0;}
        if(a>=0&&b<0) {float fraction=a/(a-b);if(fraction<result.fraction) {result.fraction=fraction;VectorCopy(normals[p],result.plane.normal);result.plane.dist=distances[p];}}
    }
    if(use_step) {
        float low[3]={96-maxs[0],-1000000-maxs[1],-1000000-maxs[2]};
        float high[3]={160-mins[0],1000000-mins[1],16-mins[2]};
        int inside=1,endinside=1;for(int i=0;i<3;i++) {inside&=start[i]>low[i]&&start[i]<high[i];endinside&=end[i]>low[i]&&end[i]<high[i];}
        if(inside){result.startsolid=1;result.allsolid=endinside;}
        else {
            float enter=0,exit=1,normal[3]={0};int miss=0;
            for(int i=0;i<3;i++) {
                float delta=end[i]-start[i];
                if(delta==0) {if(start[i]<=low[i]||start[i]>=high[i]) {miss=1;break;}continue;}
                float near=(delta>0?low[i]-start[i]:high[i]-start[i])/delta;
                float far=(delta>0?high[i]-start[i]:low[i]-start[i])/delta;
                if(near>=enter) {enter=near;VectorClear(normal);normal[i]=delta>0?-1:1;}
                if(far<exit)exit=far;if(enter>exit){miss=1;break;}
            }
            if(!miss&&(normal[0]!=0||normal[1]!=0||normal[2]!=0)&&enter<result.fraction&&exit>0){result.fraction=enter;VectorCopy(normal,result.plane.normal);}
        }
    }
    for(int i=0;i<3;i++)result.endpos[i]=start[i]+(end[i]-start[i])*result.fraction;
    if(result.fraction<1||result.startsolid) {result.entityNum=ENTITYNUM_WORLD;result.contents=CONTENTS_SOLID;}
    return result;
}
static int fixture_contents(vec3_t p) {return use_water && p[2]<64 ? CONTENTS_WATER : 0;}
void Com_Printf(const char *fmt,...) {(void)fmt;}
void Com_DPrintf(const char *fmt,...) {(void)fmt;}
void Com_Error(int code,const char *fmt,...) {(void)code;(void)fmt;abort();}
void trap_SnapVector(float *v) { for(int i=0;i<3;i++)v[i]=nearbyintf(v[i]); }
static void arena_trace(trace_t *out,const vec3_t start,const vec3_t mins,const vec3_t maxs,const vec3_t end,int actor,int mask) {
    (void)actor;(void)mask;*out=fixture_trace((float*)start,(float*)mins,(float*)maxs,(float*)end);
}
static int arena_contents(const vec3_t p,int actor) {(void)actor;return fixture_contents((float*)p);}
int main(void) {
    for(int scenario=0;scenario<6;scenario++) {
        playerState_t state;memset(&state,0,sizeof state);state.gravity=800;state.speed=320;state.stats[STAT_HEALTH]=100;state.origin[2]=24;state.groundEntityNum=ENTITYNUM_NONE;
        pmove_t move;memset(&move,0,sizeof move);move.ps=&state;move.trace=arena_trace;move.pointcontents=arena_contents;move.tracemask=MASK_PLAYERSOLID;use_step=scenario==1;use_water=scenario==5;
        uint32_t seed=0x12345678;
        for(int frame=0;frame<192;frame++) {
            seed=seed*1664525u+1013904223u;
            move.cmd.serverTime+=8+(seed%17);
            move.cmd.forwardmove=scenario<2?127:(int)((seed>>8)%255)-127;
            move.cmd.rightmove=scenario<2?0:(int)((seed>>18)%255)-127;
            move.cmd.upmove=scenario==3&&frame%48<8?127:0;
            if(scenario==4){move.cmd.upmove=frame%64<32?-127:0;move.cmd.angles[0]=frame%64<32?8192:-8192;}
            move.cmd.angles[1]=scenario<2?0:(frame/48)*16384;
            Pmove(&move);
            printf("%d %d %.9g %.9g %.9g %.9g %.9g %.9g %d %d %d\n",scenario,frame,state.origin[0],state.origin[1],state.origin[2],state.velocity[0],state.velocity[1],state.velocity[2],state.groundEntityNum!=ENTITYNUM_NONE,!!(state.pm_flags&PMF_JUMP_HELD),state.pm_time);
        }
    }
}
