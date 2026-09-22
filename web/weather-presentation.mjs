import { weatherLabel } from "./stage-locale.mjs";
import { resourceDirectory } from "./embed-contract.mjs";

/** A missing source icon stays missing; labels never select replacement art. */
export function sourceWeatherIcon(file, assets, baseUrl) {
  if (typeof file !== "string" || !file || /[\\:?#\x00-\x1f]/.test(file))
    return null;
  try {
    const parts = file.split("/").map(decodeURIComponent);
    if (
      parts.some(
        (part) =>
          !part ||
          part === "." ||
          part === ".." ||
          /[\/\\:?#\x00-\x1f]/.test(part),
      )
    )
      return null;
    const base = resourceDirectory(assets, baseUrl);
    const origin = base.origin;
    if (
      base.origin !== origin ||
      !base.pathname.endsWith("/") ||
      base.search ||
      base.hash
    )
      return null;
    const icon = new URL(`phenomena/${file}`, base);
    return icon.origin === origin &&
      icon.pathname.startsWith(base.pathname + "phenomena/")
      ? icon.href
      : null;
  } catch {
    return null;
  }
}

export function presentWeather(
  weather,
  { assets, baseUrl, locale, region, packed = false, iconUrls = new Map() },
) {
  if (!weather) return weather;
  const options = weather.options.map((option) => ({
    ...option,
    label:
      locale === "ja-JP" && region === "jp" && option.metadata?.name
        ? option.metadata.name
        : weatherLabel(locale, option.name, option.metadata?.name),
    // A logical path is not proof of the returned pixels, even in flat mode.
    iconUrl: iconUrls.get(option.icon) ?? null,
  }));
  const committedId = weather.transition
    ? weather.transition.committedId
    : weather.id;
  const current = options.find((option) => option.id === committedId);
  const requested = options.find(
    (option) => option.id === weather.transition?.requestedId,
  );
  return {
    ...weather,
    options,
    label: current?.label ?? weatherLabel(locale, weather.name),
    committedId,
    requestedLabel: requested?.label ?? null,
    iconUrl: current?.iconUrl ?? null,
  };
}
