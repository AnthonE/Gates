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

fn reconcile(
    mut commands: Commands,
    mut ring: ResMut<StructRing>,
    mirror: Res<Mirror>,
    models: Res<super::super::viewmodel::Models>,
    assets: Res<AssetServer>,
    meshes: Res<Assets<Mesh>>,
    mats: Res<Assets<StandardMaterial>>,
) {
    let ring = &mut *ring;
    ring.gen += 1;
    ring.pending_loot = sync_loot(
        &mut commands,
        &mut ring.gitems,
        ring.kit.as_ref().unwrap(),
        ring.gen,
        mirror.0.ground_items(),
        &mirror.0.catalog,
        &models,
        &assets,
        &meshes,
        &mats,
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
