//! Shared particle simulation, module evaluation and world-space presentation.
//! Domain adapters own selection, asset loading, instance anchors and teardown.
use bevy::prelude::*;
mod motion;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod motion_samples;
use moly_law::particle::schema::SimulationSpace;
use moly_law::particle::shape::{circle_base, cone_base, cone_volume, donut_position, hemisphere_position, single_sided_edge, sphere_position};
use moly_law::particle::step::set_remaining;
use moly_law::particle::{accumulate_rate, advance_lifetime, apply_gravity, burst_check,
    euler_rotate_deg, integrate, ring_push, BurstOutcome, DragSize,
    EmissionState, EmitterParams, LifetimeVerdict, LimitVelocity, Particle,
    RingPushVerdict, RotationOverLifetime, StepVerdict};
use crate::billboard::{Alignment, Quad, SizeClamp};
const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];
pub(crate) const PREWARM_STEP: f32 = 1.0 / 60.0;
/// 三类锚（effect 档案的 `kind`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectKind {
    Sky,
    Camera,
    Site,
}

#[derive(Clone)]
pub(crate) enum Geometry {
    Billboard { alignment: Alignment, clamp: SizeClamp, pivot: [f32; 3] },
    Mesh(crate::particle_geometry::MeshDraw),
}

/// 一条在跑的粒子系统。
pub(crate) struct Runtime {
    pub(crate) node: String,
    pub(crate) effect: String,
    pub(crate) emitter: EmitterParams,
    pub(crate) kind: EffectKind,
    pub(crate) camera_rotation: bool,
    pub(crate) node_affine: GlobalTransform,
    pub(crate) mesh: Handle<Mesh>,
    pub(crate) anchor: Option<Entity>,
    pub(crate) geometry: Geometry,
    pub(crate) ring_cursor: usize,
    pub(crate) pool: Vec<Particle>,
    pub(crate) side: Vec<Side>,
    pub(crate) emission: EmissionState,
    /// 播头（秒）：率曲线与 burst 的时间轴。
    pub(crate) playback_head: f32,
    /// 上一帧的播头（burst 裁决要一个左端点）。
    pub(crate) previous_head: f32,
    /// The initial zero-time burst belongs to the first positive simulation step.
    pub(crate) emission_started: bool,
    pub(crate) rng: Rng,
    /// 惰性 prewarm 的闸：首个推进帧快进一个周期。
    pub(crate) prewarmed: bool,
    pub(crate) cone_angle: Option<f32>,
    pub(crate) rol: Option<RotationOverLifetime>,
    pub(crate) limit: Option<LimitVelocity>,
    pub(crate) velocity_law: Option<moly_law::particle::velocity::VelocityOverLifetime>,
    pub(crate) size_law: Option<moly_law::particle::size::SizeOverLifetime>,
    pub(crate) color_law: Option<moly_law::particle::color::ColorOverLifetime>,
    pub(crate) born_total: u64,
    pub(crate) died_total: u64,
    pub(crate) full_total: u64,
    pub(crate) refused_total: u64,
}

/// 逐粒子的出生抽定值，与律池同下标平行存。
#[derive(Clone, Copy)]
pub(crate) struct Side {
    /// 出生种子因子（归一 [0,1)）：生命期内稳定求值读它。
    pub(crate) rand: f32,
    /// 自旋/限速族的种子杂凑吃全宽 u32。
    pub(crate) seed: u32,
    /// 自旋状态（弧度；公告板只画 Z 分量）。
    pub(crate) rot: [f32; 3],
    /// Authored birth size on all three axes. Renderer scale is applied in
    /// the geometry transform, not prematurely collapsed into one X factor.
    pub(crate) size: [f32; 3],
    pub(crate) gravity: f32,
    pub(crate) colour: [f32; 4],
    /// Persistent plus animated velocity, before the integration speed modifier.
    pub(crate) total_velocity: [f32; 3],
}

/// 出生抽签的确定性随机：splitmix64（站点链同款流算法、不同种子）。
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / 16_777_216.0
    }

    /// 全宽 u32（取混合输出的低 32 位，与 next_f32 的高 24 位不同位）。
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        z as u32
    }
}

/// 推进帧的锚点快照（天空/相机）。
pub(crate) struct Context {
    pub(crate) sky: GlobalTransform,
    pub(crate) camera: GlobalTransform,
    pub(crate) site: GlobalTransform,
}

// ---- 链头：锚点监视 ----

/// 锚 ∘ 节点链（按 effect 类别选锚）。
pub(crate) fn compose_to_world(system: &Runtime, ctx: &Context) -> GlobalTransform {
    let anchor = match system.kind {
        EffectKind::Sky => ctx.sky,
        EffectKind::Camera => {
            if system.camera_rotation {
                ctx.camera
            } else {
                GlobalTransform::from_translation(ctx.camera.translation())
            }
        }
        EffectKind::Site => ctx.site,
    };
    anchor * system.node_affine
}

/// 把律状态换算成公告板批次。
pub(crate) fn build_quads(system: &Runtime, to_world: &GlobalTransform) -> Vec<Quad> {
    let custom_data = system.emitter.custom_data.as_ref();
    let mut quads = Vec::with_capacity(system.pool.len());
    for (index, particle) in system.pool.iter().enumerate() {
        let side = system.side[index];
        // 归一化年龄：律的倒计时折算。
        let age = particle.normalized_age();
        let scale = system.node_affine.to_scale_rotation_translation().0.x;
        let evaluated_size = motion::size_at_age(system, &side, age);
        let size = [evaluated_size[0] * scale, evaluated_size[1] * scale];
        // The source render path multiplies byte colour into immutable birth
        // colour. Keep that operation separate from raw gradient evaluation.
        let colour = system.color_law.as_ref().map_or(side.colour, |law| {
            let birth = moly_law::particle::gradient::quantize_rgba8(side.colour);
            moly_law::particle::gradient::rgba8_to_float(
                law.apply(birth, side.seed, age * 100.0))
        });
        // 逐粒子自定义流：两个槽各按归一化年龄求值，与 size/colour 同口径
        // （`evaluate(age, side.rand)`）。componentCount 之外的分量留零。
        let mut custom1 = [0.0f32; 4];
        let mut custom2 = [0.0f32; 4];
        if let Some(cd) = custom_data {
            for (slot, out) in [(cd.custom1.as_ref(), &mut custom1), (cd.custom2.as_ref(), &mut custom2)] {
                if let Some(slot) = slot {
                    for (channel, curve) in slot.components.iter().enumerate() {
                        out[channel] = curve.evaluate(age, side.rand);
                    }
                }
            }
        }
        quads.push(Quad {
            centre: to_world.transform_point(Vec3::from_array(particle.position)),
            size: Vec2::from_array(size),
            // 自旋状态（弧度）：出生角 + rotationOverLifetime 的积分。
            rotation: side.rot[2],
            colour,
            custom1,
            custom2,
        });
    }
    quads
}

/// 一个仿真步：发射（率 + burst）→ 模块批 → 推进 → 死亡移除。
pub(crate) fn simulate(system: &mut Runtime, dt: f32, ctx: &Context) {
    simulate_with_emission(system, dt, ctx, true);
}

/// Source ParticleSystem.Stop() uses StopEmitting, not StopEmittingAndClear.
/// Existing particles still execute their module/lifetime batch. The host owns
/// the independent destruction deadline; no prewarm or burst runs after Stop.
pub(crate) fn simulate_stopped(system: &mut Runtime, dt: f32, ctx: &Context) {
    simulate_with_emission(system, dt, ctx, false);
}

fn simulate_with_emission(system: &mut Runtime, dt: f32, ctx: &Context, emitting: bool) {
    if emitting && dt > 0.0 {
        let emission = system
            .emitter
            .emission
            .as_ref()
            .expect("判读已门 emission 在场")
            .clone();
        system.previous_head = if system.emission_started {
            system.playback_head
        } else {
            // burst_check uses (previous, now]. Include exactly zero on first
            // playback without shifting the source clock or repeating it later.
            system.emission_started = true;
            -f32::from_bits(1)
        };
        let rate = emission
            .rate_over_time
            .evaluate(system.playback_head, 0.5);
        let mut emitted = accumulate_rate(&mut system.emission, rate, dt);
        system.playback_head += dt;
        // 非循环系统播到头就停发（现存粒子活完寿命）。循环系统的播头按
        // duration 回卷——burst 的时间轴与率曲线共用它。
        if system.emitter.looping && system.emitter.duration > 0.0 {
            while system.playback_head >= system.emitter.duration {
                system.playback_head -= system.emitter.duration;
                system.previous_head -= system.emitter.duration;
            }
        }
        for burst in &emission.bursts {
            let rand_fire = system.rng.next_f32();
            let rand_count = system.rng.next_f32();
            match burst_check(
                burst,
                system.previous_head,
                system.playback_head,
                rand_fire,
                rand_count,
            ) {
                BurstOutcome::Fired(count) => emitted += count,
                BurstOutcome::NotDue | BurstOutcome::MissedByProbability => {}
            }
        }
        if !system.emitter.looping && system.previous_head > system.emitter.duration {
            emitted = 0;
        }
        for _ in 0..emitted {
            spawn_one(system, ctx);
        }

    }

    let owner = compose_to_world(system, ctx);
    let velocity_over_lifetime = system.velocity_law.as_ref();
    let mut dead: Vec<usize> = Vec::new();
    for index in 0..system.pool.len() {
        let side = system.side[index];
        let start_lifetime = system.pool[index].start_lifetime;
        // 引擎的模块批（自旋/叠加速/限速）先于推进跑，读的是**推进前**
        // 的年寿进程量——余寿先留底。
        let pre_remaining = system.pool[index].remaining_lifetime;
        match advance_lifetime(
            &mut system.pool[index],
            dt,
            system.emitter.ring_buffer_mode,
            system.emitter.ring_buffer_loop_range,
        ) {
            LifetimeVerdict::Died => {
                dead.push(index);
                continue;
            }
            // 模式 2 回卷：律带回卷目标值，落值后下帧从头算（语料 14 条
            // 模式 2，回卷区间全 [0,1] 即整段循环）。
            LifetimeVerdict::Looped(remaining) => {
                set_remaining(&mut system.pool[index], remaining);
            }
            LifetimeVerdict::Alive(_) | LifetimeVerdict::PausedAtEnd => {}
        }
        // 寿限非正的按出生即死（律对非正寿限拒绝推进，会永生）。
        if !(start_lifetime > 0.0) {
            dead.push(index);
            continue;
        }
        let age_pre = moly_law::particle::step::normalized_age(pre_remaining, start_lifetime);
        // ---- 模块批（引擎次序：自旋 → 叠加速度 → 限速）----
        if let Some(rol) = &system.rol {
            let mut rot = system.side[index].rot;
            let _ = rol.advance_rotation(
                &mut rot,
                side.seed,
                0.0,
                age_pre * 100.0,
                system.emitter.start.rotation3d,
                dt,
            );
            system.side[index].rot = rot;
        }
        let batch_seed = system.side[index & !3].seed;
        let (anim, modifier) = match velocity_over_lifetime {
            Some(value) => motion::velocity_at_age(value, &system.pool[index], &side, batch_seed,
                system.emitter.simulation_space, &owner, age_pre, dt),
            None => ([0.0; 3], 1.0),
        };
        if let Some(law) = &system.limit {
            let mut velocity = system.pool[index].velocity;
            let _ = law.step(
                &mut velocity,
                anim,
                side.seed,
                age_pre * 100.0,
                dt,
                DragSize {
                    components: motion::size_at_age(system, &side, age_pre),
                    size3d: system.emitter.start.size3d || system.emitter.size_over_lifetime
                        .as_ref().is_some_and(|size| size.separate_axes),
                },
            );
            system.pool[index].velocity = velocity;
        }
        // The same pre-simulation speed modifier owns both orbital displacement
        // and integration. Animated velocity remains transient, including when
        // a zero speed modifier stops movement without clearing base velocity.
        let total = std::array::from_fn(|axis|
            system.pool[index].velocity[axis] + anim[axis]);
        let eff = total.map(|value| value * modifier);
        system.side[index].total_velocity = total;
        // 积分：律的 `integrate` 会用帧速度覆写状态速度——先存后还原。
        let state_velocity = system.pool[index].velocity;
        if let StepVerdict::Refused = integrate(&mut system.pool[index], dt, eff) {
            system.refused_total += 1;
        }
        system.pool[index].velocity = state_velocity;
        // 重力在积分**之后**（引擎次序：模块批 → 推进 → 重力；表情链同。
        // 雨/站点链是重力在积分前，各自注释里具名为未核——本链按引擎侧）。
        if side.gravity != 0.0 {
            apply_gravity(&mut system.pool[index], dt, GRAVITY, side.gravity);
        }
    }
    for &index in dead.iter().rev() {
        system.pool.swap_remove(index);
        system.side.swap_remove(index);
        system.died_total += 1;
    }
}

/// 出生一颗：形状抽样 → 出生取值 → 律的入池裁决。
fn spawn_one(system: &mut Runtime, ctx: &Context) {
    let (position, direction) = if let Some(shape) = system.emitter.shape.as_ref() {    // Current native RNG consumption: Circle/Cone 2, Sphere/Hemisphere 3,
    // SingleSidedEdge 1. A billboard's facing direction is not its birth velocity.
    let (local, raw_dir) = match shape.shape_type.as_str() {
        "Circle" => {
            let arc = system.rng.next_f32();
            let radial = system.rng.next_f32();
            circle_base(shape.radius, shape.radius_thickness, shape.arc, arc, radial)
        }
        "Cone" => {
            let angle = system
                .cone_angle
                .expect("判读已门 Cone 的 angle 在场");
            let t_theta = system.rng.next_f32();
            let t_radial = system.rng.next_f32();
            cone_base(
                shape.radius,
                shape.radius_thickness,
                angle,
                shape.arc,
                t_theta,
                t_radial,
            )
        }
        "ConeVolume" => {
            let arc = system.rng.next_f32();
            let radial = system.rng.next_f32();
            let distance = system.rng.next_f32();
            cone_volume(shape.radius, shape.radius_thickness,
                shape.controls.angle.expect("validated source cone angle"), shape.arc,
                shape.controls.length.expect("validated source cone length"), arc, radial, distance)
        }
        "Sphere" => {
            let t_theta = system.rng.next_f32();
            let t_cos = system.rng.next_f32();
            sphere_position(shape.radius, shape.radius_thickness, shape.arc, t_theta, t_cos, system.rng.next_f32())
        }
        "Hemisphere" => {
            let t_theta = system.rng.next_f32();
            let t_cos = system.rng.next_f32();
            hemisphere_position(shape.radius, shape.radius_thickness, shape.arc, t_theta, t_cos, system.rng.next_f32())
        }
        "Donut" => {
            let major_arc = system.rng.next_f32();
            let tube_angle = system.rng.next_f32();
            let radial = system.rng.next_f32();
            donut_position(shape.radius, shape.controls.donut_radius.expect("validated source donut radius"),
                shape.radius_thickness, shape.arc, major_arc, tube_angle, radial)
        }
        "SingleSidedEdge" => {
            let t_theta = system.rng.next_f32();
            single_sided_edge(shape.radius, t_theta)
        }
        other => panic!("判读已门形状族，运行时遇到 {other}——判读与推进的门不一致"),
    };
    // Shape-module TRS scales both position and velocity before rotation.
    // Independent engine measurements and current EmitterStoreData agree;
    // treating the scale as a billboard-size control would flatten the wrong data.
    let shape_scale = shape.controls.scale.unwrap_or([1.0; 3]);
    let local = euler_rotate_deg(shape.rotation, std::array::from_fn(|i| local[i] * shape_scale[i]));
    let position = [
        local[0] + shape.position[0],
        local[1] + shape.position[1],
        local[2] + shape.position[2],
    ];
    // 出发方向：形状函数的方向经形状旋转后归一（锥形函数按引擎分工返回
    // 未归一向量，归一在调用方）。
    let mut direction = euler_rotate_deg(shape.rotation, std::array::from_fn(|i| raw_dir[i] * shape_scale[i]));
    let length = (direction[0] * direction[0]
        + direction[1] * direction[1]
        + direction[2] * direction[2])
    .sqrt();
    if length > 1e-30 {
        direction = [
            direction[0] / length,
            direction[1] / length,
            direction[2] / length,
        ];
    } else {
        direction = [0.0; 3];
    }

    (position, direction)
    } else {
        assert_eq!(system.emitter.shape_enabled, Some(false), "missing shape requires explicit disabled-module evidence");
        ([0.0; 3], [0.0, 0.0, 1.0])
    };

    // ---- 出生取值表（表情链转录：逐项各抽一次，速度与重力共用稳定
    // 因子，种子 u32 最后一抽）----
    let r = system.rng.next_f32();
    let lifetime = system
        .emitter
        .start
        .lifetime
        .evaluate(0.0, system.rng.next_f32())
        .max(0.01);
    let size_x = system
        .emitter
        .start
        .size
        .evaluate(0.0, system.rng.next_f32());
    let size_y = match &system.emitter.start.size_y {
        Some(curve) => curve.evaluate(0.0, system.rng.next_f32()),
        None => size_x,
    };
    let size_z = match &system.emitter.start.size_z {
        Some(curve) => curve.evaluate(0.0, system.rng.next_f32()),
        None => size_x,
    };
    let colour = moly_law::particle::gradient::rgba8_to_float(
        moly_law::particle::color::initial_rgba8(
            &system.emitter.start.color, 0.0, system.rng.next_f32()));
    let spin0 = system
        .emitter
        .start
        .rotation
        .evaluate(0.0, system.rng.next_f32());
    let (spin_x, spin_y) = if system.emitter.start.rotation3d {
        (system.emitter.start.rotation_x.as_ref().expect("validated source X rotation")
            .evaluate(0.0, system.rng.next_f32()),
         system.emitter.start.rotation_y.as_ref().expect("validated source Y rotation")
            .evaluate(0.0, system.rng.next_f32()))
    } else { (0.0, 0.0) };
    let speed = system.emitter.start.speed.evaluate(0.0, r);
    let gravity = system.emitter.start.gravity_modifier.evaluate(0.0, r);
    let seed = system.rng.next_u32();

    // ---- 空间锚定：世界空间仿真出生即锚（位置过全变换、方向过线性部
    // 后已归一），局部空间仿真留在节点局部系逐帧换算 ----
    let kind = system.kind;
    let camera_rotation = system.camera_rotation;
    let node_affine = system.node_affine;
    // 尺寸吃链缩放：语料 35 条**局部空间**条目带非恒等链缩放（4.0 ×28、
    // 0.9994 ×7，全部均匀；世界空间条目的链全恒等），折进出生尺寸与
    // 「渲染时折算」给出同一结果。非均匀链缩放语料里不存在，按 X 分量
    // 处理（具名记为未实现）。
    // Particle module coordinates are raw Unity coordinates; GLB and game
    // anchors use the producer's reflected-X basis. Convert once at this boundary.
    let position = crate::particle_geometry::reflect(Vec3::from_array(position)).to_array();
    let direction = crate::particle_geometry::reflect(Vec3::from_array(direction)).to_array();
    let (position, direction) = if system.emitter.simulation_space == SimulationSpace::World {
        let anchor = match kind {
            EffectKind::Sky => ctx.sky,
            EffectKind::Camera => {
                if camera_rotation {
                    ctx.camera
                } else {
                    GlobalTransform::from_translation(ctx.camera.translation())
                }
            }
            EffectKind::Site => ctx.site,
        };
        let to_world = anchor * node_affine;
        let position = to_world.transform_point(Vec3::from_array(position)).to_array();
        let dir = to_world.affine().transform_vector3(Vec3::from_array(direction));
        let length = dir.length();
        let dir = if length > 1e-30 {
            (dir / length).to_array()
        } else {
            direction
        };
        (position, dir)
    } else {
        (position, direction)
    };
    let velocity = [
        direction[0] * speed,
        direction[1] * speed,
        direction[2] * speed,
    ];

    let side = Side {
        rand: r,
        seed,
        rot: [spin_x, spin_y, spin0],
        size: [size_x, size_y, size_z],
        gravity,
        colour,
        total_velocity: velocity,
    };
    let particle = Particle::born(position, velocity, lifetime);
    match ring_push(
        &mut system.pool,
        &mut system.ring_cursor,
        system.emitter.ring_buffer_mode,
        system.emitter.max_particles as usize,
        particle,
    ) {
        RingPushVerdict::Appended => {
            system.side.push(side);
            system.born_total += 1;
        }
        RingPushVerdict::Replaced { index } => {
            system.side[index] = side;
            system.born_total += 1;
        }
        RingPushVerdict::Full => {
            system.full_total += 1;
        }
    }
}



pub(crate) fn write_geometry(
    mesh: &mut Mesh, system: &Runtime, to_world: &GlobalTransform,
    owner: &GlobalTransform, camera: &GlobalTransform, basis: crate::billboard::CameraBasis,
) {
    match &system.geometry {
        Geometry::Billboard { alignment, clamp, pivot } => {
            crate::billboard::write_quads(mesh, &build_quads(system, to_world), *alignment, basis, *clamp, *pivot);
        }
        Geometry::Mesh(draw) => {
            let frame = crate::particle_geometry::source_frame(owner, camera);
            let appearance = build_quads(system, to_world);
            let instances: Vec<_> = system.pool.iter().enumerate().map(|(index, particle)| {
                let side = system.side[index];
                let age = particle.normalized_age();
                let size = Vec3::from_array(motion::size_at_age(system, &side, age));
                let view = &appearance[index];
                crate::particle_geometry::Instance {
                    position: crate::particle_geometry::reflect(view.centre),
                    velocity: crate::particle_geometry::reflect(to_world.affine().transform_vector3(Vec3::from_array(side.total_velocity))),
                    rotation: Vec3::from_array(side.rot), size,
                    colour: Vec4::from_array(view.colour),
                    custom1: Vec4::from_array(view.custom1), custom2: Vec4::from_array(view.custom2),
                }
            }).collect();
            crate::particle_geometry::write_mesh(mesh, draw, &instances, &frame);
        }
    }
}
