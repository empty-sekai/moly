import {
  weatherUiMessages,
  weatherPeriodLabel,
  weatherPhaseLabel,
} from "./weather-ui-locale.mjs";

/** Keyed source catalogue view. It owns focus and filtering, never the engine's
 * weather state, timeline, availability, renderer or transition clock. */
export function createWeatherPicker({
  dialog,
  trigger,
  onSelect,
  onFocus = () => {},
}) {
  const document = dialog.ownerDocument;
  const make = (tag, cls, text = "") => {
    const e = document.createElement(tag);
    e.className = cls;
    e.textContent = text;
    return e;
  };
  const header = make("header", "weather-panel-header"),
    title = make("h2", "weather-panel-title"),
    badge = make("span", "weather-preview-badge");
  title.id = "weather-panel-title";
  dialog.setAttribute("aria-labelledby", title.id);
  const close = make("button", "weather-panel-close", "×");
  close.type = "button";
  header.append(title, badge, close);
  const filters = make("div", "weather-periods");
  filters.setAttribute("role", "group");
  const list = make("div", "weather-options");
  list.setAttribute("role", "listbox");
  const footer = make("footer", "weather-panel-footer"),
    status = make("p", "weather-panel-status"),
    scope = make("p", "weather-panel-scope");
  status.setAttribute("role", "status");
  status.setAttribute("aria-live", "polite");
  footer.append(status, scope);
  dialog.append(header, filters, list, footer);
  trigger.setAttribute("aria-haspopup", "dialog");
  trigger.setAttribute("aria-controls", dialog.id);
  trigger.setAttribute("aria-expanded", "false");
  let latest = null,
    locale = "en-US",
    filter = "all",
    focusId = null;
  const items = new Map(),
    chips = new Map();
  const period = (option) => option.metadata?.timePeriodType ?? "special";
  const visible = () =>
    latest?.options.filter(
      (option) => filter === "all" || period(option) === filter,
    ) ?? [];
  const setFocus = (id) => {
    focusId = id;
    for (const [key, item] of items) item.button.tabIndex = key === id ? 0 : -1;
  };
  function render() {
    if (!latest) return;
    const t = weatherUiMessages(locale);
    title.textContent = t.title;
    badge.textContent = t.preview;
    scope.textContent = t.scope;
    close.setAttribute("aria-label", t.close);
    trigger.setAttribute("aria-label", `${t.title} · ${latest.label}`);
    filters.setAttribute("aria-label", t.title);
    list.setAttribute("aria-label", t.title);
    const periods = ["all", ...new Set(latest.options.map(period))];
    if (!periods.includes(filter)) filter = "all";
    for (const [key, chip] of chips)
      if (!periods.includes(key)) {
        chip.remove();
        chips.delete(key);
      }
    for (const key of periods) {
      if (!chips.has(key)) {
        const chip = make("button", "weather-period");
        chip.type = "button";
        chip.dataset.period = key;
        chip.addEventListener("click", () => {
          filter = key;
          render();
        });
        chips.set(key, chip);
        filters.append(chip);
      }
      const chip = chips.get(key);
      chip.textContent = weatherPeriodLabel(key, locale);
      chip.setAttribute("aria-pressed", String(filter === key));
    }
    const ids = new Set(latest.options.map((o) => o.id));
    for (const [id, item] of items)
      if (!ids.has(id)) {
        item.button.remove();
        items.delete(id);
      }
    const committed = latest.transition
      ? latest.transition.committedId
      : latest.id;
    const requested = latest.transition?.requestedId;
    const busy = latest.transition && latest.transition.phase !== "ready";
    latest.options.forEach((option, index) => {
      if (!items.has(option.id)) {
        const button = make("button", "weather-option");
        button.type = "button";
        button.setAttribute("role", "option");
        button.dataset.phenomenonId = String(option.id);
        const picture = make("span", "weather-option-picture"),
          image = make("img", "weather-option-icon"),
          absent = make("span", "weather-option-no-art", "—");
        image.alt = "";
        image.width = 64;
        image.height = 64;
        image.hidden = true;
        picture.append(image, absent);
        const label = make("span", "weather-option-label"),
          state = make("span", "weather-option-state");
        button.append(picture, label, state);
        button.addEventListener("focus", () => setFocus(option.id));
        button.addEventListener("click", () => {
          setFocus(option.id);
          onSelect(option.id);
        });
        image.addEventListener("error", () => {
          image.hidden = true;
          absent.hidden = false;
          button.dataset.iconState = "error";
        });
        image.addEventListener("load", () => {
          button.dataset.iconState = "ready";
        });
        items.set(option.id, { button, image, absent, label, state });
      }
      const { button, image, absent, label, state } = items.get(option.id);
      if (list.children[index] !== button)
        list.insertBefore(button, list.children[index] ?? null);
      label.textContent = option.label;
      button.hidden = filter !== "all" && period(option) !== filter;
      button.setAttribute("aria-selected", String(option.id === committed));
      button.dataset.pending = String(Boolean(busy && option.id === requested));
      state.textContent =
        busy && option.id === requested
          ? t.pending
          : option.id === committed
            ? t.current
            : weatherPeriodLabel(period(option), locale);
      button.setAttribute(
        "aria-label",
        `${option.label} · ${state.textContent}`,
      );
      if (option.iconUrl && image.dataset.source !== option.iconUrl) {
        image.dataset.source = option.iconUrl;
        image.src = option.iconUrl;
        image.hidden = false;
        absent.hidden = true;
        button.dataset.iconState = "loading";
      } else if (!option.iconUrl) {
        image.removeAttribute("src");
        delete image.dataset.source;
        image.hidden = true;
        absent.hidden = false;
        absent.title = option.icon ? t.loading : t.missingArt;
        button.dataset.iconState = option.icon ? "pending" : "not-authored";
      }
    });
    const shown = visible();
    if (!shown.some((o) => o.id === focusId))
      focusId =
        shown.find((o) => o.id === committed)?.id ?? shown[0]?.id ?? null;
    setFocus(focusId);
    const message = weatherPhaseLabel(latest, locale);
    if (status.textContent !== message) status.textContent = message;
    dialog.dataset.phase = latest.transition?.phase ?? "ready";
  }
  close.addEventListener("click", () => dialog.close());
  // Canvas runtimes may prevent the browser's default Escape action. The
  // modal owns its close gesture even when that default was cancelled upstream.
  dialog.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      dialog.close();
    }
    event.stopPropagation();
  });
  dialog.addEventListener("keyup", (event) => event.stopPropagation());
  dialog.addEventListener("close", () => {
    trigger.setAttribute("aria-expanded", "false");
    onFocus(false);
    trigger.focus({ preventScroll: true });
  });
  dialog.addEventListener("click", (event) => {
    if (event.target !== dialog) return;
    const rect = dialog.getBoundingClientRect();
    if (
      event.clientX < rect.left ||
      event.clientX > rect.right ||
      event.clientY < rect.top ||
      event.clientY > rect.bottom
    )
      dialog.close();
  });
  list.addEventListener("keydown", (event) => {
    const options = visible(),
      current = options.findIndex((o) => o.id === focusId);
    if (current < 0 || !options.length) return;
    const columns = Math.max(
      1,
      getComputedStyle(list).gridTemplateColumns.split(" ").length,
    );
    const next = {
      ArrowLeft: current - 1,
      ArrowRight: current + 1,
      ArrowUp: current - columns,
      ArrowDown: current + columns,
      Home: 0,
      End: options.length - 1,
    }[event.key];
    if (next === undefined) return;
    event.preventDefault();
    const id = options[Math.max(0, Math.min(options.length - 1, next))].id;
    setFocus(id);
    items.get(id).button.focus({ preventScroll: true });
    items.get(id).button.scrollIntoView({ block: "nearest" });
  });
  return {
    update(weather, nextLocale) {
      latest = weather;
      locale = nextLocale;
      render();
    },
    open() {
      if (!latest?.options.length || dialog.open) return;
      render();
      dialog.showModal();
      onFocus(true);
      trigger.setAttribute("aria-expanded", "true");
      items.get(focusId)?.button.focus({ preventScroll: true });
    },
    close() {
      if (dialog.open) dialog.close();
    },
  };
}
