"""Owner CSV metadata compiler, ported from the C port's developer generator.

Source: quake-anthology/tools/generate_unified_cvars.py. Its parsing and
verification algorithm is retained; C headers, emitters and storage are absent.
These helpers run only during development, never inside the shipped engine.
"""
import re

COLUMNS = ('canonical aliases type range_units default_q1 default_qw default_q2 '
           'default_q2rr default_q3 flags owner effect_q1 effect_qw effect_q2 '
           'effect_q2rr effect_q3 conversion ts_status c_port_status '
           'rust_port_status sources').split()
DIALECTS = ('Q1', 'QW', 'Q2', 'Q2R', 'Q3')
TYPE_NAMES = ('float', 'int', 'bool', 'enum', 'string', 'bitmask')
TYPE_HINT, DEFAULT_POLICY, FLAG_POLICY, CONVERSION_POLICY, HOME_POLICY, PORT_DISAGREEMENT = (1, 2, 4, 8, 16, 32)
FLAG_BITS = {'archive': 1, 'userinfo': 2, 'serverinfo': 4, 'systeminfo': 8,
             'init': 16, 'noset': 16, 'latch': 32, 'readonly': 64, 'rom': 64,
             'temp': 256, 'cheat': 512, 'norestart': 1024}
AUX_BITS = {'private': 1, 'noarchive': 2, 'game': 4, 'files': 8,
            'refresh': 16, 'sound': 32, 'server-notify': 64}
KINDS = ('IDENTITY', 'RECIPROCAL', 'BOOL_INVERT', 'LINEAR', 'ENUM_DETAIL',
         'BIT_VIEW', 'COMPOSITE', 'RESOLUTION', 'CONSUMER_UNITS', 'SIDE_SCOPE', 'POLICY')
NUMBER = r'[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?'
NAME = r'[A-Za-z_][A-Za-z_0-9<>*-]*'
CONDITIONS = ('ALWAYS', 'MAC', 'NOT_MAC', 'LINUX', 'NOT_LINUX', 'DEDICATED', 'CLIENT', 'ENGINE', 'GAME', 'CGAME', 'UNRESOLVED', 'WINDOWS', 'NOT_WINDOWS')
OPERATIONS = ('NONE', 'KHZ_HZ', 'SKILL', 'VIEW_SIZE', 'BOOL_DETAIL', 'AUTOSWITCH', 'GUN', 'FOOTSTEPS', 'LAGOMETER', 'DRAW_2D', 'SHADOWS', 'OLD_RAIL', 'INPUT_GRAB', 'SOUND_BACKEND', 'NO_SKINS', 'FORCE_RESPAWN', 'DEATHMATCH', 'COOP', 'TEAMPLAY', 'CTF', 'QW_SKIN', 'SEX', 'COLOR', 'PLAYER_COLORS', 'NEEDPASS', 'SAME_LEVEL', 'NO_EXIT', 'DOWNLOAD', 'CLEAR_COLOR', 'FULLSCREEN', 'VIDEO_MODE', 'MUSIC_MUTE', 'SPECTATOR')


def aliases(cell):
    if cell == 'none':
        return []
    result = []
    for part in cell.split(';'):
        match = re.fullmatch(r'\s*([^\s();]+)\(([^)]*)\)\s*', part)
        if not match:
            raise ValueError('malformed alias: ' + part)
        games = match[2].split(',')
        if set(games) - set(DIALECTS) - {'TS', 'ports'}:
            raise ValueError('unknown alias source: ' + part)
        result.append((match[1], games))
    return result


class Catalog:
    def __init__(self, rows, policy):
        self.rows_input, self.policy = rows, policy
        self.pool = {'': 0}
        self.pool_reverse = {0: ''}
        self.pool_size = 1
        self.defaults, self.flags, self.conversions = [], [], []
        self.operands, self.maps, self.bindings, self.rows = [], [], [], []
        self.conversion_ids, self.issues = {}, []
        self.row_index = {row['canonical'].lower(): i for i, row in enumerate(rows)}
        self.names = {row['canonical'].lower(): row['canonical'] for row in rows}
        for row in rows:
            for name, unused in aliases(row['aliases']):
                self.names.setdefault(name.lower(), name)

    def text(self, value):
        if '\0' in value:
            raise ValueError('input contains NUL')
        if value not in self.pool:
            self.pool[value] = self.pool_size
            self.pool_reverse[self.pool_size] = value
            self.pool_size += len(value.encode('utf-8')) + 1
        return self.pool[value]

    def issue(self, row, field, reason, bits):
        self.issues.append({'row': row, 'name': self.rows_input[row]['canonical'],
                            'field': field, 'reason': reason, 'bits': bits})

    def default_clauses(self, row_index, dialect, cell):
        first = len(self.defaults)
        row = self.rows_input[row_index]
        canonical = row['canonical']
        kind, value = 0, cell
        if 'stored only' in cell:
            kind, value = 2, ''
        elif 'unset' in cell:
            kind = 1
            match = re.search(r'unset\s*=\s*(.*)', cell)
            value = match[1] if match else cell
        elif cell.startswith('Anthology default'):
            kind = 3
            value = cell[len('Anthology default'):].strip()
        members = list(re.finditer(r'(?:^|[;|]\s*)(' + NAME + r')\s*=\s*', value))
        chunks = [(canonical, value)]
        if members:
            chunks = [(m[1], value[m.end():members[n + 1].start() if n + 1 < len(members) else len(value)].strip(' ;|'))
                      for n, m in enumerate(members)]
        for member, raw_value in chunks:
            for value, condition, condition_kind in self.default_alternatives(raw_value, row['type'], kind):
                issues = DEFAULT_POLICY if condition_kind == 'UNRESOLVED' else 0
                if issues:
                    self.issue(row_index, 'default_' + DIALECTS[dialect], 'conditional, owner-dependent or symbolic default', issues)
                if value.startswith('"') and value.endswith('"') and value.count('"') == 2:
                    value = value[1:-1]
                self.defaults.append((self.text(member), self.text(value), self.text(condition),
                                      self.text(cell), issues, kind, CONDITIONS.index(condition_kind)))
        return first, len(self.defaults) - first

    @staticmethod
    def default_alternatives(raw, type_hint, kind):
        if kind == 2:
            return [('', '', 'ALWAYS')]
        value = r'(?:' + NUMBER + r'|"[^"\n]*")'
        # Only alternatives stated literally in the owner input become selectors.
        platform_value = value if type_hint != 'string' else r'(?:' + value + r'|[A-Za-z_][A-Za-z_0-9.-]*)'
        operating_system = re.fullmatch(r'(' + platform_value + r') \((' + platform_value + r') on (__(?:MACOS|linux)__|Linux|Windows)\)', raw)
        if operating_system:
            condition = {'__MACOS__': 'MAC', '__linux__': 'LINUX', 'Linux': 'LINUX', 'Windows': 'WINDOWS'}[operating_system[3]]
            return [(operating_system[1], operating_system[0], 'NOT_' + condition),
                    (operating_system[2], operating_system[0], condition)]
        roles = list(re.finditer(r'(' + value + r') \((game|server|engine|cgame)\)', raw))
        if roles and re.fullmatch(r'(?:' + value + r' \((?:game|server|engine|cgame)\)\s*(?:[/;]\s*)?)+', raw):
            return [(match[1], match[0], 'ENGINE' if match[2] == 'server' else match[2].upper()) for match in roles]
        role_pair = re.fullmatch(r'(' + value + r') \((' + value + r') in game\)', raw)
        if role_pair:
            return [(role_pair[1], role_pair[0], 'ENGINE'), (role_pair[2], role_pair[0], 'GAME')]
        client_dedicated = re.fullmatch(r'(' + value + r') on client builds, (' + value + r') on dedicated servers', raw)
        if client_dedicated:
            return [(client_dedicated[1], raw, 'CLIENT'), (client_dedicated[2], raw, 'DEDICATED')]
        dedicated_client = re.fullmatch(r'(' + value + r') on dedicated servers, else (' + value + r')', raw)
        if dedicated_client:
            return [(dedicated_client[1], raw, 'DEDICATED'), (dedicated_client[2], raw, 'CLIENT')]
        dedicated_build = re.fullmatch(r'(' + value + r') \((' + value + r') and CVAR_ROM when DEDICATED\)', raw)
        if dedicated_build:
            return [(dedicated_build[1], raw, 'CLIENT'), (dedicated_build[2], raw, 'DEDICATED')]
        condition = ''
        if '(' in raw:
            condition = raw[raw.index('('):]
            before = raw[:raw.index('(')].strip()
            # Parenthetic source names, neutral/stock notes and numeric macros
            # describe a literal, while alternative values/platforms are policy.
            informational = bool(re.fullmatch(r'\((?:neutral|stock(?:[^()]*)?|none|server|PORT_CLIENT|pm_[A-Za-z_]+|CTF_DEFAULT_[A-Za-z_]+|not defined in QC; engine supplies)\)', condition))
            if informational and re.fullmatch(value, before):
                return [(before, condition, 'ALWAYS')]
            return [(before, condition, 'UNRESOLVED')]
        dynamic = bool(re.search(r'\s[|/]\s|\b(?:if|else|when|depending|registers|never registered|random|built|current|build-time)\b', raw))
        dynamic |= any(token in raw for token in ('__', '<', '()', 'MAX_', 'DEFAULT_', 'OPENGL_DRIVER_NAME'))
        if type_hint != 'string' and raw and not re.fullmatch(NUMBER, raw):
            dynamic = True
        if kind != 2 and not raw:
            dynamic = True
        return [(raw, condition, 'UNRESOLVED' if dynamic else 'ALWAYS')]

    def flag_clauses(self, row_index, cell):
        grouped = [[] for unused in DIALECTS]
        anthology = []
        for section in cell.split('||'):
            section = section.strip()
            match = re.match(r'(Q1|QW|Q2R|Q2|Q3|Anthology):?\s*(.*)', section)
            if not match:
                self.issue(row_index, 'flags', 'unknown dialect section preserved in raw_flags', FLAG_POLICY)
                continue
            game, body = match.groups()
            clauses = list(re.finditer(r'(?:^|;\s*)(' + NAME + r')\s*:\s*', body))
            if not clauses:
                self.issue(row_index, 'flags', 'unparsed dialect flags preserved verbatim', FLAG_POLICY)
                continue
            entries = []
            for n, item in enumerate(clauses):
                raw = body[item.end():clauses[n + 1].start() if n + 1 < len(clauses) else len(body)].strip(' ;')
                tokens = re.findall(r'[A-Za-z_-]+', re.sub(r'\([^)]*\)', '', raw))
                flags, aux, issues = 0, 0, 0
                for token in tokens:
                    token = token.lower().removeprefix('cvar_')
                    if token in FLAG_BITS:
                        flags |= FLAG_BITS[token]
                    elif token in AUX_BITS:
                        aux |= AUX_BITS[token]
                    elif token not in ('none',):
                        issues |= FLAG_POLICY
                if re.search(r'[()/;]|\bon\b|\bwhen\b', raw):
                    issues |= FLAG_POLICY
                # Parsed bits are the mentioned union, not a resolved alternative.
                if issues:
                    self.issue(row_index, 'flags_' + game, 'conditional/owner flag alternatives; bits are only the mentioned union', issues)
                entries.append((self.text(item[1]), self.text(raw), flags, aux, issues))
            if game == 'Anthology':
                anthology.extend(entries)
            else:
                grouped[DIALECTS.index(game)].extend(entries)
        return grouped, anthology

    def add_conversion(self, kind='IDENTITY', scale=1, offset=0, lower=0, upper=0,
                       raw='', detail=0, role=0, issues=0, operands=(), maps=(), operation='NONE'):
        key = (kind, scale, offset, lower, upper, raw, detail, role, issues, tuple(operands), tuple(maps), operation)
        if key in self.conversion_ids:
            return self.conversion_ids[key]
        first_operand, first_map = len(self.operands), len(self.maps)
        self.operands.extend(operands)
        self.maps.extend(maps)
        entry = (scale, offset, lower, upper, self.text(raw), first_operand,
                 first_map, issues, len(operands), len(maps), KINDS.index(kind), detail, role, OPERATIONS.index(operation))
        index = len(self.conversions)
        self.conversions.append(entry)
        self.conversion_ids[key] = index
        return index

    def conversion(self, row_index, name, dialect, canonical):
        row = self.rows_input[row_index]
        raw, target = row['conversion'], row['canonical']
        lower = raw.lower()
        options = {'raw': raw}
        operation = self.operation(row_index, name, dialect, canonical)
        if operation != 'NONE':
            options['operation'] = operation
        if operation == 'TEAMPLAY' and dialect == 0 and not canonical:
            assignment = re.search(r'Q1 teamplay 1/2 also set (' + NAME + r') 0/1', raw)
            if not assignment:
                raise ValueError('Q1 teamplay policy lost its paired friendly-fire assignment')
            options['operands'] = ((self.row_index[assignment[1].lower()], 0, 0, 0, (0, 0, 0, 0, 0)),)
        if operation == 'CTF':
            options['issues'] = CONVERSION_POLICY
        if 'UNIT CONFLICT on the same name' in raw:
            if 'pixels/second' in raw and dialect in (0, 1):
                return self.add_conversion('LINEAR', scale=100, **options)
            if 'seconds-per-character' in raw and dialect == 3:
                return self.add_conversion('RECIPROCAL', role=1, **options)
            return self.add_conversion(**options)
        if canonical:
            if raw.startswith('NOT stored.'):
                return self.add_conversion('COMPOSITE', operands=self.dmflags_operands(), **options)
            # These MD conflicts explicitly refine the CSV's inferred identity.
            conflict = re.search(r'`([^`]+)` is a password string in QW and a 0/1 bool in Q2/Q2R', self.policy)
            if conflict and target == conflict[1] and dialect in (2, 3):
                return self.add_conversion('ENUM_DETAIL', detail=1, **options)
            if raw.startswith('Bit view of dmflags:'):
                return self.add_conversion('BIT_VIEW', operands=self.rule_operand(row_index), **options)
            if operation != 'NONE':
                return self.add_conversion('ENUM_DETAIL' if operation not in ('CLEAR_COLOR', 'VIEW_SIZE') else 'POLICY', detail=int('detail' in lower), **options)
            return self.add_conversion(**options)
        escaped = re.escape(name)
        # Text patterns name the alias in the input; no maintained alias/default list.
        if re.search(escaped + r'\s*(?:=|is[^.]*:)\s*(?:' + re.escape(target) + r')\s*/\s*(' + NUMBER + ')', raw):
            divisor = float(re.search(escaped + r'\s*(?:=|is[^.]*:)\s*(?:' + re.escape(target) + r')\s*/\s*(' + NUMBER + ')', raw)[1])
            return self.add_conversion('LINEAR', scale=1 / divisor, **options)
        multiplier = re.search(escaped + r'\s*=\s*' + re.escape(target) + r'\s*x\s*(' + NUMBER + ')', raw)
        if multiplier:
            return self.add_conversion('LINEAR', scale=float(multiplier[1]), **options)
        if re.search(escaped + r'\s*=\s*-' + re.escape(target), raw) or 'sign flip' in lower:
            return self.add_conversion('LINEAR', scale=-1, **options)
        if 'inverse convention' in lower or re.search(escaped + r'\s*=\s*1/' + re.escape(target), raw):
            return self.add_conversion('RECIPROCAL', **options)
        if re.search(escaped + r'[^.;]*(?:inverted|!\s*' + re.escape(target) + ')', raw, re.I):
            detail = int('detail slot' in lower)
            return self.add_conversion('BOOL_INVERT', detail=detail, **options)
        if '11<->11025' in raw and re.search(escaped + r' is Hz', raw):
            pairs = [(float(hz), float(khz), 0) for khz, hz in re.findall(r'(\d+)<->(\d+)', raw)]
            return self.add_conversion('ENUM_DETAIL', maps=pairs, **options)
        if 'skill = clamp' in raw:
            return self.add_conversion('LINEAR', offset=-1, lower=0, upper=3, **options)
        if 'SIDE-SCOPED' in raw:
            return self.add_conversion('SIDE_SCOPE', detail=int('detail' in lower), **options)
        if 'mode tables differ' in lower or 'vid_modelist index' in lower and dialect == 3:
            return self.add_conversion('RESOLUTION', issues=CONVERSION_POLICY, **options)
        if operation == 'COLOR':
            return self.add_conversion('ENUM_DETAIL', detail=1, maps=self.color_maps(raw), **options)
        if operation == 'PLAYER_COLORS':
            operand_names = re.findall(r'(?:high|low) nibble -> (' + NAME + ')', raw)
            operands = tuple((self.row_index[n.lower()], 0, 0, 0, (0, 0, 0, 0, 0)) for n in operand_names)
            return self.add_conversion('COMPOSITE', detail=1, operands=operands, maps=self.color_maps(raw), **options)
        if re.search(escaped + r': (?:bool )?identity[.]', raw) or re.search(escaped + r' identity[.]', raw):
            options.pop('operation', None)
            return self.add_conversion(**options)
        if operation == 'NONE' and (lower.startswith('identity') or re.search(escaped + r'(?:[^.;]*): (?:bool )?identity', raw) or re.search(escaped + r' identity', raw) or 'Q2/Q2R dialect: skin == model string (identity)' in raw or 'Q3 dialect reads (needpass != 0)' in raw or 'vid_modelist index' in lower and dialect != 3):
            return self.add_conversion(**options)
        if ('detail' in lower or 'skin part' in lower or 'neuter' in lower or 'last nonzero' in lower
                or 'g_gametype' in lower or 'g_forcerespawn>0' in raw):
            if operation == 'NONE':
                options['issues'] = CONVERSION_POLICY
            return self.add_conversion('ENUM_DETAIL', detail=int('detail' in lower), **options)
        if operation != 'NONE':
            return self.add_conversion('ENUM_DETAIL', detail=int('detail' in lower), **options)
        if re.match(escaped + r' = ' + re.escape(target) + r' \(', raw):
            return self.add_conversion(**options)
        if raw.startswith('none:') or lower.startswith('identity') or 'identity' in lower or ' = ' in raw and 'same function' in lower:
            return self.add_conversion(**options)
        return self.add_conversion('POLICY', issues=CONVERSION_POLICY, **options)

    def operation(self, row_index, name, dialect, canonical):
        raw = self.rows_input[row_index]['conversion']
        lower = raw.lower()
        named = re.escape(name)
        if '30..100' in raw and 'status bar' in lower:
            return 'VIEW_SIZE'
        if 'Q3 dialect reads (needpass != 0)' in raw and dialect == 4:
            return 'NEEDPASS'
        if 'Numeric values are Quake palette indexes' in raw:
            return 'CLEAR_COLOR'
        if 'password string in QW' in self.policy and canonical:
            conflict = re.search(r'`([^`]+)` is a password string in QW', self.policy)
            if conflict and self.rows_input[row_index]['canonical'] == conflict[1] and dialect in (2, 3):
                return 'SPECTATOR'
        if canonical:
            return 'NONE'
        if '11<->11025' in raw:
            return 'KHZ_HZ'
        if 'skill = clamp' in raw:
            return 'SKILL'
        if 'mode tables differ' in lower:
            return 'VIDEO_MODE'
        if 'vid_modelist index' in lower and dialect == 3:
            return 'FULLSCREEN'
        if 'SIDE-SCOPED' in raw:
            return 'DOWNLOAD' if 'download' in lower else 'NONE'
        if 'handedness override' in lower and re.search(named + r': 0->0', raw):
            return 'GUN'
        if 'DF_NO_FOOTSTEPS' in raw:
            return 'FOOTSTEPS'
        if 'netgraph strip' in lower:
            return 'LAGOMETER'
        if 'status bar suppressed' in lower and not re.search(named + r' is inverted', raw):
            return 'DRAW_2D'
        if 'shadow style' in lower:
            return 'SHADOWS'
        if 'baseskin' in lower:
            return 'NO_SKINS'
        if 'autoswitch:' in lower:
            return 'AUTOSWITCH'
        if 'style kept in alias detail slot' in lower:
            return 'OLD_RAIL'
        if re.search(named + r'[^.;]*(?:inverted|!\s*' + re.escape(self.rows_input[row_index]['canonical']) + ')', raw, re.I) and 'exits kill' not in lower:
            return 'NONE'
        if 'smart release' in lower and re.search(named + r' \(Q2R\)', raw):
            return 'INPUT_GRAB'
        if 'backend choice' in lower and re.search(named + r' \(Q2R\)', raw):
            return 'SOUND_BACKEND'
        if 'last nonzero value' in lower and re.search(named + r' was a 0/1 mute', raw):
            return 'MUSIC_MUTE'
        if 'reads g_forcerespawn>0' in raw and re.search(named + r' \(bool', raw):
            return 'FORCE_RESPAWN'
        if 'deathmatch: write' in raw:
            # Alias names are parsed from the input's named clauses.
            for clause in raw.split(';'):
                if re.search(r'(?:^|[. ]+)' + named + r'(?: \(Q2R\))?:', clause):
                    if '(FFA)' in clause:
                        return 'DEATHMATCH'
                    if '9' in clause and 'only if current is 9' in clause:
                        return 'COOP'
                    if 'team DM' in clause:
                        return 'TEAMPLAY'
                    if '1 -> 4' in clause:
                        return 'CTF'
            return 'BOOL_DETAIL'
        if 'samelevel identity' in raw:
            return 'SAME_LEVEL'
        if 'exits kill' in lower:
            return 'NO_EXIT'
        if 'skin reads/writes only the skin part' in lower and dialect == 1:
            return 'QW_SKIN'
        if 'gender none <-> sex neuter' in raw:
            return 'SEX'
        if 'high nibble' in lower and name in re.search(r'([^.]*?) is composite:', raw)[1].split():
            return 'PLAYER_COLORS'
        if 'nonzero -> 1' in raw and not re.search(r'identity for ' + named + r'[.]', raw):
            return 'BOOL_DETAIL'
        if 'colour table' in lower or 'topcolor table' in lower or 'index by table' in lower:
            return 'COLOR'
        if lower.startswith('identity') and not re.search(named, raw):
            return 'NONE'
        if 'detail slot' in lower or 'kept as detail' in lower:
            return 'BOOL_DETAIL'
        return 'NONE'

    def color_maps(self, raw):
        reference = re.search(r'\(see (' + NAME + r')\)', raw)
        if reference:
            raw = self.rows_input[self.row_index[reference[1].lower()]]['conversion']
        table = re.search(r'by table ([^;]+); reverse ([^;]+);', raw)
        if not table:
            raise ValueError('owner colour policy has no complete forward/reverse table')
        forward = [(float(alias), float(canonical), 0) for alias, canonical in re.findall(r'(\d+)->(\d+)', table[1])]
        reverse = [(float(alias), float(canonical), 1) for canonical, alias in re.findall(r'(\d+)->(\d+)', table[2])]
        if len(forward) != 14 or len(reverse) != 7:
            raise ValueError('owner colour table must define14forward/7reverse values')
        return tuple(forward + reverse)

    def rule_operand(self, row_index):
        raw = self.rows_input[row_index]['conversion']
        match = re.search(r'(?:unified bit|dmflags bit) (\d+)', raw)
        if not match:
            return ()
        bit = int(match[1])
        q3 = re.search(r'Q3 bit (\d+)', raw)
        classic = bit if bit <= 0x100000 else 0
        return ((row_index, int(bool('Bit is inverted relative' in raw or re.search(r'dmflags bit \d+ view \(inverted', raw))), int('g_forcerespawn>0' in raw), bit,
                 (classic, classic, classic, classic, int(q3[1]) if q3 else 0)),)

    def dmflags_operands(self):
        result = []
        for i, row in enumerate(self.rows_input):
            operands = self.rule_operand(i)
            if operands:
                result.extend(operands)
            elif 'dmflags bit 1024' in row['conversion']:
                result.append((i, 0, 1, 1024, (1024, 1024, 1024, 1024, 0)))
        bits = [entry[3] for entry in result]
        if len(bits) != len(set(bits)) or len(bits) != 22:
            raise ValueError('dmflags input does not define every unique unified rule bit: ' + str(bits))
        return tuple(sorted(result, key=lambda item: item[3]))

    def build(self):
        self.add_conversion()
        for index, row in enumerate(self.rows_input):
            alias_entries = aliases(row['aliases'])
            grouped_flags, anthology = self.flag_clauses(index, row['flags'])
            dialect_data, native = [], 0
            mentioned = 0
            issues = TYPE_HINT if not alias_entries else 0
            if issues:
                self.issue(index, 'type/range_units', 'single-name inferred hints; not runtime validators', issues)
            for game in range(5):
                cell = row[COLUMNS[4 + game]]
                first_default, count_default = self.default_clauses(index, game, cell)
                flags = grouped_flags[game] or anthology
                first_flags = len(self.flags)
                self.flags.extend(flags)
                raw_flags = ' ; '.join(self.pool_text(entry[1]) for entry in grouped_flags[game])
                dialect_data.append((self.text(cell), self.text(raw_flags), self.text(row[COLUMNS[11 + game]]),
                                     first_default, first_flags, count_default, len(flags)))
                if not ('not native' in cell or 'unset' in cell or cell.startswith('Anthology')):
                    native |= 1 << game
                for entry in flags:
                    mentioned |= entry[2] & 1
                    issues |= entry[4]
                for entry in self.defaults[first_default:first_default + count_default]:
                    issues |= entry[4]
            canonical_native = []
            for game, entries in enumerate(grouped_flags):
                if any(self.pool_text(entry[0]).lower() == row['canonical'].lower() for entry in entries):
                    canonical_native.append(game)
            home = 4 if 4 in canonical_native else canonical_native[0] if canonical_native else 255
            if home == 255:
                issues |= HOME_POLICY
                self.issue(index, 'owner', 'no native canonical home dialect; Anthology/guest owner decides', HOME_POLICY)
            elif len(canonical_native) > 1 and home != 4:
                flagsets = {entry[2:4] for game in canonical_native for entry in grouped_flags[game]}
                if len(flagsets) > 1:
                    issues |= HOME_POLICY
                    self.issue(index, 'owner', 'several native homes have different flags; first home is advisory', HOME_POLICY)
            disagreements = re.search(r'\*\*Port default disagreement:\*\*(.*)', self.policy)
            if disagreements and row['canonical'] in re.findall(r'`([^`]+)`', disagreements[1]):
                issues |= PORT_DISAGREEMENT
                self.issue(index, 'ports', 'owner policy explicitly records unresolved port default/flag disagreement', PORT_DISAGREEMENT)
            family = re.search(r'<(\d+)-(\d+)>', row['canonical'])
            family_count = int(family[2]) - int(family[1]) + 1 if family else 1
            names = [(row['canonical'], [], True)] + [(name, games, False) for name, games in alias_entries]
            for name, games, canonical in names:
                scope = 0
                if row['conversion'].startswith('SIDE-SCOPED'):
                    if 'on the server registry ' + name + ' aliases ' + row['canonical'] in row['conversion']:
                        scope = 2
                    elif 'Client-side meaning' in row['conversion']:
                        scope = 1
                if canonical:
                    scope = 0
                conversions = tuple(self.conversion(index, name, game, canonical) for game in range(5))
                member_native = sum(1 << game for game in canonical_native) if canonical else sum(1 << DIALECTS.index(game) for game in games if game in DIALECTS)
                expanded = [(name, 0)]
                if family:
                    expanded = [(name.replace(family[0], str(seat)), seat) for seat in range(int(family[1]), int(family[2]) + 1)]
                for expanded_name, seat in expanded:
                    self.bindings.append((self.text(expanded_name), index, conversions, scope, int(canonical), member_native, seat))
            rule = self.conversion(index, row['canonical'], 4, True)
            policies = int('Latched: applies at next map.' in row['conversion'])
            if 'PRIVATE (never echoed or sent in info strings)' in row['conversion']:
                policies |= 2
            raw_texts = [self.text(row[key]) for key in ('canonical', 'aliases', 'range_units', 'owner', 'conversion', 'sources', 'flags')]
            statuses = [self.text(row[key]) for key in ('ts_status', 'c_port_status', 'rust_port_status')]
            self.rows.append((raw_texts, statuses, dialect_data, issues, mentioned, policies, rule,
                              TYPE_NAMES.index(row['type']), home, family_count, int(row['conversion'].startswith('NOT stored.'))))
        self.bindings.sort(key=lambda binding: (self.pool_text(binding[0]).lower(), binding[3], binding[1]))
        collisions = {}
        for item in self.bindings:
            name = self.pool_text(item[0]).lower()
            collisions.setdefault(name, []).append(item)
        for name, entries in collisions.items():
            if len(entries) > 1 and (len(entries) != 2 or not all(entry[3] for entry in entries) and not any(entry[3] == 2 for entry in entries)):
                raise ValueError('unscoped binding collision: ' + name)
        for i, row in enumerate(self.rows_input):
            if any(self.conversions[binding[2][game]][7] for binding in self.bindings if binding[1] == i for game in range(5)):
                self.issue(i, 'conversion', 'typed conversion requires policy implementation/unsupplied lookup data; raw prose retained', CONVERSION_POLICY)
                a = list(self.rows[i]); a[3] |= CONVERSION_POLICY; self.rows[i] = tuple(a)

    def pool_text(self, offset):
        return self.pool_reverse[offset]

    def verify(self):
        for index, entry in enumerate(self.rows):
            raw, status, dialects = entry[:3]
            original = self.rows_input[index]
            restored = dict(zip(('canonical', 'aliases', 'range_units', 'owner', 'conversion', 'sources', 'flags'), (self.pool_text(x) for x in raw)))
            restored.update(zip(('ts_status', 'c_port_status', 'rust_port_status'), (self.pool_text(x) for x in status)))
            restored['type'] = TYPE_NAMES[entry[7]]
            for d, values in enumerate(dialects):
                restored[COLUMNS[4 + d]] = self.pool_text(values[0])
                restored[COLUMNS[11 + d]] = self.pool_text(values[2])
                if values[3] + values[5] > len(self.defaults) or values[4] + values[6] > len(self.flags):
                    raise ValueError('dialect clause span exceeds generated array')
            if restored != original:
                raise ValueError('input cell changed or dropped: ' + original['canonical'])
            names = [original['canonical']] + [name for name, games in aliases(original['aliases'])]
            family = re.search(r'<(\d+)-(\d+)>', original['canonical'])
            expanded = [name.replace(family[0], str(seat)) for name in names for seat in range(int(family[1]), int(family[2]) + 1)] if family else names
            actual = [self.pool_text(item[0]) for item in self.bindings if item[1] == index]
            if sorted(expanded) != sorted(actual):
                raise ValueError('canonical/alias/family binding coverage changed')
        if len(self.rows) >= 65535 or len(self.conversions) >= 65535:
            raise ValueError('generated catalog exceeds16-bit indices')
        for entry in self.conversions:
            if entry[5] + entry[8] > len(self.operands) or entry[6] + entry[9] > len(self.maps):
                raise ValueError('conversion span exceeds generated array')
        for entry in self.operands:
            if entry[0] >= len(self.rows):
                raise ValueError('composite operand has no canonical row')
        # Inputs contain no embedded NUL; every pooled offset names a whole cell.
        if sum(len(value.encode('utf-8')) + 1 for value in self.pool) != self.pool_size:
            raise ValueError('string pool offset mismatch')
