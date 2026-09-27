import test from "node:test";
import assert from "node:assert/strict";
import { weatherLabel } from "./stage-locale.mjs";

const expected = {
  "zh-CN": ["泡泡", "宇宙", "水下"],
  "zh-TW": ["泡泡", "宇宙", "水下"],
  "en-US": ["Bubbles", "Universe", "Underwater"],
  "ja-JP": ["シャボン玉", "宇宙", "水中"],
  "ko-KR": ["비눗방울", "우주", "수중"],
};
test("JP-only source weather names are localized for stage badge and host snapshots", () => {
  for (const [locale, values] of Object.entries(expected)) {
    assert.deepEqual(
      ["012_soapbubble", "013_universe", "016_underwater"].map((name) =>
        weatherLabel(locale, name),
      ),
      values,
    );
  }
});
test("weather localization preserves known phenomena and future-name fallback", () => {
  assert.equal(weatherLabel("zh-CN", "006_rain"), "雨");
  assert.equal(weatherLabel("ja-JP", "012_soapbubble"), "シャボン玉");
  assert.equal(weatherLabel("unknown-locale", "012_soapbubble"), "Bubbles");
  assert.equal(
    weatherLabel("zh-CN", "099_future-phenomenon"),
    "future-phenomenon",
  );
});
