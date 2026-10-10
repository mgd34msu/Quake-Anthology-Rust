use qa_core::primitives::{Plane, Vec3};
use qa_formats::bsp::{Bsp, Face, IndexRange, Map, TextureInfo, Vertex};
use qa_render::world::geometry::grid::{GridFan, GridOptions, subdivide_surface};
use qa_render::world::geometry::{GeometryOptions, WorldGeometry, load_geometry};

fn geometry(points: &[[f32; 3]], offset: [f32; 2]) -> WorldGeometry {
    let header = 4 + 15 * 8;
    let mut bytes = vec![0; header];
    bytes[..4].copy_from_slice(&29u32.to_le_bytes());
    for lump in 0..15 {
        bytes[4 + lump * 8..8 + lump * 8].copy_from_slice(&(header as u32).to_le_bytes());
    }
    let count = points.len() as u32;
    let map = Map {
        bsp: Bsp::parse(&bytes).unwrap(),
        planes: vec![Plane {
            encoding: None,
            normal: Vec3([0.0, 0.0, 1.0]),
            distance: 0.0,
            axis: None,
        }],
        vertices: points
            .iter()
            .map(|&position| Vertex {
                position: Vec3(position),
                normal: Vec3([0.0, 0.0, 1.0]),
                texcoord: [0.0; 2],
                lightmap_coord: [0.0; 2],
                color: [255; 4],
            })
            .collect(),
        nodes: vec![],
        leaves: vec![],
        edges: (0..count).map(|i| [i, (i + 1) % count]).collect(),
        surface_edges: (0..count as i32).collect(),
        faces: vec![Face {
            plane: 0,
            flags: 0,
            edges: IndexRange { first: 0, count },
            texture_info: 0,
            styles: [255; 4],
            lighting_offset: -1,
        }],
        leaf_faces: vec![],
        leaf_brushes: vec![],
        clipnodes: vec![],
        texture_info: vec![TextureInfo {
            projection: [[1.0, 0.0, 0.0, offset[0]], [0.0, 1.0, 0.0, offset[1]]],
            flags: 0,
            texture: 0,
            value: 0,
            next: -1,
            name: &[],
        }],
        textures: vec![],
        models: vec![],
        brushes: vec![],
        brush_sides: vec![],
        shaders: vec![],
        fogs: vec![],
        surfaces: vec![],
        indices: vec![],
        areas: vec![],
        area_portals: vec![],
        extensions: vec![],
    };
    load_geometry(&map, GeometryOptions::default()).unwrap()
}

#[test]
fn grid_subdivision_replaces_gl_triangles_and_preserves_cpu_polygon() {
    let points = [
        [-64.0, -64.0, 0.0],
        [192.0, -64.0, 0.0],
        [192.0, 192.0, 0.0],
        [-64.0, 192.0, 0.0],
    ];
    for fan in [GridFan::PolygonAnchor, GridFan::CenterFan] {
        let mut world = geometry(&points, [7.0, -3.0]);
        let source = world.surfaces[0].clone();
        let vertices = world.vertices.clone();
        let boundaries = world.boundaries.clone();
        let original_indices = world.indices.clone();
        let stats = subdivide_surface(
            &mut world,
            0,
            GridOptions {
                spacing: 128.0,
                fan,
                texture_offset: [7.0, -3.0],
                ..GridOptions::default()
            },
        )
        .unwrap();
        assert_eq!(stats.fragments, 9);
        assert_eq!(stats.splits, 8);
        assert_eq!(
            stats.appended_vertices,
            if fan == GridFan::CenterFan { 54 } else { 36 }
        );
        assert_eq!(
            stats.appended_indices,
            if fan == GridFan::CenterFan { 108 } else { 54 }
        );
        assert_eq!(world.surfaces[0].source_id, source.source_id);
        assert_eq!(world.surfaces[0].vertices, source.vertices);
        assert_eq!(world.surfaces[0].boundaries, source.boundaries);
        assert_eq!(
            world.surfaces[0].texture_projection,
            source.texture_projection
        );
        assert_eq!(world.boundaries, boundaries);
        assert_eq!(&world.vertices[..vertices.len()], vertices);
        assert_eq!(&world.indices[..original_indices.len()], original_indices);
        for point in &world.vertices[vertices.len()..] {
            assert_eq!(
                point.vertex.texcoord,
                [point.vertex.position.0[0], point.vertex.position.0[1]]
            );
            assert_eq!(point.normal, Vec3([0.0, 0.0, 1.0]));
            assert_eq!(point.vertex.normal, point.normal);
            assert_eq!(point.vertex.color, [255; 4]);
        }
        assert!(
            world.indices[world.surfaces[0].indices.indices()]
                .iter()
                .all(|&i| i as usize >= vertices.len())
        );
    }
}

#[test]
fn native_eight_unit_margin_and_exact_plane_vertices_are_retained() {
    for (left, right, fragments) in [
        (-7.0, 40.0, 1),
        (-8.0, 40.0, 2),
        (-40.0, 7.0, 1),
        (-40.0, 8.0, 2),
    ] {
        let points = [
            [left, -4.0, 0.0],
            [right, -4.0, 0.0],
            [right, 4.0, 0.0],
            [left, 4.0, 0.0],
        ];
        let mut world = geometry(&points, [0.0; 2]);
        assert_eq!(
            subdivide_surface(&mut world, 0, GridOptions::default())
                .unwrap()
                .fragments,
            fragments
        );
    }
    let points = [
        [-32.0, 0.0, 0.0],
        [0.0, -32.0, 0.0],
        [32.0, 0.0, 0.0],
        [0.0, 32.0, 0.0],
    ];
    let mut world = geometry(&points, [0.0; 2]);
    let stats = subdivide_surface(&mut world, 0, GridOptions::default()).unwrap();
    assert_eq!(stats.fragments, 4);
    assert_eq!(stats.appended_vertices, 12);
}

#[test]
fn generic_vertex_attributes_use_the_same_edge_fraction() {
    let points = [
        [-16.0, -4.0, 0.0],
        [16.0, -4.0, 0.0],
        [16.0, 4.0, 0.0],
        [-16.0, 4.0, 0.0],
    ];
    let mut world = geometry(&points, [0.0; 2]);
    for vertex in &mut world.vertices {
        let x = vertex.vertex.position.0[0];
        vertex.vertex.lightmap_coord = [x / 32.0, 1.0];
        vertex.vertex.color = [if x < 0.0 { 0 } else { 200 }, 17, 33, 255];
        vertex.normal = Vec3([x / 16.0, 0.0, 1.0]);
        vertex.vertex.normal = vertex.normal;
    }
    subdivide_surface(&mut world, 0, GridOptions::default()).unwrap();
    let intersections: Vec<_> = world.vertices[4..]
        .iter()
        .filter(|v| v.vertex.position.0[0] == 0.0)
        .collect();
    assert_eq!(intersections.len(), 4);
    for point in intersections {
        assert_eq!(point.vertex.lightmap_coord, [0.0, 1.0]);
        assert_eq!(point.vertex.color, [100, 17, 33, 255]);
        assert_eq!(point.normal, Vec3([0.0, 0.0, 1.0]));
        assert_eq!(point.vertex.normal, point.normal);
    }
}

#[test]
fn subdivision_caps_fail_without_mutating_source_geometry() {
    let points = [
        [-64.0, -64.0, 0.0],
        [192.0, -64.0, 0.0],
        [192.0, 192.0, 0.0],
        [-64.0, 192.0, 0.0],
    ];
    for options in [
        GridOptions {
            max_fragments: 8,
            ..GridOptions::default()
        },
        GridOptions {
            max_output_vertices: 35,
            ..GridOptions::default()
        },
        GridOptions {
            max_split_depth: 1,
            ..GridOptions::default()
        },
        GridOptions {
            spacing: f32::NAN,
            ..GridOptions::default()
        },
        GridOptions {
            max_boundary_vertices: 3,
            ..GridOptions::default()
        },
        GridOptions {
            texture_offset: [f32::INFINITY, 0.0],
            ..GridOptions::default()
        },
    ] {
        assert_failed_subdivision_unchanged(geometry(&points, [0.0; 2]), 0, options);
    }
    // Overflow detected after preparing all fragments must also stay local.
    let mut world = geometry(&points, [0.0; 2]);
    for vertex in &mut world.vertices {
        vertex.vertex.texcoord[0] = f32::MAX;
    }
    assert_failed_subdivision_unchanged(
        world,
        0,
        GridOptions {
            texture_offset: [-f32::MAX, 0.0],
            ..GridOptions::default()
        },
    );
    assert_failed_subdivision_unchanged(geometry(&points, [0.0; 2]), 1, GridOptions::default());
    let mut world = geometry(&points, [0.0; 2]);
    world.boundaries[0].first = u32::MAX;
    assert_failed_subdivision_unchanged(world, 0, GridOptions::default());
}

fn assert_failed_subdivision_unchanged(
    mut world: WorldGeometry,
    surface: usize,
    options: GridOptions,
) {
    let vertices = world.vertices.clone();
    let indices = world.indices.clone();
    let boundaries = world.boundaries.clone();
    let light_samples = world.light_samples.clone();
    let spans = |world: &WorldGeometry| {
        world
            .surfaces
            .iter()
            .map(|s| {
                (
                    s.source_id,
                    s.kind,
                    s.vertices,
                    s.indices,
                    s.boundaries,
                    s.light_samples,
                    s.bounds,
                    s.texture_projection,
                )
            })
            .collect::<Vec<_>>()
    };
    let originals = spans(&world);
    assert!(subdivide_surface(&mut world, surface, options).is_err());
    assert_eq!(world.vertices, vertices);
    assert_eq!(world.indices, indices);
    assert_eq!(world.boundaries, boundaries);
    assert_eq!(world.light_samples, light_samples);
    assert_eq!(spans(&world), originals);
}

#[test]
fn raised_boundary_limit_accepts_a_large_unsplit_polygon() {
    let points: Vec<_> = (0..80)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 80.0;
            // A polygon centred on the origin would still split on grid plane
            // zero, regardless of its spacing. Keep this raised-limit fixture
            // wholly inside one positive grid region.
            [
                256.0 + angle.cos() * 100.0,
                256.0 + angle.sin() * 100.0,
                0.0,
            ]
        })
        .collect();
    let mut world = geometry(&points, [0.0; 2]);
    assert_eq!(
        subdivide_surface(
            &mut world,
            0,
            GridOptions {
                spacing: 1024.0,
                ..GridOptions::default()
            }
        )
        .unwrap()
        .appended_vertices,
        80
    );
    assert_failed_subdivision_unchanged(
        geometry(&points, [0.0; 2]),
        0,
        GridOptions {
            max_boundary_vertices: 60,
            ..GridOptions::default()
        },
    );
}

fn original_function(text: &str, signature: &str) -> String {
    let start = text.find(signature).unwrap();
    let opening = start + text[start..].find('{').unwrap();
    let mut depth = 1;
    let mut end = opening + 1;
    for byte in text.as_bytes()[end..].iter() {
        if *byte == b'{' {
            depth += 1;
        }
        if *byte == b'}' {
            depth -= 1;
        }
        end += 1;
        if depth == 0 {
            break;
        }
    }
    text[start..end].to_owned()
}

// An optional developer-only original-C comparison. Normal tests above need no
// compiler or reference checkout. The parent owns when this probe is run.
#[test]
fn original_gl_warp_function_comparison() {
    use std::fmt::Write;
    use std::path::PathBuf;
    use std::process::Command;
    let Ok(qsrc) = std::env::var("QA_GRID_QSRC") else {
        return;
    };
    let directory = PathBuf::from(std::env::var("QA_GRID_EVIDENCE").unwrap());
    std::fs::create_dir_all(&directory).unwrap();
    let cases = [
        vec![
            [-64.0, -64.0, 0.0],
            [192.0, -64.0, 0.0],
            [192.0, 192.0, 0.0],
            [-64.0, 192.0, 0.0],
        ],
        vec![
            [-32.0, 0.0, 0.0],
            [0.0, -32.0, 0.0],
            [32.0, 0.0, 0.0],
            [0.0, 32.0, 0.0],
        ],
        vec![
            [-128.0, -4.0, 0.0],
            [128.0, -4.0, 0.0],
            [128.0, 4.0, 0.0],
            [-128.0, 4.0, 0.0],
        ],
        vec![
            [-256.0, -128.0, 0.0],
            [256.0, -128.0, 0.0],
            [192.0, 128.0, 0.0],
            [-128.0, 192.0, 0.0],
        ],
        vec![
            [-7.0, -4.0, 0.0],
            [40.0, -4.0, 0.0],
            [40.0, 4.0, 0.0],
            [-7.0, 4.0, 0.0],
        ],
        vec![
            [-8.0, -4.0, 0.0],
            [40.0, -4.0, 0.0],
            [40.0, 4.0, 0.0],
            [-8.0, 4.0, 0.0],
        ],
    ];
    for (name, relative, fan, spacing) in [
        (
            "q1",
            "quake/WinQuake/gl_warp.c",
            GridFan::PolygonAnchor,
            128.0,
        ),
        ("q2", "quake-2/ref_gl/gl_warp.c", GridFan::CenterFan, 64.0),
    ] {
        let original = std::fs::read_to_string(PathBuf::from(&qsrc).join(relative)).unwrap();
        let prelude = ORIGINAL_PRELUDE.replace("REFERENCE_SPACING", &format!("{spacing}"));
        let source = format!(
            "{prelude}\n{}\n{}\n{ORIGINAL_DRIVER}",
            original_function(&original, "void BoundPoly"),
            original_function(&original, "void SubdividePolygon")
        );
        let path = directory.join(format!("{name}-original.c"));
        std::fs::write(&path, source).unwrap();
        let executable = directory.join(format!("{name}-original"));
        let compile = Command::new("cc")
            .args([
                "-std=c99",
                "-O2",
                "-ffp-contract=off",
                "-fno-strict-aliasing",
            ])
            .arg(&path)
            .args(["-lm", "-o"])
            .arg(&executable)
            .output()
            .unwrap();
        std::fs::write(
            directory.join(format!("{name}-compile.log")),
            &compile.stderr,
        )
        .unwrap();
        assert!(compile.status.success());
        let mut input = String::new();
        let mut expected = String::new();
        for (case, points) in cases.iter().enumerate() {
            writeln!(input, "{}", points.len()).unwrap();
            for point in points {
                writeln!(input, "{} {} {}", point[0], point[1], point[2]).unwrap();
            }
            let mut world = geometry(points, [0.0; 2]);
            let first = world.vertices.len();
            subdivide_surface(
                &mut world,
                0,
                GridOptions {
                    spacing,
                    fan,
                    ..GridOptions::default()
                },
            )
            .unwrap();
            for (index, vertex) in world.vertices[first..].iter().enumerate() {
                write!(expected, "{case} {index}").unwrap();
                for value in vertex
                    .vertex
                    .position
                    .0
                    .into_iter()
                    .chain(vertex.vertex.texcoord)
                {
                    write!(expected, " {:08x}", value.to_bits()).unwrap();
                }
                expected.push('\n');
            }
        }
        let input_path = directory.join(format!("{name}-input.txt"));
        std::fs::write(&input_path, input).unwrap();
        let output = Command::new(&executable).arg(input_path).output().unwrap();
        assert!(output.status.success());
        let actual = String::from_utf8(output.stdout).unwrap();
        std::fs::write(directory.join(format!("{name}-original.txt")), &actual).unwrap();
        std::fs::write(directory.join(format!("{name}-rust.txt")), &expected).unwrap();
        // Exact numeric f32 values, permitting only signed-zero encoding differences.
        let rows: Vec<_> = actual.lines().collect();
        let expected_rows: Vec<_> = expected.lines().collect();
        assert_eq!(rows.len(), expected_rows.len());
        let mut different_bits = 0;
        for (left, right) in rows.iter().zip(expected_rows) {
            let a: Vec<_> = left.split_whitespace().collect();
            let b: Vec<_> = right.split_whitespace().collect();
            assert_eq!(&a[..2], &b[..2]);
            assert_eq!(a.len(), b.len());
            for (x, y) in a[2..].iter().zip(&b[2..]) {
                let x = u32::from_str_radix(x, 16).unwrap();
                let y = u32::from_str_radix(y, 16).unwrap();
                different_bits += usize::from(x != y);
                assert_eq!(f32::from_bits(x), f32::from_bits(y));
            }
        }
        let report = format!(
            "source={relative}\nfunctions=BoundPoly,SubdividePolygon\nfixtures={}\nvertices={}\ndifferent_float_bits={different_bits}\nresult=PASS\nscope=analytic positions/warp UV and fan representation; native linked list reversed to recursion leaf order; no retail image proof\n",
            cases.len(),
            rows.len()
        );
        std::fs::write(directory.join(format!("{name}-result.txt")), report).unwrap();
    }
}

const ORIGINAL_PRELUDE: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <stdint.h>
#include <stdarg.h>
#define VERTEXSIZE 7
#define SUBDIVIDE_SIZE REFERENCE_SPACING
#define ERR_DROP 1
typedef float vec3_t[3];
typedef struct poly_s { struct poly_s *next; int numverts; float verts[4][VERTEXSIZE]; } glpoly_t;
typedef struct { float vecs[2][4]; } texinfo_t;
typedef struct { glpoly_t *polys; texinfo_t *texinfo; } msurface_t;
static texinfo_t texture={{{1,0,0,0},{0,1,0,0}}};
static msurface_t surface={0,&texture};
static msurface_t *warpface=&surface;
static struct { float value; } gl_subdivide_size={REFERENCE_SPACING};
static void *Hunk_Alloc(size_t n) { return calloc(1,n); }
static void Sys_Error(const char *s, ...) { (void)s; exit(2); }
static void drop(int n,const char *s, ...) { (void)n; (void)s; exit(2); }
static struct { void (*Sys_Error)(int,const char *,...); } ri={drop};
#define VectorCopy(a,b) ((b)[0]=(a)[0],(b)[1]=(a)[1],(b)[2]=(a)[2])
#define VectorClear(a) ((a)[0]=(a)[1]=(a)[2]=0)
#define VectorAdd(a,b,c) ((c)[0]=(a)[0]+(b)[0],(c)[1]=(a)[1]+(b)[1],(c)[2]=(a)[2]+(b)[2])
#define VectorScale(a,b,c) ((c)[0]=(a)[0]*(b),(c)[1]=(a)[1]*(b),(c)[2]=(a)[2]*(b))
#define DotProduct(a,b) ((a)[0]*(b)[0]+(a)[1]*(b)[1]+(a)[2]*(b)[2])
"#;

const ORIGINAL_DRIVER: &str = r#"
static uint32_t bits(float v) { uint32_t b; memcpy(&b,&v,4); return b; }
int main(int argc, char **argv) {
    if (argc != 2 || !freopen(argv[1], "r", stdin)) return 2;
    int count,fixture=0;
    while(scanf("%d",&count)==1) {
        if(count<3 || count>60) return 2;
        float points[64][3]={{0}};
        for(int i=0;i<count;i++) if(scanf("%f%f%f",&points[i][0],&points[i][1],&points[i][2])!=3) return 2;
        surface.polys=NULL;
        SubdividePolygon(count,points[0]);
        glpoly_t *list[4096]; int total=0;
        for(glpoly_t *p=surface.polys;p;p=p->next) { if(total==4096) return 2; list[total++]=p; }
        int vertex=0;
        for(int i=total-1;i>=0;i--) {
            glpoly_t *p=list[i];
            for(int j=0;j<p->numverts;j++) {
                printf("%d %d",fixture,vertex++);
                for(int k=0;k<5;k++) printf(" %08x",bits(p->verts[j][k]));
                putchar('\n');
            }
            free(p);
        }
        fixture++;
    }
    return 0;
}
"#;
