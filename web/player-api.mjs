// Same-origin player-data relay. The upstream does not expose CORS headers.
export const DEFAULT_PLAYER_API = "https://haruki-api.menardi.top/api/{region}/mysekai/{target_user_id}";
const MAX_BYTES = 32 * 1024 * 1024;

export async function playerDataResponse(request, template = DEFAULT_PLAYER_API) {
  const headers = { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store" };
  const error = (status, message) => new Response(JSON.stringify({ error: message }), { status, headers });
  if (request.method !== "GET") return error(405, "GET only");
  const match = new URL(request.url).pathname.match(/^\/player-api\/(cn|jp)\/([0-9]{1,20})$/);
  if (!match || /^0+$/.test(match[2])) return error(400, "Invalid region or player UID");
  if (!/^https?:\/\//.test(template) || !template.includes("{region}") || !template.includes("{target_user_id}")) {
    return error(503, "Player API is not configured correctly");
  }
  const endpoint = template.replaceAll("{region}", match[1]).replaceAll("{target_user_id}", match[2]);
  try {
    const upstream = await fetch(endpoint, {
      headers: { Accept: "application/json" },
      signal: AbortSignal.timeout(60_000),
      redirect: "error",
    });
    if (!upstream.ok) {
      await upstream.body?.cancel();
      return error(upstream.status >= 400 ? upstream.status : 502, `Player API returned HTTP ${upstream.status}`);
    }
    if (Number(upstream.headers.get("content-length")) > MAX_BYTES) {
      await upstream.body?.cancel();
      return error(502, "Player response exceeds 32 MiB");
    }
    const reader = upstream.body?.getReader();
    if (!reader) return error(502, "Player API returned an empty response");
    const chunks = [];
    let length = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > MAX_BYTES) {
        await reader.cancel();
        return error(502, "Player response exceeds 32 MiB");
      }
      chunks.push(value);
    }
    const body = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { body.set(chunk, offset); offset += chunk.byteLength; }
    // Preserve the original bytes; parsing JSON here can round 64-bit user IDs.
    return new Response(body, { status: 200, headers });
  } catch (cause) {
    return error(cause?.name === "TimeoutError" ? 504 : 502, "Could not fetch player data");
  }
}
