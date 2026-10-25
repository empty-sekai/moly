// The public iframe adapter exposes the maintained stage contract.
import { mountStage } from "./embed-stage.mjs";
export {
  loadTextSession,
  loadTextIndex,
  localizeEntry,
  validateTextSession,
} from "./text-localization.mjs";

export function mountMoly(container, options = {}) {
  if (options.view !== "stage") throw new TypeError('view must be "stage"');
  return mountStage(container, options);
}
