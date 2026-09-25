import test from "node:test";
import assert from "node:assert/strict";
import { forwardActivation } from "./stage-activation.mjs";
import { audioActivation } from "./stage-audio.mjs";

test("the host click reaches the stage synchronously", () => {
  const seen = [];
  class Event {
    constructor(type) {
      this.type = type;
    }
  }
  const stage = { Event, dispatchEvent: (event) => seen.push(event.type) };
  assert.equal(forwardActivation({ contentWindow: stage }), true);
  assert.deepEqual(seen, ["moly-activate"]);
});

test("a frame without a readable stage document is not activated", () => {
  assert.equal(forwardActivation({ contentWindow: null }), false);
  const foreign = {
    get Event() {
      throw new DOMException("Blocked a frame", "SecurityError");
    },
  };
  assert.equal(forwardActivation({ contentWindow: foreign }), false);
});

test("activation resumes the engine's held-back audio contexts only", async () => {
  const resumed = [];
  class NativeContext {
    constructor(state) {
      this.state = state;
    }
    resume() {
      resumed.push(this);
      this.state = "running";
      return Promise.resolve();
    }
  }
  const scope = { AudioContext: NativeContext };
  const resume = audioActivation(scope);
  resume();
  assert.equal(resumed.length, 0, "no context is created by the stage itself");
  const held = new scope.AudioContext("suspended");
  const playing = new scope.AudioContext("running");
  const closed = new scope.AudioContext("closed");
  assert.ok(held instanceof NativeContext);
  resume();
  assert.deepEqual(resumed, [held]);
  resume();
  assert.deepEqual(resumed, [held], "a running context is left alone");
  assert.equal(playing.state, "running");
  assert.equal(closed.state, "closed");
});

test("a rejected resume stays inside the activation handler", async () => {
  class NativeContext {
    state = "suspended";
    resume() {
      return Promise.reject(new DOMException("not allowed", "NotAllowedError"));
    }
  }
  const scope = { AudioContext: NativeContext };
  const resume = audioActivation(scope);
  new scope.AudioContext();
  assert.doesNotThrow(resume);
  await new Promise((done) => setTimeout(done, 0));
});

test("a scope without Web Audio is left unchanged", () => {
  const scope = {};
  audioActivation(scope)();
  assert.equal("AudioContext" in scope, false);
});
