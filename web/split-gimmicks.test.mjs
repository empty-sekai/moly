import test from "node:test";
import assert from "node:assert/strict";
import { sourcePackages } from "./split-gimmicks.mjs";

test("splitting preserves original whitespace, large signed IDs and escaped strings", () => {
  const raw =
    '{ "name": "a", "id": -9223372036854775808, "string":"brace } and \\\"quote", "curve": [0.10000000000000001, {"v": 3}] }';
  const doc =
    '{"version":1,"sources":[],"packages":[' + raw + '],"summary":{"count":1}}';
  const rows = [...sourcePackages(doc)];
  assert.equal(rows.length, 1);
  assert.equal(rows[0].raw, raw);
  assert.equal(rows[0].name, "a");
});
test("array/object nesting and escaped slashes do not split inside a package", () => {
  const raw = '{"name":"a","value":[{"x":"\\\\"}],"end":[]}';
  const rows = [
    ...sourcePackages('{"version":1,"packages":[' + raw + ',{"name":"b"}]}'),
  ];
  assert.deepEqual(
    rows.map((r) => r.name),
    ["a", "b"],
  );
  assert.equal(rows[0].raw, raw);
});
test("unknown versions, duplicate identities, truncation and trailing commas fail", () => {
  for (const text of [
    '{"version":2,"packages":[]}',
    '{"version":1,"packages":[{"name":"a"},{"name":"a"}]}',
    '{"version":1,"packages":[{"name":"a"}]} {}',
    '{"version":1,"packages":[{"name":"a"},]}',
    '{"version":1,"packages":[{"name":"a"}]} ,',
    '{"version":1,"packages":[{"name":"a"}]}x',
    '{"version":1,"packages":[{"name":"a"}]',
    '{"version":1,"packages":[{}]}',
    '{"version":1,"version":1,"packages":[]}',
    '{"version":1,"packages":[],}',
  ])
    assert.throws(() => [...sourcePackages(text)], text);
  assert.deepEqual([...sourcePackages('{"version":1,"packages":[]}')], []);
});
