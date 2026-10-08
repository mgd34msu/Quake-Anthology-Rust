import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_hull_trace import function
from check_draw_sort import function as draw_function
from check_brush_tree import extract_function as tree_function


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

    def test_tree_bspc_branches_share_one_closing_brace(self):
        body = '''void CM_TestInLeaf(void) {
#ifdef BSPC
    if (1) {
#else
    if (!cm_noCurves->integer) {
#endif //BSPC
        nested();
    }
}'''
        source = 'void forward(void);\n' + body + '\nvoid after(void) { }\n'
        extracted, span = tree_function(source, 'CM_TestInLeaf')
        self.assertEqual(extracted, body)
        self.assertEqual((span['start_line'], span['end_line']), (2, 10))
        self.assertTrue(span['exact_contiguous_source_slice'])
        self.assertIn('#ifdef BSPC', extracted)
        self.assertIn('#else', extracted)

    def test_tree_nested_default_and_elif_branches_preserve_all_bytes(self):
        body = '''void trace(void) {
#if 0
    {{{
#elif defined(ALWAYS_CAPSULE_VS_CAPSULE)
    }}
#else
#ifndef BSPC
    if (1) {
#ifdef CAPSULE_DEBUG
        }}{{
#endif
        const char *text = "}"; // }
    }
#endif
#endif
}'''
        extracted, _ = tree_function(body + '\nvoid next(void) { }', 'trace')
        self.assertEqual(extracted, body)

    def test_tree_conditional_crossing_function_boundary_is_rejected(self):
        with self.assertRaises(ValueError):
            tree_function('void trace(void) {\n#ifndef BSPC\n}\n', 'trace')

    def test_tree_unsupported_conditional_is_rejected(self):
        with self.assertRaises(ValueError):
            tree_function('void trace(void) {\n#if UNKNOWN + 1\n{}\n#endif\n}', 'trace')


if __name__ == '__main__':
    unittest.main()
