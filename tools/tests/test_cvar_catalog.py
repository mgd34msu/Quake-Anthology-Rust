import copy
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_cvars import compare_cells
from cvar_catalog import Catalog, COLUMNS
from gen_cvars import load_catalog

ROOT = Path(__file__).resolve().parents[2]


def encoded_rows(rows):
    output = bytearray(struct.pack('<I', len(rows)))
    for row in rows:
        for cell in row.values():
            value = cell.encode('utf-8')
            output.extend(struct.pack('<I', len(value)))
            output.extend(value)
    return bytes(output)


class CvarCatalog(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.catalog, cls.owner, cls.engine = load_catalog(ROOT)

    def test_owner_cells_and_existing_typed_metadata_remain_unchanged(self):
        self.assertEqual(len(self.owner), 1260)
        self.assertEqual(sum(len(row) for row in self.owner), 26460)
        original = Catalog(self.owner, (ROOT/'data/unified-cvars.md').read_text())
        original.build()
        original.verify()
        self.assertEqual(self.catalog.rows[:len(self.owner)], original.rows)
        self.assertEqual(self.catalog.defaults[:len(original.defaults)], original.defaults)
        self.assertEqual(self.catalog.flags[:len(original.flags)], original.flags)
        self.assertEqual(self.catalog.conversions[:len(original.conversions)], original.conversions)
        self.assertEqual([binding for binding in self.catalog.bindings
                          if binding[1] < len(self.owner)], original.bindings)

    def test_cpu_bands_has_extension_provenance_and_all_source_flags(self):
        row = next(row for row in self.engine if row['canonical'] == 'r_cpuBands')
        self.assertEqual(row['aliases'], 'none')
        self.assertTrue(row['sources'].startswith('engine-extension:'))
        for column in COLUMNS[4:9]:
            self.assertEqual(row[column], 'Anthology default 0')
        index = len(self.owner) + self.engine.index(row)
        definition = self.catalog.rows[index]
        self.assertEqual(definition[8], 255)
        bindings = [binding for binding in self.catalog.bindings if binding[1] == index]
        self.assertEqual(len(bindings), 1)
        self.assertEqual(self.catalog.pool_text(bindings[0][0]), 'r_cpuBands')
        self.assertEqual(bindings[0][5], 0)
        for source in definition[2]:
            for default in self.catalog.defaults[source[3]:source[3]+source[5]]:
                self.assertEqual(self.catalog.pool_text(default[1]), '0')
                self.assertEqual(default[5], 3)
            clauses = self.catalog.flags[source[4]:source[4]+source[6]]
            self.assertEqual(len(clauses), 1)
            self.assertEqual(clauses[0][2:5], (1 | 32, 0, 0))

    def test_engine_canonical_collision_is_case_insensitive(self):
        engine = copy.deepcopy(self.engine)
        engine[0]['canonical'] = self.owner[0]['canonical'].upper()
        with patch('gen_cvars.read_rows', side_effect=[self.owner, engine]):
            with self.assertRaises(ValueError):
                load_catalog(ROOT)

    def test_engine_metadata_cannot_claim_native_provenance(self):
        for field, value in [('sources', 'Q3 renderer/tr_init.c'), ('default_q3', '0')]:
            with self.subTest(field=field):
                engine = copy.deepcopy(self.engine)
                engine[0][field] = value
                with patch('gen_cvars.read_rows', side_effect=[self.owner, engine]):
                    with self.assertRaises(ValueError):
                        load_catalog(ROOT)

    def test_full_cell_oracle_reports_owner_and_extension_counts_separately(self):
        report = compare_cells(encoded_rows(self.owner + self.engine), self.owner, self.engine)
        self.assertEqual((report['rows'], report['cells']), (1260, 26460))
        self.assertEqual((report['engine_rows'], report['engine_cells']), (1, 21))

    def test_full_cell_oracle_rejects_owner_extension_and_trailing_changes(self):
        rows = self.owner + self.engine
        for index in [0, len(self.owner)]:
            with self.subTest(index=index):
                changed = copy.deepcopy(rows)
                changed[index]['default_q3'] = 'different-value'
                with self.assertRaises(ValueError):
                    compare_cells(encoded_rows(changed), self.owner, self.engine)
        with self.assertRaises(ValueError):
            compare_cells(encoded_rows(rows) + b'\0', self.owner, self.engine)


if __name__ == '__main__':
    unittest.main()
