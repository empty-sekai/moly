// Optional Worker entry point for a deployment with an ASSETS binding.
import { playerDataResponse } from "./player-api.mjs";

export default {
  fetch(request, env) {
    if (new URL(request.url).pathname.startsWith("/player-api/")) {
      const template = env.MOLY_PLAYER_API;
      if (typeof template !== "string" || !template)
        return new Response(
          JSON.stringify({
            error: "Player API is not configured: set MOLY_PLAYER_API",
          }),
          {
            status: 503,
            headers: {
              "Content-Type": "application/json; charset=utf-8",
              "Cache-Control": "no-store",
            },
          },
        );
      return playerDataResponse(request, template);
    }
    return env.ASSETS.fetch(request);
  },
};
