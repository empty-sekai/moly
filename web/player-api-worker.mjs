// Optional Worker entry point for a deployment with an ASSETS binding.
import { playerDataResponse, DEFAULT_PLAYER_API } from "./player-api.mjs";

export default {
  fetch(request, env) {
    if (new URL(request.url).pathname.startsWith("/player-api/")) {
      return playerDataResponse(request, env.MOLY_PLAYER_API ?? DEFAULT_PLAYER_API);
    }
    return env.ASSETS.fetch(request);
  },
};
