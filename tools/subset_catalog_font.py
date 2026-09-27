"""Rebuild the licensed UI font subset from a census of the text the product draws.

Requires fonttools. Three steps, each reproducible from its recorded inputs:

  masters  fetch the upstream master tables the census reads, and the region's
           current_version.json, into OUT/<region>/
  census   collect the code points of every text a product root and its
           region's master tables can put on screen into a census file
  build    subset the original OFL font to the existing subset's code points,
           the census files and the product's own literal texts

Control characters (Unicode category C) never enter a census: the product's
atlas asks for no glyph for them. Fetched tables and census files hold game
text; keep them outside the repository. The build refuses to write when a
retained glyph's outline or metrics differ from the original font, and never
overwrites its inputs or an existing candidate. Review the candidate and its
coverage report before installing it. Example:
  python tools/subset_catalog_font.py masters --region jp --output MASTERS
  python tools/subset_catalog_font.py census --root ROOT_JP --region jp \
    --masters MASTERS/jp --output census-jp.json
  python tools/subset_catalog_font.py build --source ORIGINAL.ttf \
    --existing crates/moly-game/assets/font/ResourceHanRoundedSC-Medium.subset.ttf \
    --census census-jp.json --census census-cn.json --output candidate.ttf
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import unicodedata
import urllib.request
from typing import Iterator

from fontTools import subset
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.ttLib import TTFont


REPO = Path(__file__).resolve().parents[1]
GAME_SRC = REPO / "crates" / "moly-game" / "src"

MASTER_BASE = "https://metadata.pjsk.moe/"
# The host refuses Python's default user agent.
USER_AGENT = "curl/8.5.0"

# Master tables whose text the UI draws, with the fields holding that text
# ("a[].b" reads field b of every element of list a). Readings used only for
# sorting (pronunciation) are not drawn. Blueprints and music records carry no
# text of their own: they print fixture names and music titles.
MASTER_FIELDS: dict[str, tuple[str, ...]] = {
    "gameCharacters": ("firstName", "givenName", "firstNameRuby", "givenNameRuby"),
    "outsideCharacters": ("name",),
    "unitProfiles": ("unitName", "unitProfileName"),
    "musics": (
        "title", "lyricist", "composer", "arranger",
        "infos[].title", "infos[].creator", "infos[].lyricist", "infos[].composer",
        "infos[].arranger",
    ),
    "musicArtists": ("name",),
    "musicVocals": ("caption",),
    "honors": ("name", "levels[].description"),
    "honorGroups": ("name",),
    "bondsHonors": ("name", "description", "levels[].description"),
    "bondsHonorWords": ("name", "description"),
    "mysekaiMaterials": ("name", "description"),
    "mysekaiItems": ("name", "description"),
    "mysekaiFixtures": ("name", "flavorText"),
    "mysekaiFixtureMainGenres": ("name",),
    "mysekaiFixtureSubGenres": ("name",),
    "mysekaiFixtureTags": ("name",),
    "mysekaiTools": ("name", "description"),
    "mysekaiMusicRecords": (),
    "mysekaiMusicRecordCategories": ("name",),
    "mysekaiBlueprints": (),
    "mysekaiSites": ("name",),
    "mysekaiGates": ("name",),
    "mysekaiPhenomenas": ("name", "description"),
    "wordings": ("value",),
}

# Root documents whose strings are drawn: talk and tweet corpora, wordings and
# the root's master copies. Every string counts except the subtrees below.
ROOT_TEXT_DOCUMENTS = (
    "talks.json",
    "tweets.json",
    "tweet-tables.json",
    "fixture-talks/out/fixture-talks.json",
    "wordings.json",
    "characters.json",
    "materials.json",
    "mysekai-fixtures.json",
    "mysekai-items.json",
    "mysekai-materials.json",
    "mysekai-tools.json",
    "site/sites.json",
    "phenomena/index.json",
)
# The extractor's own notes, sort readings and lookup keys: never drawn.
EXCLUDED_KEYS = frozenset({"semantics", "pronunciation", "wordingKey", "rowOrder"})

# UI roots: the region's own root when it has one, and the shared root.
UI_ROOTS = ("ui-{region}", "ui-layout-v2")
TEXT_COMPONENT = "Sekai.UI.CustomTextMesh"

# Text the product writes itself beyond the constants read from its sources.
UI_TEXT = "对话与互动 独立体验 内容 对话 家具 演出 搜索 筛选 角色 全部 当前 场景 空场景 所需 可体验 播放 停止 返回 关闭 上一页 下一页 暂不可用 正在准备 已结束 名称 台词 清空 详情 实例 继续 查看 全文 仅看 收藏 最近 使用 中 缺少 资源 重试 缩略图 没有找到 确定 取消 下一句 · … ← → ↑ ↓ × − + / # 0123456789"


def printable(ch: str) -> bool:
    return not unicodedata.category(ch).startswith("C")


class Census:
    def __init__(self) -> None:
        self.points: set[int] = set()
        self.excluded: dict[str, int] = {}

    def add(self, text: str) -> None:
        for ch in text:
            if printable(ch):
                self.points.add(ord(ch))
            elif ch != "\n":
                key = f"U+{ord(ch):04X}"
                self.excluded[key] = self.excluded.get(key, 0) + 1


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def strings(value: object) -> Iterator[str]:
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for key, item in value.items():
            if key not in EXCLUDED_KEYS:
                yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


def field_values(row: object, field: str) -> Iterator[str]:
    head, _, rest = field.partition(".")
    if not isinstance(row, dict):
        return
    if head.endswith("[]"):
        items = row.get(head[:-2])
        for item in items if isinstance(items, list) else ():
            yield from field_values(item, rest)
    elif rest:
        yield from field_values(row.get(head), rest)
    elif isinstance(row.get(head), str):
        yield row[head]


def text_fields(row: object, prefix: str = "") -> Iterator[tuple[str, str]]:
    """Every (field path, string) of a row, in `field_values` notation."""
    if isinstance(row, dict):
        for key, value in row.items():
            path = f"{prefix}{key}"
            if isinstance(value, str):
                yield path, value
            elif isinstance(value, list):
                for item in value:
                    yield from text_fields(item, f"{path}[].")
            elif isinstance(value, dict):
                yield from text_fields(value, f"{path}.")


TMP_ESCAPE = re.compile(r"\\u([0-9A-Fa-f]{4})|\\U([0-9A-Fa-f]{8})")


def layout_texts(document: dict) -> Iterator[str]:
    """The serialized text of every TMP text component, and the characters
    its escapes spell (the product lays text out after TMP's escape pass)."""
    for node in document.get("nodes", []):
        for component in node.get("components", []):
            if component.get("class") != TEXT_COMPONENT:
                continue
            text = component.get("fields", {}).get("m_text")
            if not isinstance(text, str):
                continue
            yield text
            for short, long in TMP_ESCAPE.findall(text):
                point = int(short or long, 16)
                if point <= 0x10FFFF:
                    yield chr(point)
            if "<nbsp>" in text.lower():
                yield "\u00a0"


def fetch_masters(args: argparse.Namespace) -> None:
    out = args.output / args.region
    out.mkdir(parents=True, exist_ok=True)
    names = ["versions/current_version"] + [f"master/{table}" for table in MASTER_FIELDS]
    for name in names:
        request = urllib.request.Request(
            f"{MASTER_BASE}{args.region}/{name}.json", headers={"User-Agent": USER_AGENT}
        )
        with urllib.request.urlopen(request, timeout=120) as response:
            body = response.read()
        json.loads(body)
        (out / f"{name.rsplit('/', 1)[1]}.json").write_bytes(body)
    version = json.loads((out / "current_version.json").read_bytes())
    print(json.dumps({"region": args.region, "dataVersion": version.get("dataVersion"),
                      "tables": len(MASTER_FIELDS)}))


def census(args: argparse.Namespace) -> None:
    root, region = args.root, args.region
    if args.output.exists():
        raise SystemExit("Output already exists; inspect it rather than overwrite prior evidence.")
    identity = root / "source.json"
    if identity.exists():
        declared = json.loads(identity.read_bytes()).get("source", {}).get("region")
        if declared != region:
            raise SystemExit(f"{root} is a {declared} root, not {region}.")
    total = Census()
    inputs: list[dict] = []
    absent: list[str] = []

    def record(kind: str, path: Path, texts: Iterator[str]) -> None:
        part = Census()
        count = 0
        for text in texts:
            part.add(text)
            count += 1
        total.points |= part.points
        for key, n in part.excluded.items():
            total.excluded[key] = total.excluded.get(key, 0) + n
        inputs.append({
            "kind": kind, "path": path.relative_to(root).as_posix(), "sha256": sha256(path),
            "strings": count, "codepoints": len(part.points),
            "excludedControl": part.excluded,
        })

    for name in ROOT_TEXT_DOCUMENTS:
        path = root / name
        if path.exists():
            record("document", path, strings(json.loads(path.read_bytes())))
        else:
            absent.append(name)
    layouts = 0
    for pattern in UI_ROOTS:
        ui = root / pattern.format(region=region)
        if not ui.is_dir():
            absent.append(ui.name + "/")
            continue
        for path in sorted(ui.rglob("*.json")):
            document = json.loads(path.read_bytes())
            if path.name == "wordings.json":
                record("wordings", path, strings(document))
            elif isinstance(document, dict) and isinstance(document.get("nodes"), list):
                layouts += 1
                record("layout", path, layout_texts(document))
    if layouts == 0:
        raise SystemExit(f"{root} holds no UI layout document.")

    masters = args.masters
    version = json.loads((masters / "current_version.json").read_bytes())
    tables = []
    for table, fields in MASTER_FIELDS.items():
        path = masters / f"{table}.json"
        rows = json.loads(path.read_bytes())
        if not isinstance(rows, list):
            raise SystemExit(f"{path} is not a master table.")
        drawn: dict[str, int] = {field: 0 for field in fields}
        unlisted: dict[str, int] = {}
        part = Census()
        for row in rows:
            for field in fields:
                for text in field_values(row, field):
                    part.add(text)
                    drawn[field] += 1
            for field, text in text_fields(row):
                if field not in drawn and any(ord(ch) > 0x7E for ch in text):
                    unlisted[field] = unlisted.get(field, 0) + 1
        total.points |= part.points
        for key, n in part.excluded.items():
            total.excluded[key] = total.excluded.get(key, 0) + n
        tables.append({
            "table": table, "sha256": sha256(path), "rows": len(rows), "fields": drawn,
            "codepoints": len(part.points), "excludedControl": part.excluded,
            "unlistedNonAsciiFields": unlisted,
        })
    report = {
        "version": 1,
        "region": region,
        "root": str(root),
        "codepoints": sorted(total.points),
        "excludedControl": total.excluded,
        "inputs": inputs,
        "absent": absent,
        "masters": {
            "path": str(masters),
            "dataVersion": version.get("dataVersion"),
            "appVersion": version.get("appVersion"),
            "currentVersionSha256": sha256(masters / "current_version.json"),
            "tables": tables,
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=1) + "\n", encoding="utf8")
    print(json.dumps({"region": region, "codepoints": len(total.points), "inputs": len(inputs),
                      "layouts": layouts, "absent": absent, "excludedControl": total.excluded,
                      "dataVersion": version.get("dataVersion")}, ensure_ascii=False))


RUST_STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')


def rust_unescape(literal: str) -> str:
    def replace(match: re.Match) -> str:
        escape = match.group(0)
        if escape.startswith("\\u{"):
            return chr(int(escape[3:-1], 16))
        return {"\\n": "\n", "\\r": "\r", "\\t": "\t", "\\0": "\0"}.get(escape, escape[1:])
    return re.sub(r"\\u\{[0-9A-Fa-f]+\}|\\.", replace, literal)


def product_literals() -> tuple[list[str], list[dict]]:
    """The texts the product's own sources hand to the atlas: every
    `FIXED_TEXTS` constant and the sitemap's phenomenon rows."""
    texts: list[str] = [UI_TEXT]
    sources = []
    block = re.compile(r"const FIXED_TEXTS: &\[&str\] = &\[(.*?)\];", re.S)
    for path in sorted(GAME_SRC.rglob("*.rs")):
        source = path.read_text(encoding="utf8")
        found = []
        for body in block.findall(source):
            code = "\n".join(line.split("//", 1)[0] for line in body.splitlines())
            found += [rust_unescape(s) for s in RUST_STRING.findall(code)]
        if path.name == "sitemap_phenomena.rs":
            found += [rust_unescape(s) for s in re.findall(r'\b(?:jp|en): "((?:[^"\\]|\\.)*)"', source)]
        if found:
            texts += found
            sources.append({"path": path.relative_to(REPO).as_posix(), "strings": len(found)})
    return texts, sources


def glyph_signature(font: TTFont, glyphs, name: str) -> tuple:
    # Components are drawn decomposed: a subset renumbers and may rename them.
    pen = DecomposingRecordingPen(glyphs)
    glyphs[name].draw(pen)
    vertical = tuple(font["vmtx"][name]) if "vmtx" in font else None
    return (tuple(pen.value), tuple(font["hmtx"][name]), vertical)


def build(args: argparse.Namespace) -> None:
    if args.output.resolve() in {args.source.resolve(), args.existing.resolve()}:
        raise SystemExit("Output must be a separate candidate, never either input font.")
    if args.output.exists():
        raise SystemExit("Output already exists; inspect it rather than overwrite prior evidence.")

    previous = TTFont(args.existing)
    old_cmap = previous.getBestCmap()
    old_points = set(old_cmap)
    font = TTFont(args.source, recalcTimestamp=False)
    source_cmap = font.getBestCmap()
    source_points = set(source_cmap)
    if not old_points <= source_points:
        raise SystemExit("The supplied original font cannot preserve the existing subset.")
    # The retained glyphs must be the original's: same outline, same metrics.
    old_glyphs, source_glyphs = previous.getGlyphSet(), font.getGlyphSet()
    differing = [
        point for point in sorted(old_points)
        if glyph_signature(previous, old_glyphs, old_cmap[point])
        != glyph_signature(font, source_glyphs, source_cmap[point])
    ]
    previous.close()
    identity = {"compared": len(old_points), "identical": len(old_points) - len(differing),
                "differing": [f"U+{point:04X}" for point in differing]}
    if differing:
        raise SystemExit(f"Retained glyphs differ from the original font: {json.dumps(identity)}")

    literals = Census()
    texts, literal_sources = product_literals()
    for text in texts:
        literals.add(text)
    for path in args.ui_source:
        literals.add(path.read_text(encoding="utf8"))
    requested = old_points | literals.points
    censuses = []
    for path in args.census:
        document = json.loads(path.read_bytes())
        points = set(document["codepoints"])
        requested |= points
        censuses.append({
            "path": path.name, "sha256": sha256(path), "region": document["region"],
            "dataVersion": document["masters"]["dataVersion"], "codepoints": len(points),
            "newCodepoints": len(points - old_points), "absent": document["absent"],
        })
    missing = sorted(requested - source_points)
    supported = requested & source_points

    options = subset.Options()
    options.recalc_timestamp = False
    options.notdef_glyph = True
    options.notdef_outline = True
    options.recommended_glyphs = True
    options.layout_features = ["*"]
    builder = subset.Subsetter(options=options)
    builder.populate(unicodes=supported)
    builder.subset(font)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    font.save(args.output)
    font.close()
    with TTFont(args.output) as result:
        actual = set(result.getBestCmap())
        glyph_count = result["maxp"].numGlyphs
    if not supported <= actual or not old_points <= actual:
        raise RuntimeError("Subset coverage verification failed.")
    added = sorted(actual - old_points)
    args.output.with_suffix(".added.txt").write_text(
        "".join(f"U+{point:04X}\t{chr(point)}\n" for point in added), encoding="utf8"
    )
    report = {
        "sourceSha256": sha256(args.source),
        "existingSha256": sha256(args.existing),
        "previousCodepoints": len(old_points),
        "retainedGlyphIdentity": identity,
        "requestedCodepoints": len(requested),
        "coveredCodepoints": len(actual),
        "addedCodepoints": len(added),
        "glyphs": glyph_count,
        "unsupportedCodepoints": [f"U+{point:04X}" for point in missing],
        "censuses": censuses,
        "productLiterals": literal_sources,
        "existingBytes": args.existing.stat().st_size,
        "outputBytes": args.output.stat().st_size,
        "outputSha256": sha256(args.output),
    }
    args.output.with_suffix(".coverage.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf8"
    )
    print(json.dumps(report, ensure_ascii=False))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    steps = parser.add_subparsers(dest="step", required=True)
    fetch = steps.add_parser("masters", help="fetch the master tables the census reads")
    fetch.add_argument("--region", required=True, choices=("jp", "cn"))
    fetch.add_argument("--output", required=True, type=Path)
    take = steps.add_parser("census", help="collect a root's drawable code points")
    take.add_argument("--root", required=True, type=Path)
    take.add_argument("--region", required=True, choices=("jp", "cn"))
    take.add_argument("--masters", required=True, type=Path)
    take.add_argument("--output", required=True, type=Path)
    make = steps.add_parser("build", help="subset the original font")
    make.add_argument("--source", required=True, type=Path)
    make.add_argument("--existing", required=True, type=Path)
    make.add_argument("--census", required=True, action="append", type=Path)
    make.add_argument("--output", required=True, type=Path)
    make.add_argument("--ui-source", action="append", default=[], type=Path)
    args = parser.parse_args()
    {"masters": fetch_masters, "census": census, "build": build}[args.step](args)


if __name__ == "__main__":
    main()
