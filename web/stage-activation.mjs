/** Forward the host's trusted click synchronously into the same-origin stage,
 * so audio the browser held back can start inside that gesture. */
export function forwardActivation(frame) {
  try {
    const stage = frame.contentWindow;
    if (!stage) return false;
    stage.dispatchEvent(new stage.Event("moly-activate"));
    return true;
  } catch {
    // A frame that failed to load holds an error document of another origin.
    return false;
  }
}
