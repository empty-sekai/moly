/** The stage starts its engine as soon as preparation completes, outside any
 * click. A browser that has not yet allowed this document to play audio creates
 * the engine's output context suspended; each later user activation (in the
 * stage, or forwarded synchronously by the host) resumes it. Only contexts the
 * engine itself creates are tracked; none is created here. */
export function audioActivation(scope) {
  const contexts = new Set();
  const Native = scope.AudioContext;
  if (typeof Native === "function")
    scope.AudioContext = class AudioContext extends Native {
      constructor(...args) {
        super(...args);
        contexts.add(this);
      }
    };
  return function resume() {
    for (const context of contexts) {
      if (context.state === "closed") contexts.delete(context);
      else if (context.state !== "running") context.resume().catch(() => {});
    }
  };
}
