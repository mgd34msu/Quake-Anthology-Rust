import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_hull_trace import function


class ReferenceExtraction(unittest.TestCase):
    def test_definition_skips_prototype_and_if_call(self):
        source = '''float VectorNormalize(float *v);
void other(float *v) {
    if (VectorNormalize(v) == 0) { return; }
}
float VectorNormalize(float *v) { return v[0]; }
'''
        self.assertEqual(function(source, 'VectorNormalize'),
                         'float VectorNormalize(float *v) { return v[0]; }')

    def test_qualified_pointer_return_and_nested_body(self):
        source = 'static inline float *choose(float *v) { if (v) { return v; } return 0; }'
        self.assertEqual(function(source, 'choose'), source)

    def test_call_without_definition_is_rejected(self):
        with self.assertRaises(ValueError):
            function('void run(float *v) { if (VectorNormalize(v)) { return; } }', 'VectorNormalize')


if __name__ == '__main__':
    unittest.main()
