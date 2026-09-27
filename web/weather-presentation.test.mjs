import assert from "node:assert/strict";
import test from "node:test";
import { presentWeather, sourceWeatherIcon } from "./weather-presentation.mjs";
const baseUrl = "https://example.test/moly/stage.html";
const assets = "/source/jp/revision/";

test("source-relative icon path is independent of weather labels", () => {
  const options = [
    {
      id: 4321,
      name: "new_source_name",
      icon: "icons/master_key.png",
      metadata: { name: "新しい空" },
    },
  ];
  const result = presentWeather(
    { id: 4321, name: options[0].name, options },
    { assets, baseUrl, locale: "ja-JP", region: "jp" },
  );
  assert.equal(
    result.iconUrl,
    null,
    "flat icons also wait for byte verification",
  );
  assert.equal(
    sourceWeatherIcon(options[0].icon, assets, baseUrl),
    "https://example.test/source/jp/revision/phenomena/icons/master_key.png",
  );
  assert.equal(result.label, "新しい空");
  assert.equal(
    options[0].iconUrl,
    undefined,
    "source snapshot was not mutated",
  );
});

test("missing art is not substituted by a name-derived icon", () => {
  const result = presentWeather(
    {
      id: 8,
      name: "thunder",
      options: [{ id: 8, name: "thunder", icon: null }],
    },
    { assets, baseUrl, locale: "en-US", region: "jp" },
  );
  assert.equal(result.iconUrl, null);
  assert.equal(result.options[0].iconUrl, null);
});

test("source art cannot escape its same-origin resource directory", () => {
  for (const file of [
    null,
    "",
    "../x",
    "/x",
    "a//b",
    "icons/%2e%2e/x",
    "icons/%2fsecret",
    "icons/%5csecret",
    "https://other.test/x",
    "x?y",
    "x#y",
    "icons/%00.png",
    "bad%zz",
  ]) {
    assert.equal(sourceWeatherIcon(file, assets, baseUrl), null, String(file));
  }
  for (const root of [
    "https://other.test/assets/",
    "/assets",
    "/assets/?q=1",
    "/assets/#x",
  ]) {
    assert.equal(sourceWeatherIcon("icons/x.png", root, baseUrl), null, root);
  }
});

test("future display names fall back to master data instead of editor slugs", () => {
  const result = presentWeather(
    {
      id: 4321,
      name: "4321_unknown",
      options: [
        { id: 4321, name: "4321_unknown", metadata: { name: "Source label" } },
      ],
    },
    { assets, baseUrl, locale: "en-US", region: "jp" },
  );
  assert.equal(result.label, "Source label");
});

test("a requested environment does not impersonate committed visible weather", () => {
  const options = [
    { id: 1, name: "sunny", icon: null },
    { id: 8, name: "thunder", icon: null },
  ];
  const weather = {
    id: 8,
    name: "thunder",
    options,
    transition: { phase: "loading", requestedId: 8, committedId: 1 },
  };
  const result = presentWeather(weather, {
    assets,
    baseUrl,
    locale: "en-US",
    region: "jp",
  });
  assert.equal(result.committedId, 1);
  assert.notEqual(result.label, result.requestedLabel);
  assert.equal(result.transition.phase, "loading");
  assert.equal(result.id, 8, "source request identity is retained for hosts");
});
