use qa_render::{Command, FrontEnd, Limits, MaterialId, Refdef, SceneEntity, Vertex};

// qsrc Q3 tr_scene.c ClearScene/RenderScene preserve earlier scene payloads.
#[test]
fn scenes_copy_payloads_and_advance_ranges() {
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let first = SceneEntity {
        frame: 7,
        ..SceneEntity::default()
    };
    assert!(frame.add_entity(first));
    let mut vertices = [Vertex::default(); 3];
    assert!(frame.add_poly(MaterialId(0), &vertices));
    let mut areas = [0x55u8, 0xaa];
    assert!(frame.render_scene(Refdef::default(), &areas));
    vertices[0].color = [0; 4];
    assert_eq!(vertices[0].color, [0; 4]);
    areas.fill(0);
    frame.clear_scene();
    assert!(frame.add_entity(SceneEntity {
        frame: 11,
        ..SceneEntity::default()
    }));
    assert!(frame.render_scene(Refdef::default(), &[]));
    let packet = frame.finish();
    let Command::View(view0) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(view1) = packet.commands()[2] else {
        panic!()
    };
    assert_eq!(packet.entities(view0.scene.entities)[0].frame, 7);
    assert_eq!(packet.entities(view1.scene.entities)[0].frame, 11);
    assert_eq!(packet.hidden_areas(view0.hidden_areas), &[0x55, 0xaa]);
    assert_eq!(
        packet.vertices(packet.polys(view0.scene.polys)[0].vertices)[0].color,
        [255; 4]
    );
    assert_eq!(view1.scene.polys.count, 0);
    assert!(front.recycle(packet).is_ok());
}

#[test]
fn two_outstanding_packets_prevent_reuse_until_returned() {
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let first = front.begin_frame([1; 4]).unwrap().finish();
    let second = front.begin_frame([2; 4]).unwrap().finish();
    assert!(front.begin_frame([3; 4]).is_none());
    assert_eq!(first.frame, 1);
    assert_eq!(second.frame, 2);
    assert!(front.recycle(first).is_ok());
    let third = front.begin_frame([3; 4]).unwrap().finish();
    assert_eq!(third.frame, 3);
    let Command::Clear(color) = second.commands()[0] else {
        panic!()
    };
    assert_eq!(color, [2; 4]);
    assert!(front.recycle(second).is_ok());
    assert!(front.recycle(third).is_ok());
}

#[test]
fn rejected_poly_and_view_leave_existing_payloads_intact() {
    let limits = Limits {
        commands: 3,
        entities: 1,
        polys: 1,
        vertices: 3,
        lights: 1,
        area_bytes: 2,
    };
    let mut front = FrontEnd::load(limits).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(!frame.add_poly(MaterialId(0), &[Vertex::default(); 4]));
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(!frame.render_scene(Refdef::default(), &[0; 3]));
    assert!(frame.render_scene(Refdef::default(), &[0; 2]));
    let packet = frame.finish();
    assert_eq!(packet.rejected, 2);
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(view.scene.polys.count, 1);
    assert_eq!(packet.polys(view.scene.polys)[0].vertices.count, 3);
}
