// GPU validation failures may be emitted as browser warnings, not exceptions.
// This gate rejects broken rendering; it is not an original-game pixel oracle.
export function isRenderFailure(text) {
  return /Invalid (?:CommandBuffer|RenderPipeline|BindGroup)|Binding doesn't exist|GPUValidationError|uncaptured(?: GPU)? error|While calling \[Device\]\.CreateRenderPipeline|Error matching .*requirements|Shader (?:validation|compilation) error|\bGL_(?:INVALID_[A-Z_]+|OUT_OF_MEMORY|CONTEXT_LOST)\b|WebGL: (?:too many errors|CONTEXT_LOST_WEBGL)/i.test(
    text,
  );
}
export function assertVisibleScene(stats) {
  if (
    !stats ||
    stats.samples < 100 ||
    stats.nonBlack / stats.samples < 0.005 ||
    stats.maximum - stats.minimum <= 4 ||
    stats.distinct <= 8
  ) {
    throw new Error(
      `Rendered scene is blank or uniform: ${JSON.stringify(stats)}`,
    );
  }
}
export async function inspectScreenshot(page, png) {
  const stats = await page.evaluate(async (encoded) => {
    const bytes = Uint8Array.from(atob(encoded), (character) =>
      character.charCodeAt(0),
    );
    const bitmap = await createImageBitmap(
      new Blob([bytes], { type: "image/png" }),
    );
    const surface = document.createElement("canvas");
    surface.width = bitmap.width;
    surface.height = bitmap.height;
    const context = surface.getContext("2d", { willReadFrequently: true });
    context.drawImage(bitmap, 0, 0);
    bitmap.close();
    const { data, width, height } = context.getImageData(
      0,
      0,
      surface.width,
      surface.height,
    );
    let samples = 0,
      nonBlack = 0,
      minimum = 255,
      maximum = 0;
    const distinct = new Set();
    // Exclude HUD, dialogue and edge controls; a visible HTML button cannot
    // make a dead scene pass. Sampling is diagnostic, not a similarity score.
    for (let y = Math.floor(height * 0.2); y < height * 0.65; y += 4) {
      for (let x = Math.floor(width * 0.2); x < width * 0.8; x += 4) {
        const i = (y * width + x) * 4;
        const low = Math.min(data[i], data[i + 1], data[i + 2]);
        const high = Math.max(data[i], data[i + 1], data[i + 2]);
        minimum = Math.min(minimum, low);
        maximum = Math.max(maximum, high);
        nonBlack += Number(high > 2);
        samples++;
        distinct.add((data[i] << 16) | (data[i + 1] << 8) | data[i + 2]);
      }
    }
    return { samples, nonBlack, minimum, maximum, distinct: distinct.size };
  }, png.toString("base64"));
  assertVisibleScene(stats);
  return stats;
}
