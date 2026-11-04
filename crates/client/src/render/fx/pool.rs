//! A fixed pool of particles and the one mesh that draws it.
//!
//! The pool itself is the shared `fx` crate's (`crates/fx`, Bevy-free and
//! vendored into oneshot and Before Colony); this is its Bevy half: the mesh
//! it writes into, a camera from a transform, and `write`, which copies a
//! pool into the mesh's attributes in place.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

pub use ::fx::pool::{
    Cam, Orient, Particle, Pool, Quad, Verts, MIN_PX, STREAK_MAX_M, STREAK_MIN_M,
};

/// The camera a pool's quads face, from the eye's transform.
pub fn cam(gt: &GlobalTransform) -> Cam {
    Cam {
        pos: gt.translation().into(),
        right: Vec3::from(gt.right()).into(),
        up: Vec3::from(gt.up()).into(),
    }
}

/// Writing a pool into the Bevy mesh [`pool_mesh`] made.
pub trait MeshWrite {
    /// Copy the pool into `mesh`, in place, relative to `anchor` — or do
    /// nothing when it is empty and the mesh already is. Returns whether it
    /// wrote. No allocation: the attributes are the ones [`pool_mesh`] made.
    fn write(&mut self, mesh: &mut Mesh, cam: &Cam, anchor: Vec3) -> bool;
}

impl MeshWrite for Pool {
    fn write(&mut self, mesh: &mut Mesh, cam: &Cam, anchor: Vec3) -> bool {
        if !self.needs_write() {
            return false;
        }
        let (mut pos, mut uv, mut col) = (None, None, None);
        for (attr, values) in mesh.attributes_mut() {
            let id = attr.id;
            match values {
                VertexAttributeValues::Float32x3(v) if id == Mesh::ATTRIBUTE_POSITION.id => {
                    pos = Some(v);
                }
                VertexAttributeValues::Float32x2(v) if id == Mesh::ATTRIBUTE_UV_0.id => {
                    uv = Some(v);
                }
                VertexAttributeValues::Float32x4(v) if id == Mesh::ATTRIBUTE_COLOR.id => {
                    col = Some(v);
                }
                _ => {}
            }
        }
        let (Some(pos), Some(uv), Some(col)) = (pos, uv, col) else {
            return false;
        };
        self.fill(Verts { pos, uv, col }, cam, anchor.into())
    }
}

/// An empty mesh for a pool of `cap` quads, with its fixed indices. Normals
/// point up: the lit pool is shaded like the ground under it.
pub fn pool_mesh(cap: usize) -> Mesh {
    let n = cap * 4;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![::fx::pool::NORMAL; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; n]);
    mesh.insert_indices(Indices::U32(::fx::pool::indices(cap)));
    mesh
}
