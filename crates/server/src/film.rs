//! Footage staging: a dev shard that films the auto turret at work
//! (`ci/film.sh record --turret`). Never on a public shard: it is read from
//! `GATES_FILM_TURRET` at boot and does nothing unset.
//!
//! [`stage_turret`] builds what a player would: two twig foundations, an
//! auto turret on one with a gun in its weapon slot and rounds in its ammo
//! slots, a small generator on the other with low grade in it, lit, and a
//! wire from the generator to the turret. Every step is the player's own
//! command, sent by a body that is then walked off, so the turret is a real
//! one: it spares its owner and shoots everyone else.

use sim_core::build::{self, BUILD_CELL_M, LOC_PLANE, MAT_TWIG, SHAPE_FOUNDATION};
use sim_core::deploy::{box_key, ARCH_GENERATOR, ARCH_TURRET};
use sim_core::footprint::Pose;
use sim_core::gather::ItemStack;
use sim_core::movement::Body;
use sim_core::power::WireRec;
use sim_core::world::{Command, World, EV_BUILD_REFUSED, EV_DEPLOY_REFUSED};

/// The body that builds it: an id no key derives to in practice.
pub const OWNER: u32 = 0x7F11_0001;
/// Where the owner goes once it is built, metres off along x.
const WALK_OFF_M: f32 = 400.0;
/// How far out, in build cells, [`stage_turret`] looks for its ground.
const SEARCH_CELLS: i32 = 12;

/// `"x,z[,gun]"`: where, and which of the turret's guns (`[turret] guns`,
/// default the assault rifle's row). `"@dx,dz"` is metres off the dev
/// spawn, wherever `dev_spawn = "biome:…"` found it.
pub fn stage_turret_spec(w: &mut World, spec: &str) -> Result<String, String> {
    let (rel, spec) = match spec.strip_prefix('@') {
        Some(s) => (w.dev_spawn.ok_or("GATES_FILM_TURRET @: no dev_spawn")?, s),
        None => ((0.0, 0.0), spec),
    };
    let mut it = spec.split(',').map(str::trim);
    let num = |s: Option<&str>| -> Result<f32, String> {
        s.ok_or("GATES_FILM_TURRET wants x,z[,gun]")?
            .parse::<f32>()
            .map_err(|e| format!("GATES_FILM_TURRET: {e}"))
    };
    let (x, z) = (rel.0 + num(it.next())?, rel.1 + num(it.next())?);
    let gun = match it.next() {
        Some(g) => g
            .parse::<usize>()
            .map_err(|e| format!("GATES_FILM_TURRET gun: {e}"))?,
        None => 6,
    };
    stage_turret(w, x, z, gun).map(|s| s.said)
}

/// What [`stage_turret`] stood up: a line for the boot log, and where the
/// turret is.
pub struct Staged {
    pub said: String,
    pub at: (f32, f32),
}

fn row_of_arch(w: &World, arch: u8) -> Result<u16, String> {
    (0..w.deploy.def_count)
        .find(|&r| w.deploy.defs[r as usize].arch == arch)
        .ok_or_else(|| format!("no deploy row of archetype {arch}"))
}

/// The owner's last refusal of this kind this tick, as its event reads.
fn refusal(w: &World, code: u8) -> String {
    w.events
        .entries()
        .iter()
        .filter(|e| e.code == code && e.a == OWNER)
        .map(|e| format!("refused: b {}, c {}", e.b, e.c))
        .last()
        .unwrap_or_else(|| "no refusal heard".into())
}

fn stack(item: u16, count: u16) -> ItemStack {
    ItemStack {
        item,
        count,
        ..ItemStack::EMPTY
    }
}

/// Stand the powered turret at the build cell holding `(x, z)`, its
/// generator one cell to the west. Returns what it built, or why not.
pub fn stage_turret(w: &mut World, x: f32, z: f32, gun: usize) -> Result<Staged, String> {
    if gun >= w.turret_def.n as usize {
        return Err(format!(
            "the turret takes {} guns, not row {gun}",
            w.turret_def.n
        ));
    }
    let tg = w.turret_def.guns[gun];
    let foundation =
        build::row_of(&w.build, SHAPE_FOUNDATION, MAT_TWIG).ok_or("no twig foundation row")?;
    let turret_row = row_of_arch(w, ARCH_TURRET)?;
    let gen_row = row_of_arch(w, ARCH_GENERATOR)?;
    let (cx0, cz0) = (build::build_cell_of(x), build::build_cell_of(z));

    w.tick(&[Command::Join { id: OWNER }]);
    let o = w
        .players
        .iter()
        .position(|p| p.id == OWNER)
        .ok_or("the owner did not join")?;
    let def = w.build.pieces[foundation as usize];
    let stand = |c: u16| (c as f32 + 0.5) * BUILD_CELL_M;
    // The nearest pair of cells, outward from `(x, z)`, where both
    // foundations go down: a landmark's ground, a rock or a slope refuses one.
    let mut found = None;
    let mut last = String::new();
    'search: for r in 0..=SEARCH_CELLS {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let (cx, cz) = ((cx0 + dx) as u16, (cz0 + dz) as u16);
                let pair = [(cx, cz), (cx - 1, cz)];
                // A cell south of the pair: in reach of both, standing in neither.
                w.players[o].body =
                    Body::at(w.seed, &w.haven, stand(cx) - 1.5, stand(cz) - BUILD_CELL_M);
                for (slot, &(item, count)) in def.costs[..def.n_costs as usize].iter().enumerate() {
                    w.players[o].inv[slot] = stack(item, count.saturating_mul(4));
                }
                let before = w.pieces.len();
                for &(fx, fz) in &pair {
                    w.tick(&[Command::Place {
                        id: OWNER,
                        row: foundation,
                        cx: fx,
                        cz: fz,
                        level: 0,
                        loc: LOC_PLANE,
                        freehand: false,
                        plate: 0,
                    }]);
                    if w.pieces.len() == before {
                        last = refusal(w, EV_BUILD_REFUSED);
                        break;
                    }
                }
                if w.pieces.len() == before + 2 {
                    found = Some((cx, cz));
                    break 'search;
                }
            }
        }
    }
    let (cx, cz) = found.ok_or_else(|| {
        format!("no two cells within {SEARCH_CELLS} of ({cx0}, {cz0}) take a foundation ({last})")
    })?;
    let cells = [(cx, cz), (cx - 1, cz)];

    let place = |w: &mut World, row: u16, (fx, fz): (u16, u16)| -> Result<u32, String> {
        let item = w.deploy.defs[row as usize].item;
        w.players[o].inv[6] = stack(item, 1);
        w.players[o].frame.sel = 6;
        let before = w.deploys.len();
        w.tick(&[Command::PlaceDeploy {
            id: OWNER,
            row,
            cx: fx,
            cz: fz,
            level: 0,
            loc: LOC_PLANE,
            pose: Pose::CENTRE,
        }]);
        if w.deploys.len() == before {
            return Err(format!(
                "deploy row {row} did not place at ({fx}, {fz}) ({})",
                refusal(w, EV_DEPLOY_REFUSED)
            ));
        }
        Ok(box_key(fx, fz, 0, LOC_PLANE))
    };
    let turret = place(w, turret_row, cells[0])?;
    let generator = place(w, gen_row, cells[1])?;

    let tb = w.deploys.box_index(turret).ok_or("the turret has no box")?;
    w.deploys.set_box_slot(tb, 0, stack(tg.item, 1));
    let per = w.gather.stack_max[tg.ammo as usize].max(1);
    for slot in 1..=6 {
        w.deploys.set_box_slot(tb, slot, stack(tg.ammo, per));
    }
    let gb = w
        .deploys
        .box_index(generator)
        .ok_or("the generator has no box")?;
    let fuel = w.cook.gen_fuel_item;
    w.deploys
        .set_box_slot(gb, 0, stack(fuel, w.gather.stack_max[fuel as usize].max(1)));
    w.tick(&[Command::Use {
        id: OWNER,
        cx: cells[1].0,
        cz: cells[1].1,
        level: 0,
        loc: LOC_PLANE,
    }]);

    w.players[o].inv[6] = stack(w.power_def.wire_tool, 1);
    w.players[o].frame.sel = 6;
    w.tick(&[Command::Wire {
        id: OWNER,
        wire: WireRec {
            from: generator,
            out: 0,
            to: turret,
            inp: 0,
        },
    }]);
    if w.deploys.power().wires().is_empty() {
        return Err("the wire was not run".into());
    }

    let (ox, oz) = (stand(cx) + WALK_OFF_M, stand(cz));
    w.players[o].body = Body::at(w.seed, &w.haven, ox, oz);
    let (tx, tz) = (stand(cx), stand(cz));
    Ok(Staged {
        said: format!(
            "turret at {tx:.0},{tz:.0} with gun row {gun}, generator lit and wired; owner walked to {ox:.0},{oz:.0}"
        ),
        at: (tx, tz),
    })
}
