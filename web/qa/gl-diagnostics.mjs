// Optional observation of a failed isolated WebGL2 run. No draw is suppressed,
// reordered or replaced. State is queried only after the original draw call.
export function installGlDiagnostics() {
  const proto = globalThis.WebGL2RenderingContext?.prototype;
  if (!proto) return;
  const identities = new WeakMap();
  let nextId = 1;
  const id = (object) => {
    if (!object) return 0;
    if (!identities.has(object)) identities.set(object, nextId++);
    return identities.get(object);
  };
  const state = (globalThis.__molyGL = {
    armed: false,
    draws: [],
    totals: {},
    programs: {},
  });
  for (const name of [
    "drawArrays",
    "drawElements",
    "drawArraysInstanced",
    "drawElementsInstanced",
  ]) {
    const original = proto[name];
    proto[name] = function (...args) {
      const result = original.apply(this, args);
      const gl = this;
      const program = gl.getParameter(gl.CURRENT_PROGRAM);
      const programId = id(program);
      state.totals[programId] = (state.totals[programId] || 0) + 1;
      if (state.armed && state.draws.length < 240) {
        if (!state.programs[programId] && program) {
          state.programs[programId] = (
            gl.getAttachedShaders(program) || []
          ).map((shader) => ({
            kind: gl.getShaderParameter(shader, gl.SHADER_TYPE),
            source: gl.getShaderSource(shader),
          }));
        }
        const framebuffer = gl.getParameter(gl.DRAW_FRAMEBUFFER_BINDING);
        const viewport = Array.from(gl.getParameter(gl.VIEWPORT));
        const draw = {
          name,
          args,
          program: programId,
          framebuffer: id(framebuffer),
          viewport,
          depthTest: gl.isEnabled(gl.DEPTH_TEST),
          depthFunction: gl.getParameter(gl.DEPTH_FUNC),
          depthWrite: gl.getParameter(gl.DEPTH_WRITEMASK),
          depthClear: gl.getParameter(gl.DEPTH_CLEAR_VALUE),
          colorMask: Array.from(gl.getParameter(gl.COLOR_WRITEMASK)),
          cull: gl.isEnabled(gl.CULL_FACE),
          cullFace: gl.getParameter(gl.CULL_FACE_MODE),
          frontFace: gl.getParameter(gl.FRONT_FACE),
          scissor: gl.isEnabled(gl.SCISSOR_TEST),
          scissorBox: Array.from(gl.getParameter(gl.SCISSOR_BOX)),
        };
        if (framebuffer) {
          draw.color = id(
            gl.getFramebufferAttachmentParameter(
              gl.DRAW_FRAMEBUFFER,
              gl.COLOR_ATTACHMENT0,
              gl.FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
            ),
          );
          draw.depth = id(
            gl.getFramebufferAttachmentParameter(
              gl.DRAW_FRAMEBUFFER,
              gl.DEPTH_ATTACHMENT,
              gl.FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
            ),
          );
        }
        draw.drawBuffer0 = gl.getParameter(gl.DRAW_BUFFER0);
        draw.samples = gl.getParameter(gl.SAMPLES);
        if (
          (!framebuffer || draw.color) &&
          (!framebuffer || draw.samples === 0)
        ) {
          // Observe the just-written colour target; restore all read state.
          const previousRead = gl.getParameter(gl.READ_FRAMEBUFFER_BINDING);
          gl.bindFramebuffer(gl.READ_FRAMEBUFFER, framebuffer);
          const previousBuffer = gl.getParameter(gl.READ_BUFFER);
          gl.readBuffer(framebuffer ? gl.COLOR_ATTACHMENT0 : gl.BACK);
          const format = gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_FORMAT);
          const type = gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_TYPE);
          const Type =
            type === gl.FLOAT
              ? Float32Array
              : type === gl.UNSIGNED_BYTE
                ? Uint8Array
                : type === gl.BYTE
                  ? Int8Array
                  : type === gl.SHORT
                    ? Int16Array
                    : type === gl.HALF_FLOAT || type === gl.UNSIGNED_SHORT
                      ? Uint16Array
                      : type === gl.INT
                        ? Int32Array
                        : Uint32Array;
          const pixels = new Type(4);
          gl.readPixels(
            Math.floor(viewport[0] + viewport[2] / 2),
            Math.floor(viewport[1] + viewport[3] / 2),
            1,
            1,
            format,
            type,
            pixels,
          );
          draw.pixel = { format, type, rgba: Array.from(pixels) };
          gl.readBuffer(previousBuffer);
          gl.bindFramebuffer(gl.READ_FRAMEBUFFER, previousRead);
        }
        state.draws.push(draw);
        if (state.draws.length === 240) state.armed = false;
      }
      return result;
    };
  }
}
