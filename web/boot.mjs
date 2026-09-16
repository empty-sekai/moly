// Preflight finishes before the trusted click that creates the audio context.
export async function selectRenderer(
  requested,
  { navigator, document },
  timeoutMs = 10_000,
) {
  if (!["auto", "webgpu", "webgl2"].includes(requested)) {
    throw new Error("renderer must be auto, webgpu or webgl2");
  }
  let gpuFailure = "WebGPU API is unavailable";
  if (requested !== "webgl2") {
    let timer;
    try {
      if (!navigator.gpu?.requestAdapter) throw new Error(gpuFailure);
      const adapter = await Promise.race([
        navigator.gpu.requestAdapter({ powerPreference: "high-performance" }),
        new Promise((_, reject) => {
          timer = setTimeout(
            () => reject(new Error("WebGPU adapter request timed out")),
            timeoutMs,
          );
        }),
      ]);
      if (!adapter) throw new Error("WebGPU did not provide an adapter");
      return { backend: "webgpu", notice: "" };
    } catch (error) {
      gpuFailure = String(error);
      if (requested === "webgpu") throw error;
    } finally {
      clearTimeout(timer);
    }
  }
  try {
    const gl = document.createElement("canvas").getContext("webgl2");
    if (!gl) throw new Error("WebGL2 context is unavailable");
    gl.getExtension("WEBGL_lose_context")?.loseContext();
    return {
      backend: "webgl2",
      notice: requested === "auto" ? gpuFailure : "",
    };
  } catch (error) {
    throw new Error(`${requested === "auto" ? gpuFailure + "; " : ""}${error}`);
  }
}

// One writable document per origin; other tabs can explore without saving.
// A localStorage flag is not a lock and must not replace this lease.
export async function holdSettingsWriter(locks) {
  if (!locks?.request) return { writable: false, release() {} };
  return new Promise((resolve) => {
    try {
      locks
        .request(
          "moly-settings-write",
          { mode: "exclusive", ifAvailable: true },
          async (lock) => {
            if (!lock) {
              resolve({ writable: false, release() {} });
              return;
            }
            let release;
            const lifetime = new Promise((done) => {
              release = done;
            });
            resolve({ writable: true, release });
            await lifetime;
          },
        )
        .catch(() => resolve({ writable: false, release() {} }));
    } catch {
      resolve({ writable: false, release() {} });
    }
  });
}
