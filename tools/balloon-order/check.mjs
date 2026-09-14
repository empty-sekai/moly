#!/usr/bin/env node
// 头顶 tweet 气泡绘制层序的成对数值判据（研究仪器，无图无像素）。
//
// 两只屏幕上**完全重叠**的探针气泡由冒烟旋钮 `MOLY_BALLOON_ORDER_SECS`
// 造成（近根在相机正前方视线轴上，远锚取过近锚点的同一视线射线两倍
// 距离处——两只锚点投影到同一屏幕点）。运行产物进程并从日志读出每
// 部件的**最终排序键**（根部 z + 根缩放 × animation 节点缩放 × 部件
// 层 z，与渲染器同式），断言两层顺序：
//
//   C1 跨气泡（成对判据之一）：近者的**全部**部件 z 严格大于远者的
//      **全部**部件 z（区间不相交——同一气泡的部件整体成层，绝不与
//      另一只交错）。出处：HUD 层每帧按「相机到各目标的欧氏距离」
//      降序重排兄弟序，近者最后画、盖住远者。
//   C2 同气泡内（成对判据之二）：背板 < 正文 < 箭头。出处：HUD 预制体
//      animation 节点的子序——bg 子树在前（正文文本件是 bg 的子节点），
//      箭头容器最后，后者画在上。
//
// 反向臂（`MOLY_BALLOON_ORDER_NO_LAYER=1`：根部 z 全喂 0，即接线前的
// 恒 0 行为）：C1 必须红（近/远区间重合，不再有覆盖序）；C2 仍绿
// （部件层 z 不随根部接线变）。两条臂都跑，全绿才退出 0。
//
// 用法：node tools/balloon-order/check.mjs [moly-app.exe]
//   exe 缺省取仓根的 moly-app-*.exe。每臂的完整日志落在仓根
//   balloon-order-<run-id>-*.log 及对应 JSON 状态供复查。

import { spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const WINDOW_SECS = '10.0';
const RUN_TIMEOUT_MS = Number(process.env.MOLY_BALLOON_ORDER_TIMEOUT_MS ?? 90_000);
if (!Number.isSafeInteger(RUN_TIMEOUT_MS) || RUN_TIMEOUT_MS <= 0) throw new Error('Invalid probe timeout');
const STRIDE = 4.0;

const READY_RE =
  /\[tweet-order\] probe ready near_tweet=(\d+) near_d=([\d.eE+-]+) far_tweet=(\d+) far_d=([\d.eE+-]+)/;
const PART_RE =
  /\[tweet-order\] probe=(\d) tweet=(\d+) part=(bg|text|arrow) base=([\d.eE+-]+) k=([\d.eE+-]+) z=([\d.eE+-]+)/;

function findExe() {
  if (process.argv.length > 2) {
    return process.argv[2];
  }
  const root = path.resolve(import.meta.dirname, '..', '..');
  const hits = fs
    .readdirSync(root)
    .filter((name) => /^moly-app-.*\.exe$/.test(name))
    .sort();
  if (hits.length === 0) {
    console.error('[check] 找不到 exe：请把 moly-app-*.exe 放进仓根，或作为参数传入');
    process.exit(2);
  }
  return path.join(root, hits[hits.length - 1]);
}

function runArm(exe, armName, extraEnv) {
  const runId = randomUUID();
  const env = { ...process.env, MOLY_BALLOON_ORDER_SECS: WINDOW_SECS, MOLY_BALLOON_ORDER_RUN_ID: runId };
  env.RUST_LOG = `${process.env.RUST_LOG ?? 'warn'},moly_game::balloon=info`;
  delete env.MOLY_BALLOON_ORDER_NO_LAYER;
  Object.assign(env, extraEnv);
  const executableSha256 = createHash('sha256').update(fs.readFileSync(exe)).digest('hex');
  const proc = spawnSync(exe, [], {
    env,
    cwd: path.dirname(exe),
    timeout: RUN_TIMEOUT_MS,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  });
  const out = `${proc.stdout ?? ''}${proc.stderr ?? ''}`;
  const logPath = path.join(path.dirname(exe), `balloon-order-${runId}-${armName}.log`);
  fs.writeFileSync(logPath, out, 'utf8');
  const result = { runId, armName, executableSha256, logPath,
    status: proc.status, signal: proc.signal,
    error: proc.error ? { code: proc.error.code, message: proc.error.message } : null };
  fs.writeFileSync(logPath.replace(/\.log$/, '.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(`[${armName}] log: ${logPath}`);
  if (result.error || result.signal || result.status !== 0) {
    throw new Error(`Probe process failed: ${JSON.stringify(result)}`);
  }
  const identity = /\[tweet-order\] run=([0-9a-f-]+) version=(\S+)/.exec(out);
  if (!identity || identity[1] !== runId || identity[2] !== expectedVersion) {
    throw new Error(`Probe run/version identity differs; see ${logPath}`);
  }
  return { out, ...result };
}

function parse(out) {
  const ready = READY_RE.exec(out) ?? null;
  const parts = [];
  for (const m of out.matchAll(new RegExp(PART_RE.source, 'g'))) {
    parts.push({
      probe: Number(m[1]),
      tweet: Number(m[2]),
      kind: m[3],
      base: Number(m[4]),
      k: Number(m[5]),
      z: Number(m[6]),
    });
  }
  return { ready, parts };
}

const zs = (parts, tweet, kind = null) =>
  parts
    .filter((p) => p.tweet === tweet && (kind === null || p.kind === kind))
    .map((p) => p.z);

function checkC1(parts, near, far, arm) {
  const nearZs = zs(parts, near);
  const farZs = zs(parts, far);
  if (nearZs.length === 0 || farZs.length === 0) {
    console.log(`[${arm}] C1 跨气泡：部件缺失（近 ${nearZs.length} 件 / 远 ${farZs.length} 件）→ 红`);
    return false;
  }
  const ok = Math.min(...nearZs) > Math.max(...farZs);
  console.log(
    `[${arm}] C1 跨气泡：近 tweet=${near} z∈[${Math.min(...nearZs).toFixed(3)},${Math.max(
      ...nearZs
    ).toFixed(3)}]（${nearZs.length} 件） 远 tweet=${far} z∈[${Math.min(...farZs).toFixed(
      3
    )},${Math.max(...farZs).toFixed(3)}]（${farZs.length} 件） → ${ok ? '绿：min(近) > max(远)' : '红'}`
  );
  return ok;
}

function checkC2(parts, tweets, arm) {
  let ok = true;
  for (const tweet of tweets) {
    const bg = zs(parts, tweet, 'bg');
    const text = zs(parts, tweet, 'text');
    const arrow = zs(parts, tweet, 'arrow');
    const complete = bg.length > 0 && text.length > 0 && arrow.length > 0;
    const good = complete && Math.max(...bg) < Math.min(...text) && Math.max(...text) < Math.min(...arrow);
    ok = ok && good;
    const desc = complete
      ? `背板 z≤${Math.max(...bg).toFixed(3)}(${bg.length}件) < 正文 z≥${Math.min(...text).toFixed(
          3
        )}(${text.length}件) < 箭头 z≥${Math.min(...arrow).toFixed(3)}(${arrow.length}件)`
      : `族缺失（背板 ${bg.length} / 正文 ${text.length} / 箭头 ${arrow.length}）`;
    console.log(`[${arm}] C2 同气泡内 tweet=${tweet}：${desc} → ${good ? '绿' : '红'}`);
  }
  return ok;
}

function checkSelf(parts, ready, arm) {
  const errors = [];
  if (ready === null) {
    errors.push('缺 probe ready 行（探针没造出来：主表/图集/相机未就绪，或窗口太短）');
    return { errors, near: null, far: null };
  }
  const near = Number(ready[1]);
  const far = Number(ready[3]);
  const nearD = Number(ready[2]);
  const farD = Number(ready[4]);
  const probeParts = parts.filter((p) => p.probe === 1);
  if (![near, far, nearD, farD, ...parts.flatMap(p => [p.tweet, p.base, p.k, p.z])].every(Number.isFinite)) {
    errors.push('探针包含非有限数值');
  }
  for (const [tweet, name] of [
    [near, '近'],
    [far, '远'],
  ]) {
    const own = probeParts.filter((p) => p.tweet === tweet);
    const counts = {
      bg: own.filter((p) => p.kind === 'bg').length,
      text: own.filter((p) => p.kind === 'text').length,
      arrow: own.filter((p) => p.kind === 'arrow').length,
    };
    if (counts.bg < 9) errors.push(`${name} tweet=${tweet} 背板件 ${counts.bg} < 9（九宫缺件）`);
    if (counts.text < 1) errors.push(`${name} tweet=${tweet} 正文件 0（字形缺失）`);
    if (counts.arrow !== 1) errors.push(`${name} tweet=${tweet} 箭头件 ${counts.arrow} != 1`);
    if (own.length > 0 && !own.every((p) => p.k > 0)) {
      errors.push(`${name} tweet=${tweet} 存在 k<=0 的部件（稳态闩到了非稳态帧）`);
    }
  }
  if (!(nearD < farD)) errors.push(`探针构造反了：near_d=${nearD} >= far_d=${farD}`);
  console.log(
    `[${arm}] 自检：近 tweet=${near} d=${nearD.toFixed(3)} / 远 tweet=${far} d=${farD.toFixed(
      3
    )}，探针部件 ${probeParts.length} 件 → ${errors.length === 0 ? '绿' : `红：${errors.join('；')}`}`
  );
  return { errors, near, far };
}

const root = path.resolve(import.meta.dirname, '..', '..');
const expectedVersion = /\[workspace\.package\][\s\S]*?\bversion\s*=\s*"([^"]+)"/
  .exec(fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8'))?.[1];
if (!expectedVersion) throw new Error('Workspace version is missing');
const exe = path.resolve(findExe());
const failures = [];

// 正向臂：接线在——C1 绿、C2 绿、自检绿。
{
  const { out } = runArm(exe, 'a', {});
  const { ready, parts } = parse(out);
  const { errors, near, far } = checkSelf(parts, ready, '正向臂');
  for (const e of errors) failures.push(`正向臂自检：${e}`);
  if (near !== null && far !== null) {
    if (!checkC1(parts, near, far, '正向臂')) failures.push('正向臂 C1 红（近者未整套盖住远者）');
    if (!checkC2(parts, [near, far], '正向臂')) failures.push('正向臂 C2 红（同气泡内层序错）');
    const nearBases = parts.filter((p) => p.tweet === near).map((p) => p.base);
    const farBases = parts.filter((p) => p.tweet === far).map((p) => p.base);
    if (nearBases.length > 0 && farBases.length > 0) {
      const nearBase = Math.max(...nearBases);
      const farBase = Math.min(...farBases);
      console.log(
        `[正向臂] 名次档：近 base=${nearBase.toFixed(3)} 远 base=${farBase.toFixed(
          3
        )}（差 ${(nearBase - farBase).toFixed(3)} ≥ 步进 ${STRIDE} 才不相交）`
      );
      if (!(nearBase - farBase >= STRIDE)) failures.push(`正向臂名次档差 < ${STRIDE}（步进接线可疑）`);
    }
  }
}

// 反向臂：根部 z 全喂 0（= 接线前恒 0）——C1 必须红，C2 必须仍绿。
{
  const { out } = runArm(exe, 'b', { MOLY_BALLOON_ORDER_NO_LAYER: '1' });
  const { ready, parts } = parse(out);
  const { errors, near, far } = checkSelf(parts, ready, '反向臂');
  for (const e of errors) failures.push(`反向臂自检：${e}`);
  if (near !== null && far !== null) {
    if (checkC1(parts, near, far, '反向臂')) {
      failures.push('反向臂 C1 仍绿（喂相同深度时它应当红——这条判据对排序接线是盲的）');
    } else {
      console.log('[反向臂] C1 如期红（根部 z 全 0 ⇒ 近/远区间重合，无覆盖序）');
    }
    if (!checkC2(parts, [near, far], '反向臂')) failures.push('反向臂 C2 红（部件层 z 不应随根部接线变）');
  }
}

if (failures.length > 0) {
  console.log('[check] 未通过：');
  for (const f of failures) console.log(`  - ${f}`);
  process.exit(1);
}
console.log('[check] 成对判据全过：正向 C1+C2 绿，反向 C1 红、C2 绿。');
