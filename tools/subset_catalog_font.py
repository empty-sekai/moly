"""Rebuild the licensed UI font subset from actual catalog text.

Requires fonttools. The caller supplies the original OFL font. Existing code
points are retained; no source font is downloaded and no runtime asset is
replaced by this tool. Review the generated subset and coverage report before
installing it. Example:
  python tools/subset_catalog_font.py --source ORIGINAL.ttf \
    --existing crates/moly-game/assets/font/ResourceHanRoundedSC-Medium.subset.ttf \
    --assets PATH_TO_EXTRACTED_ASSETS --output candidate.ttf
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import unicodedata
from typing import Iterator

from fontTools import subset
from fontTools.ttLib import TTFont


CATALOGS = (
    "characters.json",
    "mysekai-fixtures.json",
    "talks.json",
    "fixture-talks/out/fixture-talks.json",
)
UI_TEXT = "对话与互动 独立体验 内容 对话 家具 演出 搜索 筛选 角色 全部 当前 场景 空场景 所需 可体验 播放 停止 返回 关闭 上一页 下一页 暂不可用 正在准备 已结束 名称 台词 清空 详情 实例 继续 查看 全文 仅看 收藏 最近 使用 中 缺少 资源 重试 缩略图 没有找到 确定 取消 下一句 · … ← → ↑ ↓ × − + / # 0123456789"


def strings(value: object) -> Iterator[str]:
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)


def codepoints(text: str) -> set[int]:
    return {ord(ch) for ch in text if not unicodedata.category(ch).startswith("C")}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--existing", required=True, type=Path)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--ui-source", action="append", default=[], type=Path)
    args = parser.parse_args()
    if args.output.resolve() in {args.source.resolve(), args.existing.resolve()}:
        parser.error("Output must be a separate candidate, never either input font.")
    if args.output.exists():
        parser.error("Output already exists; inspect it rather than overwrite prior evidence.")

    with TTFont(args.existing) as previous:
        old_points = set(previous.getBestCmap())
    font = TTFont(args.source, recalcTimestamp=False)
    source_points = set(font.getBestCmap())
    if not old_points <= source_points:
        raise ValueError("The supplied original font cannot preserve the existing subset.")
    requested = old_points | codepoints(UI_TEXT)
    inputs = []
    for name in CATALOGS:
        path = args.assets / name
        raw = path.read_bytes()
        value = json.loads(raw)
        for text in strings(value):
            requested.update(codepoints(text))
        inputs.append({"path": name, "sha256": hashlib.sha256(raw).hexdigest()})
    for path in args.ui_source:
        requested.update(codepoints(path.read_text(encoding="utf8")))
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
    if not supported <= actual or not old_points <= actual:
        raise RuntimeError("Subset coverage verification failed.")
    report = {
        "sourceSha256": hashlib.sha256(args.source.read_bytes()).hexdigest(),
        "previousGlyphs": len(old_points),
        "requestedCodepoints": len(requested),
        "coveredCodepoints": len(actual),
        "addedCodepoints": len(actual - old_points),
        "unsupportedCodepoints": [f"U+{point:04X}" for point in missing],
        "unsupportedCharacters": "".join(chr(point) for point in missing),
        "existingCoveragePreserved": True,
        "inputs": inputs,
        "outputBytes": args.output.stat().st_size,
        "outputSha256": hashlib.sha256(args.output.read_bytes()).hexdigest(),
    }
    args.output.with_suffix(".coverage.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf8"
    )
    print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
