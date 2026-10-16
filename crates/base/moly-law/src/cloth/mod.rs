//! 布料律：SD 角色头发/饰品链的二级物理（Magica Cloth 2 骨布模式的
//! 可复算核心）。纯函数 + dt 显式入参，与 [`crate::path`] 同款范式；
//! 引擎的 Transform 读写归上屏接线单，本模块只认平数据。
//!
//! # 真源与锚
//!
//! 真源机制（负责人 2026-09-07 核定）：SD 站点角色的二级
//! 运动全部由 Magica Cloth 2 骨布驱动——动画库 42 条目标链不含任何
//! 头发/饰品骨，每角色一份 `sd_*.rig.json` 的 `cloth` 节携带全部
//! 数据。律锚 = `MagicaCloth.dll` 反编译（伪 C），demo `cloth.js`
//! （用户验收过的先行实现）只作对照臂。
//!
//! # 反编译锚定的主干（PhysicsManagerCompute.UpdateStartSimulation）
//!
//! ```text
//! CalcMaxUpdateCount(dt)          → 本帧子步数（90Hz 定步长累加器）
//! UpdateBoneToParticle            → 读动画世界位置
//! 循环 n 次 UpdatePhysics：       → 一次子步
//!     ForceAndVelocityJob         → Verlet 积分（阻尼/速度上限/重力·dt²）
//!     约束迭代 ×SolverIterationCount，按注册序：
//!         ColliderCollision       → 碰撞
//!         ClampDistance           → 根距比钳制（minRatio/maxRatio）
//!         RestoreDistance         → 结构距离（刚度 + 质量分摊）
//!         CompositeRotation       → 限角 + 回拉（algorithm=1 的组合体）
//!     FixPositionJob              → 固定点回钉 / 速度簿记
//! UpdateParticleToBone + ConvertWorldToLocal → 写回骨_TRANSFORM
//! ```
//!
//! 注册序（Create 的 ctor+Add 序列）中 Extrusion/Penetration/Spring/
//! Twist/TriangleBend/ClampPosition 在本批数据上全部未注册
//! （constraintCounts 全 0 或 use 开关为假）；本批 31 份 rig 的
//! selection 普查只有 fixed(482)/move(990)，无 extend。
//!
//! # 与 demo 对照臂的形差（差异都追到了反编译层）
//!
//! | 点 | demo cloth.js | 反编译（本模块采） |
//! |---|---|---|
//! | 根距钳制修正 | 直接夹到 `min/maxRatio·len` | 每次迭代按 `clamp01(1-velInfl·0.5)` 逼近（×4 迭代收敛，velInfl=0.2） |
//! | 结构距离分摊 | 固定 50/50、按边去重 | 按 `(mass_j+5·infl_j)/(mass_i+5·infl_i+mass_j+5·infl_j)` 单侧分摊，条目按 rig 双向原样 |
//! | 限角参数 | `clampRotationAngle` | `GetClampRotationAngle(algorithm=1)` = `clampRotationAngle2`（值恰同：3°→20°） |
//! | 回拉参数 | `restoreRotation`（0.11→0.02） | `GetRestoreRotationPower(algorithm=1)` = `restoreRotation2`（0.10→0.02） |
//! | 回拉时机 | 每子步一次、迭代前 | CompositeRotation 注册在迭代环内（组合体内部式子只部分可读，见 solver.rs 注） |
//! | 速度吸收 | 无（PBD 惯例：修正全额漏进速度） | 独立速度缓冲按 `(1-velInfl)` 吸收（本律 pos/prev 形，见 solver.rs 注） |
//! | 碰撞摩擦 | 无 | 最深命中 0.03m 深度窗内做摩擦簿记（数据 0.05/0.03，量级可忽略） |
//!
//! # 风（具名结论，未决②）
//!
//! 每链 windInfluence=1.0 / windRandomScale=0.7 / windSynchronization=0.6
//! （三单位全链一致），但风源=场景里登记的 MagicaWind 组件；
//! SD prefab 组件直方图（rig `stats`）与站点都没有——
//! `windDataList` 为空 ⇒ `Wind()` 对空表求和 ≡ 0。具名「无风」：
//! 参数在、源不在，本律不造风；外部力入口
//! [`solver::FrameInput::external_force`] 默认零。

pub mod bezier;
pub mod collider;
pub mod math;
pub mod schema;
pub mod solver;

pub use bezier::BezierParam;
pub use collider::{ColliderDef, WorldCollider};
pub use schema::{
    cloth_from_value, json_parse, params_from_value, ChainDef, ClothParamsDef, Selection,
};
pub use solver::{
    advance, build_chain, ChainTopology, ClothState, EvaluatedParams, FrameInput, StepStats,
};


