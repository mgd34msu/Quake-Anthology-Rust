import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_hull_trace import function
from check_draw_sort import function as draw_function


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

    def test_draw_definition_skips_forward_declaration_and_unrelated_body(self):
        source = '''qboolean GL_Upload8(byte *data);
void other(void) { int value = 1; }
qboolean GL_Upload8(byte *data) { return data[0]; }
'''
        body, first, last = draw_function(source, 'GL_Upload8')
        self.assertEqual(body, 'qboolean GL_Upload8(byte *data) { return data[0]; }')
        self.assertEqual((first, last), (3, 3))

    def test_draw_definition_masks_comments_and_literal_braces(self):
        source = '''void draw(void) {
    const char *text = "}";
    /* } */
    if (text) { return; }
}
'''
        body, first, last = draw_function(source, 'draw')
        self.assertEqual(body, source.rstrip())
        self.assertEqual((first, last), (1, 5))

    def test_draw_call_without_definition_is_rejected(self):
        with self.assertRaises(RuntimeError):
            draw_function('void run(void) { return draw(); }', 'draw')


if __name__ == '__main__':
    unittest.main()
