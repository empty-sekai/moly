/** Forward the host's trusted click synchronously to the existing same-origin
 * bootstrap gate. Never create a second audio context or renderer. */
export function activatePreparedStage(frame) {
  const gate = frame.contentDocument?.getElementById("stage-start");
  if (
    !gate ||
    gate.hidden ||
    gate.disabled ||
    gate.dataset.molyReady !== "true"
  )
    return false;
  gate.click();
  return true;
}
