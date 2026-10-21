use super::*;
use bevy::asset::AssetPlugin;
use client_core::core::ClientCore;

#[derive(Resource)]
struct Mirror(ClientCore);

fn init(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(StructRing {
        kit: Some(build_kit(&assets, &mut meshes, &mut mats)),
        ..default()
    });
}

#[allow(clippy::too_many_arguments)]
fn reconcile(
    mut commands: Commands,
    mut ring: ResMut<StructRing>,
    mirror: Res<Mirror>,
    models: Res<super::super::viewmodel::Models>,
    mut skin_mats: ResMut<super::super::viewmodel::SkinMats>,
    assets: Res<AssetServer>,
    meshes: Res<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let ring = &mut *ring;
    ring.gen += 1;
    ring.pending_loot = sync_loot(
        &mut commands,
        &mut ring.gitems,
        ring.kit.as_ref().unwrap(),
        ring.gen,
        mirror.0.ground_items(),
        mirror.0.clock.server_est,
        (&mirror.0.catalog, &mirror.0.skins),
        &models,
        &mut skin_mats,
        &assets,
        &meshes,
        &mut mats,
        |_, _| 0.0,
    );
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>();
    app.init_resource::<super::super::feed::Feed>();
    app.insert_resource(Mirror(ClientCore::new(20260731, 1, 0)));
    app.add_systems(Startup, (init, super::super::viewmodel::load_models));
    app.add_systems(Update, reconcile.run_if(structures_changed));
    app.update(); // The kit is warm before the first loot-only update.
    app
}

fn sync(app: &mut App, items: &[protocol::WireGItem]) {
    let mut buf = [0; 1024];
    let n = protocol::event::encode_event_gitem_sync(true, items, &mut buf).unwrap();
    let (first, second) = {
        let mut mirror = app.world_mut().resource_mut::<Mirror>();
        let first = mirror.0.on_stream(&buf[..n]).unwrap();
        (first, mirror.0.applied2())
    };
    assert_eq!(first, 0, "the repro must have no unrelated world update");
    let mut feed = app.world_mut().resource_mut::<super::super::feed::Feed>();
    feed.applied = first;
    feed.applied2 = second;
    app.update();
}

fn item(id: u32) -> protocol::WireGItem {
    protocol::WireGItem {
        id,
        qx: 1000,
        qy: 0,
        qz: 1000,
        item: 16,
        count: 1,
        dir: [0; 3],
        cond: 0,
        skin: 0,
        toss: None,
    }
}

#[test]
fn loot_only_wire_updates_spawn_pick_up_and_clear_the_last_model() {
    let mut app = app();
    sync(&mut app, &[item(1), item(2)]);
    let ring = app.world().resource::<StructRing>();
    assert_eq!(ring.gitems.len(), 2);
    let removed = ring.gitems[&1].entity;
    let kept = ring.gitems[&2].entity;
    assert!(app.world().get::<Mesh3d>(kept).is_some());
    sync(&mut app, &[item(2)]);
    assert!(app.world().get_entity(removed).is_err());
    assert_eq!(app.world().resource::<StructRing>().gitems[&2].entity, kept);
    sync(&mut app, &[]);
    assert!(app.world().get_entity(kept).is_err());
    assert!(app.world().resource::<StructRing>().gitems.is_empty());
}

#[test]
fn a_late_catalog_replaces_the_pouch_with_the_shared_held_model() {
    let mut app = app();
    sync(&mut app, &[item(1)]);
    let old = app.world().resource::<StructRing>().gitems[&1].entity;
    app.world_mut()
        .resource_mut::<Mirror>()
        .0
        .catalog
        .set(
            16,
            b"Torch",
            protocol::ItemRow {
                cond_max: 0,
                armor_pct: 0,
                wear_slot: 0,
                stack_max: 1,
                ..protocol::ItemRow::EMPTY
            },
        )
        .unwrap();
    let mut feed = app.world_mut().resource_mut::<super::super::feed::Feed>();
    feed.applied = client_core::core::APPLIED_CATALOG;
    feed.applied2 = 0;
    app.update();
    let ring = app.world().resource::<StructRing>();
    let model = ring.gitems[&1].model.expect("torch has a generated model");
    let entity = ring.gitems[&1].entity;
    let (mesh, _) = app
        .world()
        .resource::<super::super::viewmodel::Models>()
        .row(model);
    assert_eq!(app.world().get::<Mesh3d>(entity).unwrap().0, mesh);
    assert!(app.world().get_entity(old).is_err());
}

#[test]
fn a_loading_model_keeps_the_pouch_then_replaces_it_without_wire_news() {
    let mut app = app();
    let i = crate::ui::hold::HELD_MODELS
        .iter()
        .position(|m| m.key == "torch")
        .unwrap();
    let (handle, _) = app
        .world()
        .resource::<super::super::viewmodel::Models>()
        .row(i);
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .remove(handle.id())
        .unwrap();
    app.world_mut()
        .resource_mut::<Mirror>()
        .0
        .catalog
        .set(
            16,
            b"Torch",
            protocol::ItemRow {
                cond_max: 0,
                armor_pct: 0,
                wear_slot: 0,
                stack_max: 1,
                ..protocol::ItemRow::EMPTY
            },
        )
        .unwrap();
    sync(&mut app, &[item(1)]);
    let ring = app.world().resource::<StructRing>();
    assert!(ring.pending_loot);
    assert_eq!(ring.gitems[&1].model, None);
    let pouch = ring.gitems[&1].entity;
    let mut feed = app.world_mut().resource_mut::<super::super::feed::Feed>();
    feed.applied = 0;
    feed.applied2 = 0;
    app.update();
    assert_eq!(
        app.world().resource::<StructRing>().gitems[&1].entity,
        pouch
    );
    app.world_mut()
        .resource_mut::<Assets<Mesh>>()
        .insert(handle.id(), mesh)
        .unwrap();
    app.update();
    let ring = app.world().resource::<StructRing>();
    assert_eq!(ring.gitems[&1].model, Some(i));
    assert!(!ring.pending_loot);
    assert!(app.world().get_entity(pouch).is_err());
    let generation = ring.gen;
    app.update();
    assert_eq!(
        app.world().resource::<StructRing>().gen,
        generation,
        "idle frames must not reconcile again"
    );
}

/// A landed arrow is drawn as a shaft standing in the ground where it
/// fell, not as the pouch every other unmodelled item gets.
#[test]
fn a_landed_arrow_stands_in_the_ground() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<Mirror>()
        .0
        .catalog
        .set(
            16,
            b"Wooden Arrow",
            protocol::ItemRow {
                stack_max: 64,
                ..protocol::ItemRow::EMPTY
            },
        )
        .unwrap();
    sync(&mut app, &[item(1), item(2)]);
    let ring = app.world().resource::<StructRing>();
    let kit = ring.kit.as_ref().unwrap();
    for id in [1, 2] {
        let e = ring.gitems[&id].entity;
        assert_eq!(
            app.world().get::<Mesh3d>(e).unwrap().0,
            kit.arrow_mesh,
            "stack {id} drew the pouch"
        );
        let t = app.world().get::<Transform>(e).unwrap();
        let axis = t.rotation * Vec3::Y;
        let lean = axis.y.clamp(-1.0, 1.0).acos();
        assert!(
            (ARROW_LEAN[0] - 1e-3..=ARROW_LEAN[1] + 1e-3).contains(&lean),
            "stack {id} leans {lean} rad"
        );
        let head = t.translation - axis * ARROW_LEN_M * 0.5;
        let surface = item(id).qy as f32 * POS_Y_Q;
        assert!(
            head.y < surface - 0.05,
            "stack {id}'s head is not in the ground ({} vs {surface})",
            head.y
        );
    }
    let a = app
        .world()
        .get::<Transform>(ring.gitems[&1].entity)
        .unwrap()
        .rotation;
    let b = app
        .world()
        .get::<Transform>(ring.gitems[&2].entity)
        .unwrap()
        .rotation;
    assert_ne!(a, b, "two arrows do not stand in ranks");
}

/// A dropped skinned stack keeps its look (v102): drawn plain while the
/// skin catalog does not know the skin, and redrawn in its tint when a
/// catalog batch lands (`APPLIED2_SKINS`) — the shared model's material,
/// multiplied, never the base material itself.
#[test]
fn a_dropped_skinned_stack_is_drawn_in_its_skin_once_the_catalog_knows_it() {
    const SKIN: u16 = 0x0A61;
    let mut app = app();
    app.world_mut()
        .resource_mut::<Mirror>()
        .0
        .catalog
        .set(
            16,
            b"Torch",
            protocol::ItemRow {
                stack_max: 1,
                ..protocol::ItemRow::EMPTY
            },
        )
        .unwrap();
    sync(
        &mut app,
        &[protocol::WireGItem {
            skin: SKIN,
            ..item(1)
        }],
    );
    let base = {
        let ring = app.world().resource::<StructRing>();
        let model = ring.gitems[&1].model.expect("torch has a generated model");
        assert_eq!(ring.gitems[&1].skin, 0, "an unknown skin is drawn plain");
        let base = app
            .world()
            .resource::<super::super::viewmodel::Models>()
            .row(model)
            .1;
        let shown = &app
            .world()
            .get::<MeshMaterial3d<StandardMaterial>>(ring.gitems[&1].entity)
            .unwrap()
            .0;
        assert_eq!(*shown, base);
        base
    };

    {
        let mut mirror = app.world_mut().resource_mut::<Mirror>();
        mirror.0.skins.count = 1;
        mirror
            .0
            .skins
            .set(
                0,
                b"Ember",
                protocol::SkinRow {
                    catalog: SKIN,
                    covers: 16,
                    tint: [255, 64, 0],
                    coin: protocol::COIN_NONE,
                    price: 0,
                    texture: [0; 8],
                    unlock: false,
                },
            )
            .unwrap();
    }
    // What `paint::paint` learns of a tint row each frame (v110): a skin is
    // drawn once its look is known.
    app.world_mut()
        .resource_mut::<super::super::viewmodel::SkinMats>()
        .learn(SKIN, super::super::viewmodel::Look::Tint);
    let mut feed = app.world_mut().resource_mut::<super::super::feed::Feed>();
    feed.applied = 0;
    feed.applied2 = client_core::core::APPLIED2_SKINS;
    app.update();
    let ring = app.world().resource::<StructRing>();
    assert_eq!(ring.gitems[&1].skin, SKIN, "the catalog landing redrew it");
    let shown = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(ring.gitems[&1].entity)
        .unwrap()
        .0
        .clone();
    assert_ne!(shown, base, "drawn in a tinted copy");
    let mats = app.world().resource::<Assets<StandardMaterial>>();
    let (plain, tinted) = (
        mats.get(&base).unwrap().base_color.to_linear(),
        mats.get(&shown).unwrap().base_color.to_linear(),
    );
    // Red × 1, blue × 0: the base colour multiplied, alpha untouched.
    assert_eq!(
        (tinted.red, tinted.blue, tinted.alpha),
        (plain.red, 0.0, plain.alpha),
        "{plain:?} → {tinted:?}"
    );
}

/// A stack first seen flying out of a hand (v114) is drawn on its way: out
/// of the hand, above where it rests mid-flight, and at the end lying
/// exactly as it would have without the throw. Before it leaves the hand on
/// this clock it is hidden.
#[test]
fn a_thrown_stack_flies_out_of_the_hand_and_lies_where_it_rests() {
    use super::super::loot::{toss_born, Pose, Tossed, SETTLE_TICKS};
    use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
    let mut app = app();
    app.add_systems(Update, super::super::loot::fly.after(reconcile));
    let born = 1000.0;
    let clock = |app: &mut App, at: f64| {
        app.world_mut()
            .resource_mut::<super::super::feed::Feed>()
            .server_tick_est = at;
        app.world_mut().resource_mut::<Mirror>().0.clock.server_est = at;
    };
    // A metre back in x and 1.2 m up: the hand that let go.
    let from = [(-1.0 / POS_XZ_Q) as i16, (1.2 / POS_Y_Q) as i16, 0];
    let thrown = protocol::WireGItem {
        toss: Some(protocol::WireToss {
            born_lo: born as u64 as u8,
            from,
        }),
        ..item(1)
    };
    clock(&mut app, born - 2.0);
    sync(&mut app, &[thrown]);
    let e = app.world().resource::<StructRing>().gitems[&1].entity;
    let toss = *app.world().get::<Tossed>(e).expect("a thrown stack flies");
    assert_eq!(
        app.world().get::<Visibility>(e),
        Some(&Visibility::Hidden),
        "out of the hand before it was thrown"
    );
    // No jump anywhere along it — out of the hand, at the touchdown, the
    // hop, the roll, and into the pose it rests in — sampled finer than
    // any frame rate draws it.
    let end = born + 60.0 + SETTLE_TICKS as f64;
    let mut last: Option<Transform> = None;
    for k in 0..=((end - born) * 4.0) as u32 {
        let t = born + k as f64 * 0.25;
        let at = match toss.pose(t) {
            Pose::Flying(p) => p,
            Pose::Rested => toss.rest(),
            Pose::Waiting => panic!("hidden after it was thrown, at {t}"),
        };
        if let Some(prev) = last {
            let moved = prev.translation.distance(at.translation);
            let turned = prev.rotation.angle_between(at.rotation);
            assert!(
                moved < 0.1 && turned < 0.2,
                "a jump {} ticks in: {moved} m, {turned} rad",
                t - born
            );
        }
        last = Some(at);
    }

    clock(&mut app, born);
    app.update();
    let tf = *app.world().get::<Transform>(e).unwrap();
    let rest = toss.rest();
    let hand = Vec3::new(1000.0 * POS_XZ_Q - 1.0, 1.2, 1000.0 * POS_XZ_Q);
    assert_eq!(
        app.world().get::<Visibility>(e),
        Some(&Visibility::Inherited)
    );
    assert!(
        tf.translation.distance(hand) < 0.3,
        "left the hand at {} not {hand}",
        tf.translation
    );

    clock(&mut app, born + 8.0);
    app.update();
    let mid = *app.world().get::<Transform>(e).unwrap();
    assert!(
        mid.translation.y > rest.translation.y + 0.3,
        "not in the air"
    );
    assert!(mid.translation.x > hand.x && mid.translation.x < rest.translation.x + 0.1);

    clock(&mut app, born + 60.0 + SETTLE_TICKS as f64);
    app.update();
    assert!(app.world().get::<Tossed>(e).is_none(), "still flying");
    assert_eq!(*app.world().get::<Transform>(e).unwrap(), rest);

    // A redraw of a stack that already landed does not throw it again.
    assert_eq!(app.world().resource::<StructRing>().gitems[&1].entity, e);

    // The tick's low byte comes back beside the clock, either side of it.
    assert_eq!(toss_born(0x10, 0x210 as f64 + 3.4), 0x210 as f64);
    assert_eq!(toss_born(0xFF, 258.0), 255.0);
    assert_eq!(toss_born(5, 250.0), 261.0);
}

/// #416: Bevy evaluates run conditions on whichever thread ticks the
/// executor, so a condition holding a non-send resource reads it off the main
/// thread and aborts. `structures_changed` took `NonSend<Net>` once.
#[test]
fn structures_changed_is_send() {
    let mut app = App::new();
    let cond = IntoSystem::into_system(structures_changed);
    let mut cond: Box<dyn System<In = (), Out = bool>> = Box::new(cond);
    cond.initialize(app.world_mut());
    assert!(
        cond.is_send(),
        "a run condition must not take a non-send resource"
    );
}
