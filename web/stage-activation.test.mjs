import test from "node:test";
import assert from "node:assert/strict";
import { activatePreparedStage } from "./stage-activation.mjs";
function frame(gate) {
  return {
    contentDocument: {
      getElementById: (id) => (id === "stage-start" ? gate : null),
    },
  };
}
test("host Play synchronously starts the prepared owner", () => {
  let calls = 0;
  assert.equal(
    activatePreparedStage(
      frame({
        hidden: false,
        disabled: false,
        dataset: { molyReady: "true" },
        click() {
          calls++;
        },
      }),
    ),
    true,
  );
  assert.equal(calls, 1);
});
for (const [name, gate] of [
  ["missing", null],
  ["downloading", { dataset: { molyReady: "false" } }],
  ["disabled", { disabled: true, dataset: { molyReady: "true" } }],
  ["hidden", { hidden: true, dataset: { molyReady: "true" } }],
])
  test(name + " cannot activate a new renderer", () =>
    assert.equal(activatePreparedStage(frame(gate)), false),
  );
