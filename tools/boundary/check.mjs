// tools/boundary/check.mjs — public/internal boundary gate for the moly repo.
//
// Standalone rewrite of a Python boundary test whose `forbidden` tuple
// and rationale this gate carries over. Scope is broader than that file's: that
// test only walks a curated "public tree" subset (src/, docs/, examples/,
// top-level README*); this one walks every file `git ls-files` reports for
// this repo, because moly has no such curated subset — anything tracked here
// can leak. It also walks untracked files that look like source under a
// source directory, because those are release candidates: `git add` publishes
// them, and nothing else inspects them first. Untracked non-source files are
// not scanned, and main() prints how many there were on every run so a "0
// violations" line can never be read as "the working tree is clean".
//
// The list of source directories is hand-maintained, so it is itself checked:
// every directory holding a source file (tracked or untracked) must be covered
// by that list, or the gate fails and names the directory. See
// checkSourceDirCoverage().
//
// What it looks for:
//   1. machine-local absolute paths — evidence that a path from this
//      developer's own disk leaked into a tracked file (see git commit
//      73fa427, which is exactly that: four such paths, unnoticed until this
//      gate was written).
//   2. internal-only workflow vocabulary — role/process nouns that only make
//      sense inside the private orchestration this repo is built under.
//   3. everything else in that test's
//      `forbidden` tuple, carried over verbatim (including that file's own
//      obfuscation of the two real third-party domain names — that
//      obfuscation is about never spelling a real external domain in
//      cleartext anywhere, not about self-scan avoidance, so it is preserved
//      here as-is rather than "cleaned up").
//
// Per this repo's boundary rule: a
// desensitization scanner's own rule strings and test inputs must stay
// literal — obfuscating them to dodge self-detection "just moves the leak
// somewhere else" and would let a marker silently drift from what actually
// gets matched. So the markers below are plain literals, and this file scans
// itself like any other tracked file; the resulting self-matches are handled
// by the ALLOWLIST below, each with a reason, not by rewriting the markers.

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.join(HERE, '..', '..'); // tools/boundary -> tools -> repo root
const SELF_REL_PATH = 'tools/boundary/check.mjs';

// boundary:allow-begin
// Only the lines between here and the matching boundary:allow-end sentinel
// are exempt from content scanning in this file (see ALLOWLIST and
// computeAllowedLines() below). Nothing outside this fence is.
// Category 1: machine-local absolute path shapes (matched case-insensitively).
const LOCAL_PATH_MARKERS = [
  'F:/',
  'F:\\',
  'C:\\Users',
  '/f/mysekai',
  '/c/Users',
];

// Category 2: internal role / workflow vocabulary. Currently 0 hits outside
// this file's own definition below — this gate exists to keep it that way.
const WORKFLOW_TERM_MARKERS = [
  'team-leader',
  '宿主组',
  '律组',
  '呈现组',
  '派工单',
  '工作单',
  'dev-high',
  'dev-low',
];

// Category 3: carried over verbatim from the original boundary test's
// `forbidden` tuple, minus the two path shapes already in category 1 above.
const INHERITED_PUBLIC_BOUNDARY_MARKERS = [
  'allium',
  'RE-jp',
  // The other decompile tree's codename. 'RE-jp' above caught one tree and
  // not this one, so an anchor written against this tree read as clean for as
  // long as it sat in the product layer: a contract file carried this
  // codename plus a line range and the gate reported 0 violations. One
  // codename in the table is not the family.
  'cn-6.0.0',
  // Narrowed from the bare word 'pseudo': it fired on ordinary English prose
  // (pseudo-random, pseudo-noise, pseudocode) while the real concern is a
  // leaked path into the decompiled tree, or prose disclosing that such a
  // tree exists. Both path separators are covered -- the backslash form is
  // the one that keeps getting missed. These are built from code points so
  // this file contains no literal backslash for a matcher to trip over.
  'pseudo/',
  'pseudo' + String.fromCharCode(92),
  'pseudo ' + String.fromCharCode(0x6811),
  'pseudo tree',
  'daily' + 'gn.com',
  'byted' + 'game.com',
];

// Category 4: assembly names. The boundary rule makes these a hard line and
// nothing enforced it -- the categories above carry tree codenames and host
// paths, not the name of a managed assembly, so an anchor written as an
// assembly-qualified type read as clean.
//
// Only the qualified forms are listed. A bare type name is a source symbol
// and the rule permits it; matching the bare name would forbid writing
// about the source at all, which is not the rule.
const ASSEMBLY_MARKERS = [
  'Assembly-CSharp',
  'Unity.RenderPipelines',
  'Unity.TextMeshPro',
];
// boundary:allow-end

const ALL_MARKERS = [
  ...LOCAL_PATH_MARKERS.map((marker) => ({ marker, category: 'local-absolute-path' })),
  ...WORKFLOW_TERM_MARKERS.map((marker) => ({ marker, category: 'internal-workflow-term' })),
  ...INHERITED_PUBLIC_BOUNDARY_MARKERS.map((marker) => ({ marker, category: 'inherited-boundary-marker' })),
  ...ASSEMBLY_MARKERS.map((marker) => ({ marker, category: 'assembly-name' })),
];

// Files listed here get a NARROW exemption, not a whole-file one: only the
// lines strictly between a `// boundary:allow-begin` / `// boundary:allow-end`
// sentinel pair in that file (see computeAllowedLines() below) are skipped.
// Everything else in the file is scanned like any other file in the scan set.
// Being listed here does not by itself exempt anything — a listed file with
// no sentinel pair is scanned in full. Keep this list to exactly the files
// whose fenced region must hold the markers verbatim; never widen a fence
// past the literal marker-table or sample lines. Reason strings here must not
// themselves contain a local path.
const ALLOWLIST = new Map([
  [
    SELF_REL_PATH,
    'the boundary gate\'s own rule tables (fenced below) must hold every ' +
      'marker literally — a scanner that obfuscates its own markers just ' +
      'moves the leak elsewhere.',
  ],
  [
    'tools/no-host-paths.test.mjs',
    'this test asserts that neither boundary gate prints a machine-local ' +
      'path on a real run; its shape table and its positive-control input ' +
      'must hold those shapes literally, for the same reason the gate\'s own ' +
      'tables do. Fenced to those lines only.',
  ],
  [
    'tools/relpath.test.mjs',
    'unit test for the path relativizer: its inputs must be absolute-path ' +
      'literals or there is nothing to relativize. The roots used are ' +
      'fixtures, not this machine\'s tree. Fenced to those lines only.',
  ],
]);

function gitTrackedFiles() {
  const out = execFileSync('git', ['ls-files', '-z'], { cwd: REPO_ROOT, encoding: 'utf8' });
  return out.split('\0').filter(Boolean);
}

// Files git knows about but does not track, minus anything the ignore rules
// already exclude (--exclude-standard). Ignored files are build output and
// local scratch — they can never enter a release by being added, so they are
// deliberately not part of this set.
function gitUntrackedFiles() {
  const out = execFileSync('git', ['ls-files', '--others', '--exclude-standard', '-z'], {
    cwd: REPO_ROOT,
    encoding: 'utf8',
  });
  return out.split('\0').filter(Boolean);
}

// An untracked file that looks like source under a source directory is a
// release candidate: the moment someone runs `git add` on it, it is in the
// published set, and until this gate looked at untracked files nothing
// inspected it beforehand. So these are scanned and can fail the gate even
// though they are not tracked yet. Everything else untracked (notes, data,
// generated artifacts, non-source tooling) is not scanned — see the blind
// spot line printed by main().
// This set does NOT decide what gets scanned among tracked files — every
// tracked file is scanned, whatever its extension. It only decides which
// *untracked* files count as release candidates, i.e. which ones get looked at
// in the window between authoring a file and `git add`-ing it.
//
// `.toml` and `.json` were missing from that window, and `src/units/*/unit.toml`
// is the file class most likely to carry a decompiler path or address, because
// its key comments exist precisely to explain why the original game behaves as
// it does. A brand-new unit is untracked until it is added, so that is
// exactly when the check was blind. Compressed and binary fixtures (.br, .brz,
// .bin, .gzz) stay out: they are not text, and scanning them would report
// matches no human wrote.
const SOURCE_EXTENSIONS = new Set(['.ts', '.rs', '.wgsl', '.mjs', '.js', '.html', '.toml', '.json', '.py', '.yml', '.yaml']);

// Directory prefixes (trailing slash mandatory) holding source that is part of
// the published product. This list is hand-maintained, which is exactly why
// checkSourceDirCoverage() below asserts it has not fallen behind the tree:
// a stale list does not fail, it silently stops classifying a whole directory
// of release candidates as release candidates.
const SOURCE_DIR_PREFIXES = ['web/', 'tools/', 'src/', 'tests/', '.cargo/', '.github/'];

// Per-crate source directories. Enumerated rather than collapsed into a single
// `^crates/[^/]+/` catch-all on purpose: a catch-all would cover every future
// subdirectory of every crate in advance, which is precisely what would keep
// the coverage assertion below from ever firing inside the crates tree.
const CRATE_SOURCE_DIR_PATTERNS = [
  /^crates\/[^/]+\/src\//,
  /^crates\/[^/]+\/tests\//,
  // examples/ ships with the crate and `git add` publishes it like any
  // other source. It held tracked source while this list named only src/
  // and tests/, which is the exact staleness checkSourceDirCoverage()
  // exists to catch -- and did.
  /^crates\/[^/]+\/examples\//,
];

// A source file sitting directly in a crate's own directory rather than in one
// of its subdirectories (per-crate build and smoke scripts live here).
const CRATE_ROOT_DIR = /^crates\/[^/]+\/$/;

const SOURCE_DIR_DESCRIPTION =
  'the repo root, web/, tools/, src/, tests/, .cargo/, .github/, crates/*/ and crates/*/{src,tests,examples}/';

// Directories deliberately outside the coverage assertion, each with the reason
// it is out. Stated explicitly rather than left to "no prefix happens to match
// it": relying on an accidental non-match means the next layout change either
// scans generated output or reports it as an uncovered directory, and both look
// like a gate malfunction. On the untracked side these are normally already
// gone before this list is consulted, because gitUntrackedFiles() passes
// --exclude-standard and every entry here is covered by an ignore rule; the
// list is what keeps that true for the tracked side and for any of these that
// stops being ignored. main() prints how many directories it removed, so it can
// never quietly eat the enumeration.
const EXCLUDED_DIR_PATTERNS = [
  {
    pattern: /(^|\/)target\//,
    reason: 'cargo build output; the .rs under it is build-script generated, never authored or added here',
  },
  {
    pattern: /(^|\/)node_modules\//,
    reason: 'third-party packages; vendored .js/.ts is not this repo\'s source',
  },
  {
    pattern: /(^|\/)dist\//,
    reason: 'bundler output, rebuilt from web/src',
  },
  {
    pattern: /(^|\/)\.build\//,
    reason: 'build scratch directory',
  },
  {
    pattern: /(^|\/)__pycache__\//,
    reason: 'python bytecode cache; holds no source-extension file today, listed so that stays a decision',
  },
  {
    pattern: /(^|\/)_scratch\//,
    reason: 'measurement scratch, rebuildable from the asset root, never version controlled',
  },
];

// '' for a file with no directory component (i.e. one at the repo root).
function dirOf(relPath) {
  const normalized = relPath.split(path.sep).join('/');
  const i = normalized.lastIndexOf('/');
  return i === -1 ? '' : normalized.slice(0, i);
}

function withTrailingSlash(dirRel) {
  if (dirRel === '') return '';
  return dirRel.endsWith('/') ? dirRel : `${dirRel}/`;
}

// The single coverage predicate. isReleaseCandidateSource() and
// checkSourceDirCoverage() both go through this one function, so the filter
// that decides what gets scanned and the assertion that the directory list is
// complete can never disagree about what "covered" means.
function isSourceDir(dirRel) {
  if (dirRel === '') return true; // repo root: build scripts live here
  const withSlash = withTrailingSlash(dirRel);
  if (SOURCE_DIR_PREFIXES.some((prefix) => withSlash.startsWith(prefix))) return true;
  if (CRATE_SOURCE_DIR_PATTERNS.some((re) => re.test(withSlash))) return true;
  if (CRATE_ROOT_DIR.test(withSlash)) return true;
  return false;
}

function hasSourceExtension(relPath) {
  return SOURCE_EXTENSIONS.has(path.extname(relPath.split(path.sep).join('/')).toLowerCase());
}

function isReleaseCandidateSource(relPath) {
  if (!hasSourceExtension(relPath)) return false;
  return isSourceDir(dirOf(relPath));
}

// ---------------------------------------------------------------------------
// Meta-check: the directory list above must not fall behind the repository
// layout. The gap this closes is not a missing marker, it is a hand-maintained
// list going stale — the state that produced it here was `crates/*/tests/`
// holding source while the list named only `crates/*/src/`, so three files that
// `git add` would publish were classified as "not source" and never scanned.
// Nothing failed; the count just came out three lower and complete-looking.
//
// So: every directory that holds a source file — tracked or untracked — must be
// covered by isSourceDir(). Any that is not is named as a violation. Directories
// removed by EXCLUDED_DIR_PATTERNS are reported separately and not asserted on.
// ---------------------------------------------------------------------------

// Floor for the number of directories actually asserted on. Without it an
// enumeration that collapsed to zero directories would report "every directory
// is covered" — vacuously true and indistinguishable from a real pass. The
// tree holds tens of source directories across web/, src/, crates/ and tools/;
// 5 is a floor, not an estimate.
const MIN_SOURCE_DIRS = 5;

function checkSourceDirCoverage(trackedFiles, untrackedFiles) {
  const dirs = new Map(); // dir -> first source file seen in it
  for (const [list, tracked] of [[trackedFiles, true], [untrackedFiles, false]]) {
    for (const rel of list) {
      if (!hasSourceExtension(rel)) continue;
      const normalized = rel.split(path.sep).join('/');
      const dir = dirOf(normalized);
      if (!dirs.has(dir)) dirs.set(dir, { example: normalized, tracked });
    }
  }

  const excluded = [];
  const checked = [];
  const uncovered = [];
  for (const [dir, info] of dirs) {
    const withSlash = withTrailingSlash(dir);
    const rule = EXCLUDED_DIR_PATTERNS.find((e) => e.pattern.test(withSlash));
    if (rule) {
      excluded.push({ dir, reason: rule.reason });
      continue;
    }
    checked.push(dir);
    if (!isSourceDir(dir)) uncovered.push({ dir, ...info });
  }
  return { checked, excluded, uncovered };
}

function displayDir(dir) {
  return dir === '' ? '<repo root>' : `${dir}/`;
}

// Shared matching core: given raw text, return every marker hit (without a
// file name attached yet). scanFile() below is real-file plumbing around
// this; the self-check further down runs this exact function against
// synthetic text, so both paths are provably the same code.
function findMarkerHits(text) {
  const hits = [];
  const lines = text.split('\n');
  for (const { marker, category } of ALL_MARKERS) {
    const markerLower = marker.toLowerCase();
    lines.forEach((line, idx) => {
      if (line.toLowerCase().includes(markerLower)) {
        hits.push({ line: idx + 1, marker, category });
      }
    });
  }
  return hits;
}

const ALLOW_BEGIN = /^\s*\/\/\s*boundary:allow-begin\s*$/;
const ALLOW_END = /^\s*\/\/\s*boundary:allow-end\s*$/;

// 1-based line numbers strictly between a boundary:allow-begin/-end sentinel
// pair (the sentinel lines themselves are still scanned — they contain no
// marker literal). Sentinel-based rather than a hardcoded range so the fence
// tracks the file as lines are added/removed above it instead of drifting.
function computeAllowedLines(text) {
  const lines = text.split('\n');
  const allowed = new Set();
  let openAt = null;
  lines.forEach((line, idx) => {
    const n = idx + 1;
    if (ALLOW_BEGIN.test(line)) {
      if (openAt !== null) throw new Error(`boundary:allow-begin at line ${n} nested inside one open at line ${openAt}`);
      openAt = n;
    } else if (ALLOW_END.test(line)) {
      if (openAt === null) throw new Error(`boundary:allow-end at line ${n} has no matching boundary:allow-begin`);
      for (let l = openAt + 1; l < n; l++) allowed.add(l);
      openAt = null;
    }
  });
  if (openAt !== null) throw new Error(`boundary:allow-begin at line ${openAt} has no matching boundary:allow-end`);
  return allowed;
}

// Reads a scannable file's text, or null when there is nothing to scan.
// Shared by the violation scan and the debt lane below so the two always
// look at the same set of bytes.
function readScannableText(relPath) {
  const absPath = path.join(REPO_ROOT, relPath);
  let text;
  try {
    text = readFileSync(absPath, 'utf8');
  } catch {
    // Unreadable (deleted-but-tracked race, permission error) — can't scan
    // content; report nothing rather than crash the whole gate over one file.
    return null;
  }
  // NUL byte is not valid UTF-8 text; treat as binary and skip the content
  // scan (a substring search over binary noise is meaningless and can only
  // manufacture false positives).
  if (text.indexOf('\0') !== -1) return null;
  return text;
}

function scanFile(relPath, tracked = true) {
  const normalized = relPath.split(path.sep).join('/');
  const text = readScannableText(relPath);
  if (text === null) return [];

  const allowedLines = ALLOWLIST.has(normalized) ? computeAllowedLines(text) : null;

  return findMarkerHits(text)
    .filter((h) => !(allowedLines && allowedLines.has(h.line)))
    .map((h) => ({ file: normalized, tracked, ...h }));
}

// ---------------------------------------------------------------------------
// Debt lane: decompile-tree anchors written into the product layer.
//
// Reported as its own count and NEVER folded into the violation count. That
// separation is the whole point: a single merged number is non-zero for every
// lane that touches this repo, so it stops being actionable and teaches
// people to re-classify their hit instead of removing it. Two columns keep
// "must be 0" reachable for the hard markers while this backlog is still
// being worked down.
//
// What belongs here rather than in ALL_MARKERS: an anchor like a decompiled
// filename plus a line number is not a leak of this machine or of a tree
// codename — a reader with only this repository cannot resolve it. It is an
// unfinished job (the coordinate belongs in the research notes, and the
// product layer should carry the argument in its own words), so it is
// counted, named, and left to the person who owns that file.
//
// The gate's own source is skipped: these patterns must appear here
// literally, and a scanner that obfuscates its own rules just moves the leak.
const DEBT_PATTERNS = [
  // boundary:allow-begin
  { name: 'decompiled-file-and-line', re: /\b[A-Za-z0-9_]+\.cs?:\d+/g },
  { name: 'native-binary-name', re: /\blibil2cpp\b/gi },
  // An internal order or scope-ruling code names a document a reader with only
  // this repository cannot open. The rule is to write what the ruling said, not
  // what it is called -- the reasoning is almost always already in the sentence,
  // so dropping the code loses nothing.
  //
  // The pattern deliberately over-matches: a font weight, a padding value and
  // a hex fragment share this shape. Over-matching is safe for a counted lane --
  // the count is a ceiling that shrinks as real references are rewritten, and
  // whatever remains can then be exempted one at a time, each with a reason.
  //
  // Two false-positive shapes are fenced off narrowly: a hex group inside a
  // 0x literal (a UUID cut into underscores) and a typeface weight suffix
  // (Name-B03) are not codes. The obvious fence -- refusing any preceding '-'
  // or '_' -- would also blind the gate to a code glued into a snake_case
  // identifier, which is a real leak shape the positive controls pin. So: a
  // '-' counts as part of a hyphenated name only when a letter precedes it,
  // and a '_' is hex-glue only inside a 0x literal. Everything else still
  // matches -- and the plain "not glued to a word character" fence stays,
  // because without it every checksum fragment in Cargo.lock lights up.
  { name: 'internal-order-or-ruling-code',
    re: /(?<![A-Za-z0-9])(?<![A-Za-z-]-)(?<!0x[0-9A-Fa-f_]{5,})(?:[ODB]-?\d{2,3})(?![A-Za-z0-9])/gi },
  // boundary:allow-end
];

function scanDebt(relPath) {
  const normalized = relPath.split(path.sep).join('/');
  if (normalized === SELF_REL_PATH) return [];
  const text = readScannableText(relPath);
  if (text === null) return [];

  const out = [];
  text.split(/\r?\n/).forEach((lineText, i) => {
    for (const { name, re } of DEBT_PATTERNS) {
      re.lastIndex = 0;
      for (const m of lineText.matchAll(re)) {
        out.push({ file: normalized, line: i + 1, pattern: name, matched: m[0] });
      }
    }
  });
  return out;
}

// ---------------------------------------------------------------------------
// Self-check: prove the matcher is alive before trusting a "0 violations"
// result from the real scan below.
//
// This gate exists to catch a class of failure that hit five different
// tools the same day this self-check was added: a matcher that silently
// stops matching one written form of a pattern (e.g. a backslash-form path
// folded to a forward-slash form by some upstream layer) keeps producing a
// count — just a low, complete-looking one — instead of an error. A "0
// violations" result is only evidence of a clean repo if the matcher that
// produced it can be shown, right before the real scan, to still recognize
// every marker it is supposed to catch.
//
// Both corpora below are run through findMarkerHits() — the exact function
// the real scan uses — so this exercises the real code path, not a
// description of it.
// ---------------------------------------------------------------------------
function buildPositiveControlCorpus() {
  // One line per marker, built from ALL_MARKERS itself so this corpus can
  // never drift out of sync with what the real scan actually looks for, and
  // so the two markers assembled from split literals (the inherited-domain
  // ones) get reconstructed the exact same way the real matcher sees them —
  // never spelled as a fresh contiguous cleartext domain in this file's own
  // source text.
  return ALL_MARKERS.map(({ marker }, i) => `synthetic self-check line ${i}: ${marker}`).join('\n');
}

const NEGATIVE_CONTROL_CORPUS = [
  'this is an ordinary line of prose with no marker in it.',
  'export function resolve(rel) { return path.join(rootDir, rel, "src"); }',
  'relative paths like ./src/main.ts or ../lib/util.ts are unremarkable.',
].join('\n');

function runSelfCheck() {
  const positiveHits = findMarkerHits(buildPositiveControlCorpus());
  const hitMarkers = new Set(positiveHits.map((h) => h.marker));
  const missed = ALL_MARKERS.filter(({ marker }) => !hitMarkers.has(marker));

  if (missed.length > 0) {
    console.error(
      '[boundary] FATAL: matcher self-check failed. The following marker(s) did ' +
      'not fire against a synthetic corpus built to contain them literally — ' +
      'the matcher is broken, and the real scan below (even a "0 violations" ' +
      'result) cannot be trusted:',
    );
    for (const { marker, category } of missed) {
      console.error(`  category=${category}  marker=${JSON.stringify(marker)}`);
    }
    return false;
  }
  const categoryCount = new Set(ALL_MARKERS.map((m) => m.category)).size;
  console.log(
    `[boundary] self-check OK — positive control: all ${ALL_MARKERS.length} ` +
    `marker(s) across ${categoryCount} categor(y/ies) fired`,
  );

  const negativeHits = findMarkerHits(NEGATIVE_CONTROL_CORPUS);
  if (negativeHits.length !== 0) {
    console.error(
      `[boundary] FATAL: matcher self-check failed. A clean synthetic corpus ` +
      `produced ${negativeHits.length} hit(s) — the matcher is over-triggering, ` +
      `so a "violations found" result cannot be trusted either:`,
    );
    for (const h of negativeHits) {
      console.error(`  matched ${JSON.stringify(h.marker)} [${h.category}] on synthetic line ${h.line}`);
    }
    return false;
  }
  console.log('[boundary] self-check OK — negative control: 0 hits on clean synthetic corpus');

  // Same two controls for the debt lane. Its count is only evidence if the
  // patterns that produced it can be shown, right here, to still fire on
  // text built to contain them and to stay quiet on text built to look like
  // them. Without this, "debt: 0" and "the patterns silently stopped
  // matching" are the same output.
  // boundary:allow-begin
  const DEBT_POSITIVE = [
    'a decompiled body cited as SomeClass.c:149 in a comment',
    'a declaration dump cited as dump.cs:535386 in a doc line',
    'the native binary named as libil2cpp.so in prose',
    'a comment citing an internal order as O15 instead of what it decided',
    'a doc line citing a scope ruling as D-104 without saying what it said',
    'a test named axis_census_matches_the_o37_hand_measurement',
    'a lowercase code as in a comment citing d288 for the decision',
    'a bare three-digit code with no dash, as in B154 named in a doc line',
    'a note referring to D287 instead of saying what D287 established',
  ];
  const DEBT_NEGATIVE = [
    'an ordinary host and port like https://example.com:443/path',
    'an ordinary source reference like main.rs:12 or index.ts:44',
    'a windows drive letter in C:/Users style prose',
    'an identifier whose letter is not on a boundary, like IO2 or NO3',
    'a flag spelled O_RDONLY, or a bare OK -- letter present, digit absent',
    'a date like 2026-09-04 and a semver like 1.2.3 name no ruling',
    'identifiers whose digits touch more letters, like o1x or vecO3d',
  ];
  // boundary:allow-end
  const debtHit = (line) =>
    DEBT_PATTERNS.some(({ re }) => {
      re.lastIndex = 0;
      return re.test(line);
    });

  const debtMissed = DEBT_POSITIVE.filter((line) => !debtHit(line));
  if (debtMissed.length > 0) {
    console.error(
      '[boundary] FATAL: debt-lane self-check failed. These synthetic lines were ' +
      'built to contain a decompile-tree anchor and did not match, so the debt ' +
      'count below (including a count of 0) means nothing:',
    );
    for (const line of debtMissed) console.error(`  ${JSON.stringify(line)}`);
    return false;
  }
  const debtFalsePositives = DEBT_NEGATIVE.filter((line) => debtHit(line));
  if (debtFalsePositives.length > 0) {
    console.error(
      '[boundary] FATAL: debt-lane self-check failed. These synthetic lines carry ' +
      'no decompile-tree anchor and matched anyway, so the debt count is inflated ' +
      'and cannot be used to attribute anything:',
    );
    for (const line of debtFalsePositives) console.error(`  ${JSON.stringify(line)}`);
    return false;
  }
  console.log(
    `[boundary] self-check OK — debt lane: ${DEBT_POSITIVE.length} positive control(s) fired, ` +
    `${DEBT_NEGATIVE.length} negative control(s) stayed quiet`,
  );
  return true;
}

// Corpus floor: below this, the tracked-file list itself is suspect (wrong
// cwd, git not on PATH resolving to a different/empty repo, some future
// change accidentally narrowing gitTrackedFiles()). Not a precise count —
// the tree grows — just a floor that catches a collapsed search space that
// would otherwise look like "an unusually clean scan".
// 2026-09-06: floor lowered 150 -> 15 for this repo's bootstrap period. This
// is a young tree (skeleton commit had 15 tracked files); the floor exists to
// catch a scan that quietly collapsed (wrong cwd / wrong repo), not to gate
// repo age. Raise back to 150 once the tree carries the first migrated law
// families and their fixtures.
// ---------------------------------------------------------------------------
// Metadata lane: the two things a clone carries that are not file content.
//
// `git log` messages and the ref list ship with the repository. The markers
// above occur in both -- an internal code in a subject, a branch named
// after an internal ruling -- and nothing here read either, so both stayed
// quiet by construction
// by construction rather than by being clean.
//
// Counted, never fatal: a message cannot be fixed without rewriting history,
// and a gate that fails on what no one can fix today gets turned off. The
// number is what matters -- it lets a rewrite be decided before publishing
// instead of discovered after.
// ---------------------------------------------------------------------------
function gitCommitMessages() {
  const out = execFileSync('git', ['log', '--all', '--format=%H%x1f%B%x1e'],
    { cwd: REPO_ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  const entries = out.split('\x1e').map((entry) => entry.trim()).filter(Boolean)
    .map((entry) => {
      const [sha, body] = entry.split('\x1f');
      return { sha: (sha || '').trim().slice(0, 8), body: body || '' };
    });
  // An empty result -- wrong cwd, a git that failed silently, a detached
  // unborn HEAD -- would read as '0 of 0, clean', which is the exact
  // failure shape this gate exists to refuse. This repo has hundreds of
  // commits; zero means the plumbing lied, not that the history is clean.
  if (entries.length === 0) {
    throw new Error('boundary gate: git log returned no commits -- ' +
      'the metadata lane cannot run');
  }
  return entries;
}

function gitRefNames() {
  const out = execFileSync('git', ['for-each-ref', '--format=%(refname:short)'],
    { cwd: REPO_ROOT, encoding: 'utf8' });
  const refs = out.split('\n').map((l) => l.trim()).filter(Boolean);
  // Same refusal as above: a repo with no refs at all cannot be this repo.
  if (refs.length === 0) {
    throw new Error('boundary gate: for-each-ref returned nothing -- ' +
      'the metadata lane cannot run');
  }
  return refs;
}

// Both matchers, over one string. Reuses findMarkerHits() and DEBT_PATTERNS
// so the metadata lane can never drift from what the file lane looks for.
function scanText(text) {
  const found = findMarkerHits(text).map((h) => `${h.category}:${h.marker}`);
  for (const { name, re } of DEBT_PATTERNS) {
    re.lastIndex = 0;
    const m = text.match(re);
    if (m) found.push(`${name}:${m[0]}`);
  }
  return found;
}

function reportMetadata() {
  const commits = gitCommitMessages();
  const refs = gitRefNames();
  const badCommits = commits
    .map((c) => ({ ...c, found: scanText(c.body) }))
    .filter((c) => c.found.length > 0);
  const badRefs = refs
    .map((name) => ({ name, found: scanText(name) }))
    .filter((r) => r.found.length > 0);
  console.log(
    `[boundary] metadata (counted, not a violation): ${badCommits.length} of ` +
    `${commits.length} commit message(s) and ${badRefs.length} of ${refs.length} ` +
    'ref name(s) carry a marker. These ship with a clone; only a history ' +
    'rewrite or a rename removes them.',
  );
  for (const c of badCommits.slice(0, 12)) {
    const subject = c.body.split('\n')[0].slice(0, 54);
    console.log(`[boundary]   message ${c.sha} ${JSON.stringify(subject)} ` +
      `[${[...new Set(c.found)].join(', ')}]`);
  }
  if (badCommits.length > 12) {
    console.log(`[boundary]   ... and ${badCommits.length - 12} more message(s)`);
  }
  for (const r of badRefs) {
    console.log(`[boundary]   ref ${r.name}  [${[...new Set(r.found)].join(', ')}]`);
  }
}

const MIN_TRACKED_FILES = 15;

function main() {
  if (!runSelfCheck()) {
    process.exitCode = 2;
    return;
  }

  const files = gitTrackedFiles();
  if (files.length < MIN_TRACKED_FILES) {
    console.error(
      `[boundary] FATAL: \`git ls-files\` returned only ${files.length} tracked ` +
      `file(s), below the floor of ${MIN_TRACKED_FILES}. Refusing to report a ` +
      'scan against a possibly-collapsed search space — that is not evidence ' +
      'of a clean repo, it may be evidence the gate did not run against the ' +
      'full tree (wrong cwd, git not on PATH, git resolving to the wrong repo, ' +
      'or gitTrackedFiles() itself regressed). If the tree has genuinely shrunk ' +
      'below this floor, lower MIN_TRACKED_FILES with a reason, don\'t delete ' +
      'the check.',
    );
    process.exitCode = 2;
    return;
  }

  const untracked = gitUntrackedFiles();
  const untrackedSource = untracked.filter(isReleaseCandidateSource);
  const notScannedCount = untracked.length - untrackedSource.length;

  const coverage = checkSourceDirCoverage(files, untracked);
  if (coverage.checked.length < MIN_SOURCE_DIRS) {
    console.error(
      `[boundary] FATAL: the source-directory coverage check asserted on only ` +
      `${coverage.checked.length} director(y/ies), below the floor of ${MIN_SOURCE_DIRS}. ` +
      'An enumeration that collapses to (almost) nothing reports "every ' +
      'directory is covered" without having looked at any, which is ' +
      'indistinguishable from a real pass. Fix the enumeration; do not lower ' +
      'the floor to make this line go away.',
    );
    process.exitCode = 2;
    return;
  }

  const hits = [
    ...files.flatMap((rel) => scanFile(rel, true)),
    ...untrackedSource.flatMap((rel) => scanFile(rel, false)),
  ];

  // REPO_ROOT itself is this machine's own absolute path to the repo — the
  // exact kind of value this gate exists to keep out of tracked output.
  // relToRepo(REPO_ROOT, REPO_ROOT) would just be the empty string (a path
  // is always "" relative to itself), which is not useful in a log line, so
  // this prints the repo directory's own name instead of relativizing it.
  console.log(`[boundary] scanned ${files.length} git-tracked file(s) under repo root "${path.basename(REPO_ROOT)}" (floor: ${MIN_TRACKED_FILES})`);
  console.log(`[boundary] scanned ${untrackedSource.length} untracked source file(s) under ${SOURCE_DIR_DESCRIPTION} — release candidates: not published yet, published the moment they are added`);
  console.log(
    `[boundary] source-dir coverage: ${coverage.checked.length} director(y/ies) holding source ` +
    `asserted against the prefix list (floor: ${MIN_SOURCE_DIRS}), ` +
    `${coverage.excluded.length} excluded by rule, ${coverage.uncovered.length} uncovered`,
  );
  for (const e of coverage.excluded) {
    console.log(`[boundary]   excluded ${displayDir(e.dir)} — ${e.reason}`);
  }

  // The blind spot, stated as a number on every run. Without this line a
  // "0 violations" result reads as "the tree is clean", which it is not:
  // it is "everything this gate looked at is clean". Anything counted here
  // was not looked at.
  console.log(`[boundary] NOT scanned: ${notScannedCount} of ${untracked.length} untracked file(s) (the non-source ones). "0 violations" below covers the ${files.length + untrackedSource.length} file(s) scanned above, not the whole working tree.`);

  // Debt lane, reported before the verdict and separately from it. This
  // number never changes the exit code -- see DEBT_PATTERNS for why these two
  // columns are kept apart.
  const debt = [
    ...files.flatMap((rel) => scanDebt(rel)),
    ...untrackedSource.flatMap((rel) => scanDebt(rel)),
  ];
  const debtFiles = new Set(debt.map((d) => d.file));
  console.log(
    `[boundary] debt (counted, not a violation): ${debt.length} out-of-repo ` +
    `reference(s) in ${debtFiles.size} file(s). New ones are forbidden; existing ` +
    `ones are an unfinished job, owned by whoever owns the file.`,
  );
  for (const d of debt) {
    console.log(`[boundary]   debt ${d.file}:${d.line}  [${d.pattern}]  matched: ${JSON.stringify(d.matched)}`);
  }

  reportMetadata();

  const violationCount = hits.length + coverage.uncovered.length;
  if (violationCount === 0) {
    console.log('[boundary] OK - 0 violations');
    return;
  }

  console.log(`[boundary] FAIL - ${violationCount} violation(s):`);
  for (const u of coverage.uncovered) {
    const origin = u.tracked ? 'tracked' : 'untracked';
    console.log(
      `  ${displayDir(u.dir)}  [source-dir-not-covered]  holds source (e.g. ${u.example}, ${origin}) ` +
      'but matches no entry in the source-directory list — untracked source there is not treated ' +
      'as a release candidate and never gets scanned. Add a prefix covering it, or an ' +
      'EXCLUDED_DIR_PATTERNS entry with a reason.',
    );
  }
  for (const h of hits) {
    const origin = h.tracked ? 'tracked' : 'untracked, release candidate';
    console.log(`  ${h.file}:${h.line}  [${h.category}]  (${origin})  matched: ${JSON.stringify(h.marker)}`);
  }
  if (hits.some((h) => !h.tracked)) {
    console.log(
      '[boundary] note: violation(s) above are in untracked files. They are ' +
      'not published yet — fix them before adding, or the add publishes them.',
    );
  }
  process.exitCode = 1;
}

main();
