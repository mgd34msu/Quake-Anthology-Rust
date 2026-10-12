#!/usr/bin/env python3
"""Extract item metadata from original QC/C tables into one engine catalogue."""
import argparse
import ast
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TOKEN = re.compile(r'"(?:\\.|[^"\\])*"|/\*[\s\S]*?\*/|//[^\n]*')


def clean(source):
    return TOKEN.sub(lambda match: match[0] if match[0].startswith('"') else ' ', source)


def block(source, start):
    opening = source.index('{', start)
    depth = 0
    for match in re.finditer(r'"(?:\\.|[^"\\])*"|[{}]', source[opening:]):
        token = match[0]
        depth += (token == '{') - (token == '}')
        if depth == 0:
            return source[opening + 1:opening + match.start()]
    raise ValueError('unclosed initializer')


def fields(source):
    depth, start, values = 0, 0, []
    for match in re.finditer(r'"(?:\\.|[^"\\])*"|[{},()]', source):
        value = match[0]
        if value in ('{', '('):
            depth += 1
        elif value in ('}', ')'):
            depth -= 1
        elif value == ',' and depth == 0:
            values.append(source[start:match.start()].strip())
            start = match.end()
    if source[start:].strip():
        values.append(source[start:].strip())
    return values


def text(value):
    strings = re.findall(r'"(?:\\.|[^"\\])*"', value)
    return ''.join(ast.literal_eval(string) for string in strings)


def row(module, number, classname, kind, amount, models, sound, label='', ammo='', weapon=0, maximum=0, pickup='', fire=''):
    return dict(module=module, number=number, classname=classname, kind=kind, amount=amount, maximum=maximum,
                models=models, sound=sound, label=label or classname, ammo=ammo, weapon=weapon, pickup=pickup, fire=fire)


def qc(qsrc, module, directory):
    source = clean((qsrc / directory / 'items.qc').read_text())
    defs = clean((qsrc / directory / 'defs.qc').read_text())
    constants = {name:int(value) for name, value in re.findall(r'float\s+(IT_\w+)\s*=\s*(\d+)\s*;', defs)}
    ammo_names = dict(shells='item_shells', nails='item_spikes', rockets='item_rockets', cells='item_cells')
    maxima = {name:int(value) for name,value in re.findall(r'if\s*\(other.ammo_(\w+)\s*>\s*(\d+)\)',source)}
    pickup = block(source,source.index('void() weapon_touch ='))
    weapons = {}
    for match in re.finditer(r'(?:if|else if)\s*\(self.classname == "(weapon_\w+)"\)',pickup):
        body = block(pickup,match.start())
        native = re.search(r'new\s*=\s*(IT_\w+)',body)[1]
        ammo,amount = re.search(r'other.ammo_(\w+)\s*=\s*other.ammo_\w+\s*\+\s*(\d+)',body).groups()
        weapons[match[1]] = (constants[native],ammo_names[ammo],int(amount))
    rows=[]
    for match in re.finditer(r'void\(\)\s+((?:item|weapon)_\w+)\s*=\s*\{',source):
        name=match[1]
        body=block(source,match.start())
        if not re.search(r'\bStartItem\s*\(',body):
            continue
        model_paths=re.findall(r'setmodel\s*\(self,\s*"([^"]+)"',body)
        noises=re.findall(r'self.noise\s*=\s*"([^"]+)"',body)
        labels=re.findall(r'self.netname\s*=\s*"([^"]+)"',body)
        touch=re.search(r'self.touch\s*=\s*(\w+)',body)
        amount_values=re.findall(r'self.(?:aflag|healamount)\s*=\s*(\d+)',body)
        kind='Health' if name=='item_health' else 'Armor' if 'armor' in name else 'Ammo' if name in [*ammo_names.values(),'item_weapon'] else 'Weapon' if name.startswith('weapon_') else 'Key' if 'key' in name or name=='item_sigil' else 'Powerup'
        amount=int(amount_values[-1]) if amount_values else 1
        maximum=0
        sound=noises[-1] if noises else ''
        native_weapon,ammo=0,''
        if kind=='Armor':
            armor = re.search(r'if\s*\(self.classname == "'+name+r'"\)\s*\{([^}]+)',source)[1]
            amount=int(re.search(r'value\s*=\s*(\d+)',armor)[1])
            maximum=amount
            sound='items/armor1.wav'
        if kind=='Ammo':
            for key,value in ammo_names.items():
                if value==name: maximum=maxima[key]
            sound='weapons/lock4.wav'
        if kind=='Weapon':
            native_weapon,ammo,amount=weapons[name]
            maximum=1
            sound='weapons/pkup.wav'
        rows.append(row(module,len(rows)+1,name,kind,amount,list(dict.fromkeys(model_paths)),sound,labels[-1] if labels else name,ammo,native_weapon,maximum,touch[1] if touch else ''))
    map_count=len(rows)
    rows.extend([row(module,65534,'weapon_axe','Weapon',0,[],'',ammo='',weapon=constants['IT_AXE'],maximum=1),
                 row(module,65535,'weapon_shotgun','Weapon',0,[],'',ammo='item_shells',weapon=constants['IT_SHOTGUN'],maximum=1)])
    return rows,map_count


def c_tables(qsrc):
    rows=[]
    counts={}
    q3header=clean((qsrc/'quake-iii-arena/code/game/bg_public.h').read_text())
    weapon_names=re.findall(r'\bWP_\w+\b',q3header[q3header.index('WP_NONE'):q3header.index('WP_NUM_WEAPONS')])
    q3weapons={name:index for index,name in enumerate(weapon_names)}
    q2client=clean((qsrc/'quake-2/game/p_client.c').read_text())
    q2max={name:int(value) for name,value in re.findall(r'max_(\w+)\s*=\s*(\d+)',q2client)}
    for module,path,table in [(2,'quake-2/game/g_items.c','itemlist'),(3,'quake-iii-arena/code/game/bg_misc.c','bg_itemlist')]:
        source=clean((qsrc/path).read_text())
        initializer=block(source,re.search(r'\b'+table+r'\[\]\s*=',source).start())
        initializer=re.sub(r'^\s*#.*$', '', initializer, flags=re.MULTILINE)
        records=fields(initializer)
        count=0
        for number,record in enumerate(records):
            values=fields(block(record,0))
            if number==0 or len(values)==1:
                continue
            classname=text(values[0])
            if module==2:
                pickup=values[1]
                fire=values[4] if values[4]!='NULL' else ''
                sound=text(values[5]);models=[text(values[6])] if text(values[6]) else []
                label=text(values[10]);amount=int(values[12]);ammo_label=text(values[13]);flags=values[14]
                kind='WeaponAmmo' if 'IT_WEAPON' in flags and 'IT_AMMO' in flags else 'Weapon' if 'IT_WEAPON' in flags else 'Ammo' if 'IT_AMMO' in flags else 'Armor' if 'IT_ARMOR' in flags else 'Key' if 'IT_KEY' in flags else 'Health' if 'Health' in pickup or 'Adrenaline' in pickup or 'AncientHead' in pickup else 'Powerup'
                # Resolve authored pickup labels to classnames after the full table is read.
                ammo='@'+ammo_label if ammo_label else ''
                maximum=q2max.get(classname.removeprefix('ammo_'),0) if kind in ('Ammo','WeaponAmmo') else 1 if kind in ('Weapon','Key') else 0
                if kind=='Armor' and values[16].startswith('&'):
                    info=re.search(r'\b'+re.escape(values[16][1:])+r'\s*=',source)
                    armor=fields(block(source,info.start()))
                    amount,maximum=int(armor[0]),int(armor[1])
                native_weapon=number if kind in ('Weapon','WeaponAmmo') else 0
            else:
                sound=text(values[1]);models=[text(value) for value in fields(values[2][1:-1]) if text(value)]
                label=text(values[4]);amount=int(values[5]);flags=values[6];tag=values[7]
                kind={'IT_WEAPON':'Weapon','IT_AMMO':'Ammo','IT_ARMOR':'Armor','IT_HEALTH':'Health','IT_POWERUP':'Powerup','IT_HOLDABLE':'Powerup','IT_PERSISTANT_POWERUP':'Powerup','IT_TEAM':'Key'}[flags]
                ammo='@'+tag if kind=='Weapon' else ''
                pickup='';fire='';native_weapon=q3weapons.get(tag,0) if kind=='Weapon' else 0
                maximum=200 if kind in ('Ammo','Armor') else 1 if kind in ('Weapon','Key') else 0
            rows.append(row(module,number,classname,kind,amount,models,sound,label,ammo,native_weapon,maximum,pickup,fire))
            rows[-1]['tag']=values[7] if module==3 else ''
            count+=1
        counts[module]=count
    for item in rows:
        if item['ammo'].startswith('@'):
            name=item['ammo'][1:]
            candidates=[candidate for candidate in rows if candidate['module']==item['module'] and candidate['kind'] in ('Ammo','WeaponAmmo') and (candidate['label'].casefold()==name.casefold() if item['module']==2 else candidate['tag']==name)]
            if name and not candidates and item['classname'] not in ('weapon_gauntlet','weapon_grapplinghook'):
                raise ValueError('missing ammo: '+item['classname']+' '+name)
            item['ammo']=candidates[0]['classname'] if candidates else ''
    return rows,counts


def active_literal_sections(source):
    """These original tables disable one experimental entry with literal #if 0."""
    return re.sub(r'^\s*#if 0[^\n]*\n[\s\S]*?^\s*#endif[^\n]*$', '', source, flags=re.MULTILINE)


def rerelease(qsrc):
    directory = qsrc / 'quake2-rerelease-dll/rerelease'
    header = clean(active_literal_sections((directory / 'g_local.h').read_text()))
    source = clean(active_literal_sections((directory / 'g_items.cpp').read_text()))
    client = clean((directory / 'p_client.cpp').read_text())
    enum = fields(block(header, re.search(r'\benum item_id_t\s*:', header).start()))
    if any(not re.fullmatch(r'IT_\w+', name) for name in enum):
        raise ValueError('unsupported rerelease item enum declaration')
    ids = {name: number for number, name in enumerate(enum)}
    if ids.get('IT_NULL') != 0 or enum[-1] != 'IT_TOTAL':
        raise ValueError('rerelease item enum reserved rows changed')
    initial = re.search(r'max_ammo\.fill\((\d+)\)', client)
    if initial is None:
        raise ValueError('missing rerelease default ammo capacity')
    caps = {tag: int(value) for tag, value in re.findall(r'max_ammo\[(AMMO_\w+)\]\s*=\s*(\d+)\s*;', client)}
    initializer = block(source, re.search(r'\bitemlist\[\]\s*=', source).start())
    records = fields(initializer)
    if len(records) != ids['IT_TOTAL']:
        raise ValueError('rerelease item enum/table lengths differ')
    rows = []
    by_id = {}
    for number, record in enumerate(records):
        values = fields(block(record, 0))
        if number == 0:
            if values:
                raise ValueError('rerelease reserved item zero is not empty')
            continue
        if len(values) < 18 or ids.get(values[0]) != number:
            raise ValueError('rerelease item ordinal changed: ' + repr((number, values)))
        classname = text(values[1])
        flags = values[17]
        kind = ('WeaponAmmo' if 'IF_WEAPON' in flags and 'IF_AMMO' in flags else
                'Weapon' if 'IF_WEAPON' in flags else 'Ammo' if 'IF_AMMO' in flags else
                'Armor' if 'IF_ARMOR' in flags else 'Key' if 'IF_KEY' in flags else
                'Health' if 'IF_HEALTH' in flags else 'Powerup')
        amount = int(values[14])
        maximum = 1 if kind in ('Weapon', 'Key') else 0
        if kind in ('Ammo', 'WeaponAmmo'):
            if len(values) < 21 or not re.fullmatch(r'AMMO_\w+', values[20]):
                raise ValueError('missing rerelease ammo tag: ' + classname)
            maximum = caps.get(values[20], int(initial[1]))
        if kind == 'Armor' and len(values) > 19 and values[19].startswith('&'):
            info = re.search(r'\b' + re.escape(values[19][1:]) + r'\s*=', header)
            armor = fields(block(header, info.start()))
            amount, maximum = int(armor[0]), int(armor[1])
        model = text(values[7])
        fire = values[5] if values[5] != 'nullptr' else ''
        pickup = values[2] if values[2] != 'nullptr' else ''
        item = row(5, number, classname, kind, amount, [model] if model else [],
                   text(values[6]), text(values[12]), values[15],
                   number if kind in ('Weapon', 'WeaponAmmo') else 0,
                   maximum, pickup, fire)
        by_id[values[0]] = item
        rows.append(item)
    for item in rows:
        ammo = item['ammo']
        if ammo == 'IT_NULL':
            item['ammo'] = ''
        else:
            if ammo not in by_id:
                raise ValueError('missing rerelease ammo item: ' + ammo)
            item['ammo'] = by_id[ammo]['classname']
            if item['kind'] in ('Weapon', 'WeaponAmmo') and by_id[ammo]['kind'] not in ('Ammo', 'WeaponAmmo'):
                raise ValueError('rerelease weapon links non-ammo item: ' + item['classname'])
    return rows, len(rows)


def rust_string(value):
    return json.dumps(value,ensure_ascii=True)


def reference_counts(qsrc):
    counts=[]
    with tempfile.TemporaryDirectory(prefix='qa-item-count-') as temp:
        for module,path,table in [(2,'quake-2/game/g_items.c','itemlist'),(3,'quake-iii-arena/code/game/bg_misc.c','bg_itemlist'),(5,'quake2-rerelease-dll/rerelease/g_items.cpp','itemlist')]:
            source=clean(active_literal_sections((qsrc/path).read_text()))
            initializer=block(source,re.search(r'\b'+table+r'\[\]\s*=',source).start())
            token=re.compile(r'^[ \t]*#[^\n]*|"(?:\\.|[^"\\])*"(?:\s*"(?:\\.|[^"\\])*")*|&\s*\w+|\b[A-Za-z_]\w*\b',re.MULTILINE)
            enum=''
            if module==5:
                header=clean(active_literal_sections((qsrc/'quake2-rerelease-dll/rerelease/g_local.h').read_text()))
                enum=block(header,re.search(r'\benum item_id_t\s*:',header).start())
            converted=token.sub(lambda match:match[0] if match[0].lstrip().startswith('#') or (module==5 and match[0].startswith('IT_')) else '(uintptr_t)('+match[0]+')' if match[0].startswith('"') else '0',initializer)
            members=';'.join('uintptr_t f'+str(index)+('[4]' if module==3 and index==2 else '') for index in range({2:19,3:10,5:32}[module]))+';'
            prelude='enum {'+enum+'};\n' if module==5 else ''
            validation='if(sizeof(items)/sizeof(items[0])!=IT_TOTAL)return 1;for(size_t i=0;i<IT_TOTAL;i++)if(items[i].f0!=i)return 2;' if module==5 else ''
            reserved=1 if module==5 else 2
            program='#include <stdint.h>\n#include <stdio.h>\n#define MISSIONPACK 1\n'+prelude+'typedef struct {'+members+'} item;\nstatic item items[]={'+converted+'};\nint main(void){'+validation+'printf("%zu",sizeof(items)/sizeof(items[0])-'+str(reserved)+');}\n'
            candidate=Path(temp)/('count'+str(module)+'.c')
            executable=Path(temp)/('count'+str(module))
            candidate.write_text(program)
            subprocess.run(['cc','-w',str(candidate),'-o',str(executable)],check=True,timeout=300)
            counts.append(int(subprocess.check_output([str(executable)],text=True,timeout=300)))
    q1=len(re.findall(r'/\*QUAKED\s+(?:item|weapon)_\w+', (qsrc/'quake/progs106/items.qc').read_text()))
    qw=len(re.findall(r'/\*QUAKED\s+(?:item|weapon)_\w+', (qsrc/'quake/qw-qc/items.qc').read_text()))
    return [q1,qw,counts[0],counts[2],counts[1]]


def generate(qsrc):
    rows,q1_count=qc(qsrc,1,'quake/progs106')
    other,counts=c_tables(qsrc)
    rows+=other
    # Append additional native sources, retaining every existing common ID.
    qw,qw_count=qc(qsrc,4,'quake/qw-qc')
    rr,rr_count=rerelease(qsrc)
    rows+=qw+rr
    lines=['// Generated by tools/generate_items.py from original id sources.','use super::{ItemKind, ItemSource};','use qa_core::primitives::RuleSetId;','pub static ITEMS: &[ItemSource] = &[']
    for item in rows:
        models='&['+','.join(rust_string(model) for model in item['models'])+']'
        lines.append('ItemSource { source: RuleSetId::%(source)s, native: %(number)d, classname: %(classname)s, kind: ItemKind::%(kind)s, amount: %(amount)d, maximum: %(maximum)d, models: %(models)s, pickup_sound: %(sound)s, label: %(label)s, ammo: %(ammo)s, native_weapon: %(weapon)d, pickup: %(pickup)s, fire: %(fire)s },' % {**item,'source':{1:'Quake',2:'Quake2',3:'Quake3',4:'QuakeWorld',5:'Quake2Rerelease'}[item['module']],'models':models,**{key:rust_string(item[key]) for key in ['classname','sound','label','ammo','pickup','fire']}})
    lines+= ['];',f'pub const SOURCE_COUNTS: [usize;5] = [{q1_count},{qw_count},{counts[2]},{rr_count},{counts[3]}];','']
    return '\n'.join(lines),{'q1_map_items':q1_count,'q1_builtin_weapons':2,'qw_map_items':qw_count,'qw_builtin_weapons':2,'q2_itemlist_without_reserved_and_sentinel':counts[2],'q2_rerelease_itemlist_without_reserved':rr_count,'q3_itemlist_with_team_arena_without_reserved_and_sentinel':counts[3],'engine_items':len(rows)}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qsrc',type=Path,required=True)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    source,report=generate(args.qsrc)
    target=ROOT/'crates/gameplay/src/registry/generated.rs'
    with tempfile.TemporaryDirectory(prefix='qa-item-gen-') as temp:
        candidate=Path(temp)/'generated.rs'
        candidate.write_text(source)
        subprocess.run(['rustfmt','--edition','2024',str(candidate)],check=True,timeout=300)
        result=candidate.read_text()
    if args.check:
        if target.read_text()!=result:
            raise ValueError('item metadata differs from original sources')
        original=reference_counts(args.qsrc)
        expected=[report['q1_map_items'],report['qw_map_items'],report['q2_itemlist_without_reserved_and_sentinel'],report['q2_rerelease_itemlist_without_reserved'],report['q3_itemlist_with_team_arena_without_reserved_and_sentinel']]
        if original!=expected:
            raise ValueError('original declaration counts differ: '+repr(original))
        report['independent_reference_counts']=original
    else:
        target.parent.mkdir(parents=True,exist_ok=True)
        target.write_text(result)
    print(json.dumps(report,indent=2))


if __name__=='__main__':
    main()
