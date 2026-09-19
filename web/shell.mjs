import { PackClient } from "./asset-pack-client.mjs";
import { PackedImages } from "./asset-pack-images.mjs";
import {
  assetUrl,
  safeColor,
  sourceLabel,
  initialSelection,
  initialFilters,
  pageSummary,
} from "./presentation.mjs";

const $ = (id) => document.getElementById(id);
function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = String(text);
  return node;
}
function action(text, className, click) {
  const button = element("button", className, text);
  button.type = "button";
  button.addEventListener("click", click);
  return button;
}

export class ExperienceShell {
  constructor(assetBase, options = {}) {
    this.assetBase = assetBase;
    this.packClient =
      options.packs || options.assetCatalog
        ? new PackClient(assetBase, options.assetCatalog ?? null)
        : null;
    this.packedImages = this.packClient
      ? new PackedImages(this.packClient)
      : null;
    if (this.packedImages)
      window.addEventListener("pagehide", () => this.packedImages.dispose(), {
        once: true,
      });
    this.wasm = null;
    this.snapshot = null;
    this.lastJson = "";
    this.lastList = "";
    this.lastDetail = "";
    this.portraits = new Map();
    this.pendingQuery = null;
    this.composing = false;
    this.failed = false;
    this.focused = false;
    this.initial = initialSelection(location.href);
    this.initialFilters = initialFilters(location.href);
    this.restoredSelection = false;
    this.awaitingInitialSelection = Boolean(this.initial.key);
    this.catalogCollapsed = false;
    $("catalog-search").value = this.initial.query;
    $("availability-filter").value = this.initialFilters.availability;
    this.renderTab(this.initial.tab);
    this.bind();
    this.loadPortraits();
  }
  async loadPortraits() {
    try {
      const path = "ui/character-portraits/character-portraits.json";
      let data;
      if (this.packClient) data = await this.packClient.json(path);
      else {
        const response = await fetch(`${this.assetBase}${path}`);
        if (!response.ok) return;
        data = await response.json();
      }
      for (const row of data.characters ?? []) {
        const image = assetUrl(this.assetBase, row.image);
        if (image)
          this.portraits.set(
            Number(row.unitId ?? row.id),
            this.packClient ? row.image : image,
          );
      }
      this.lastList = "";
      this.lastDetail = "";
      if (this.snapshot) this.render(this.snapshot);
    } catch {
      /* Portraits are optional display assets; names remain available. */
    }
  }
  connect(wasm) {
    this.wasm = wasm;
    this.initial = initialSelection(location.href);
    this.initialFilters = initialFilters(location.href);
    this.restoredSelection = false;
    this.awaitingInitialSelection = Boolean(this.initial.key);
    this.send("open");
    this.send("pageSize", { value: 24 });
    this.send("focus", { value: document.activeElement !== $("app-canvas") });
    this.send("tab", { value: this.initial.tab });
    this.send("mode", { value: this.initialFilters.mode });
    this.send("availability", { value: this.initialFilters.availability });
    this.send("character", { value: this.initialFilters.character });
    if (this.initial.query) {
      $("catalog-search").value = this.initial.query;
      this.send("query", { value: this.initial.query });
    }
    this.timer = setInterval(() => this.poll(), 250);
    this.poll();
  }
  fail(message) {
    if (!this.failed) this.send("stop");
    this.failed = true;
    clearInterval(this.timer);
    $("stop-button").disabled = true;
    $("playback-state").textContent = "运行中断";
    $("playback-title").textContent = message;
    $("playback-error").textContent = message;
    $("playback-error").hidden = false;
    $("stage-loading").hidden = true;
    document
      .querySelectorAll("[data-play]")
      .forEach((button) => (button.disabled = true));
  }
  send(type, fields = {}) {
    if (!this.wasm || this.failed) return false;
    try {
      this.wasm.library_command(
        JSON.stringify({ schemaVersion: 1, type, ...fields }),
      );
      return true;
    } catch (error) {
      console.error("Content command rejected", error);
      this.notify("操作暂时无法完成，请稍候再试。");
      return false;
    }
  }
  poll() {
    if (document.hidden || this.failed) return;
    try {
      const json = this.wasm.library_snapshot();
      if (json === this.lastJson) return;
      this.lastJson = json;
      const snapshot = JSON.parse(json);
      if (snapshot.schemaVersion !== 1)
        throw new Error("Unsupported library snapshot");
      this.snapshot = snapshot;
      if (
        snapshot.inspection?.id &&
        this.lastInspection !== snapshot.inspection.id
      ) {
        this.lastInspection = snapshot.inspection.id;
        this.awaitingInitialSelection = false;
        this.setFocus(false);
        this.setCollapsed(false);
        this.send("focus", { value: true });
        $("catalog-search").value = "";
        $("detail-panel").scrollIntoView({
          block: "nearest",
          behavior: "instant",
        });
      }
      if (
        this.awaitingInitialSelection &&
        snapshot.selected?.key === this.initial.key
      )
        this.awaitingInitialSelection = false;
      if (
        snapshot.ready &&
        snapshot.tab === this.initial.tab &&
        !this.restoredSelection
      ) {
        this.restoredSelection = true;
        if (
          this.initialFilters.fixture &&
          ["performances", "activities", "furniture"].includes(this.initial.tab)
        ) {
          this.send("related", {
            fixtureId: this.initialFilters.fixture,
            tab: this.initial.tab,
          });
          this.send("availability", {
            value: this.initialFilters.availability,
          });
          this.send("character", { value: this.initialFilters.character });
          if (this.initial.query)
            this.send("query", { value: this.initial.query });
        }
        if (this.initial.key) this.send("select", { key: this.initial.key });
        if (this.initialFilters.page)
          this.send("page", { value: this.initialFilters.page });
      }
      this.render(snapshot);
    } catch (error) {
      console.error("Content view could not synchronize", error);
      this.send("stop");
      this.fail("内容状态无法同步，已请求停止。请重新加载后继续。");
      this.notify("内容状态暂时无法同步，请重新加载页面。");
      $("boot-panel").hidden = false;
      $("boot-start").disabled = true;
      $("boot-status").textContent =
        "内容状态无法同步，已请求停止。请重新加载后继续。";
      $("boot-retry").hidden = false;
    }
  }
  notify(text) {
    $("toast").textContent = text;
    $("toast").hidden = false;
    clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => {
      $("toast").hidden = true;
    }, 3800);
  }
  image(relative, fallback, className = "card-image") {
    const wrap = element("span", className);
    const url = assetUrl(this.assetBase, relative);
    if (url) {
      const img = element("img");
      if (!this.packedImages) img.src = url;
      img.alt = "";
      img.loading = "lazy";
      img.decoding = "async";
      img.addEventListener(
        "error",
        () => {
          img.remove();
          wrap.textContent = fallback;
        },
        { once: true },
      );
      wrap.append(img);
      if (this.packedImages) this.packedImages.bind(img, relative);
    } else wrap.textContent = fallback;
    return wrap;
  }
  avatar(character) {
    const wrap = element("span", "character-avatar");
    const name = character.name ?? "";
    wrap.title = name;
    const color = safeColor(character.color);
    if (color)
      wrap.style.background = `color-mix(in srgb, ${color} 16%, var(--surface))`;
    const url = this.portraits.get(character.id);
    if (url) {
      const image = element("img");
      if (!this.packedImages) image.src = url;
      image.alt = "";
      image.loading = "lazy";
      image.addEventListener(
        "error",
        () => {
          image.remove();
          wrap.textContent = name.slice(-2);
        },
        { once: true },
      );
      wrap.append(image);
      if (this.packedImages) this.packedImages.bind(image, url);
    } else wrap.textContent = name.slice(-2) || "♪";
    return wrap;
  }
  rowArt(row) {
    if (row.image) return this.image(row.image, "⌂");
    if (row.characters?.length) {
      const pair = element("span", "avatar-pair");
      row.characters
        .slice(0, 2)
        .forEach((character) => pair.append(this.avatar(character)));
      return pair;
    }
    return this.image(null, row.kind?.includes("家具") ? "⌂" : "♪");
  }
  choose(key) {
    this.abandonInitialSelection();
    this.send("select", { key });
    this.updateUrl({ content: key });
    $("detail-panel").scrollTop = 0;
  }
  emitHost(type, value) {
    if (window.parent !== window)
      window.parent.postMessage(
        { source: "moly", schemaVersion: 1, type, value },
        location.origin,
      );
  }
  updateUrl(fields) {
    const url = new URL(location.href);
    for (const [key, value] of Object.entries(fields)) {
      if (value) url.searchParams.set(key, value);
      else url.searchParams.delete(key);
    }
    history.replaceState(null, "", url);
  }
  abandonInitialSelection() {
    this.restoredSelection = true;
    this.awaitingInitialSelection = false;
  }
  renderTab(tab) {
    for (const button of $("catalog-tabs").querySelectorAll("button")) {
      const active = tab === button.dataset.tab;
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    }
  }
  syncSelectionUrl(force = false) {
    const s = this.snapshot;
    // The first ready snapshot predates the queued selection command. Preserve
    // the user's opaque deep-link until that exact entry has been acknowledged.
    if (
      (!this.restoredSelection && !force) ||
      (!force && this.awaitingInitialSelection) ||
      !s?.ready
    )
      return;
    this.updateUrl({
      tab: s?.tab && s.tab !== "conversations" ? s.tab : "",
      q: s?.query || "",
      content: s?.selected?.key || "",
      character: s?.character || "",
      availability: s?.availability !== "all" ? s?.availability : "",
      experience: s?.mode === "current" ? "current" : "",
      page: s?.page || "",
      fixture: s?.relatedFixture || "",
    });
  }
  render(s) {
    $("source-label").textContent = sourceLabel(s.region, s.version);
    this.renderTab(s.tab);
    const selectionStamp = JSON.stringify([s.selected?.key, s.tab, s.region]);
    if (
      s.ready &&
      !this.awaitingInitialSelection &&
      selectionStamp !== this.hostSelectionStamp
    ) {
      this.hostSelectionStamp = selectionStamp;
      this.emitHost("selection", {
        key: s.selected?.key ?? null,
        tab: s.tab,
        region: s.region,
      });
    }
    for (const button of $("catalog-tabs").querySelectorAll("button")) {
      const info = s.tabs?.find((tab) => tab.id === button.dataset.tab);
      if (info)
        button.title = `${info.label} · ${info.count.toLocaleString("zh-CN")} 项`;
    }
    const characterStamp = JSON.stringify(s.characters ?? []);
    if (characterStamp !== this.characterStamp) {
      this.characterStamp = characterStamp;
      const options = [
        new Option("全部角色", ""),
        ...(s.characters ?? []).map((c) => new Option(c.name, c.id)),
      ];
      $("character-filter").replaceChildren(...options);
    }
    if (document.activeElement !== $("character-filter"))
      $("character-filter").value = s.character ?? "";
    if (document.activeElement !== $("availability-filter"))
      $("availability-filter").value = s.availability;
    if (
      document.activeElement !== $("catalog-search") &&
      this.pendingQuery === null &&
      !this.composing
    )
      $("catalog-search").value = s.query ?? "";
    $("result-count").textContent = s.ready
      ? `${(s.total ?? 0).toLocaleString("zh-CN")} 项内容`
      : "正在整理内容…";
    const issueText = [...new Set(s.issues ?? [])].join("\n");
    $("catalog-issues").hidden = !issueText;
    $("catalog-issues").querySelector("div").textContent = issueText;
    $("related-filter").hidden = !s.relatedFixture;
    if (s.relatedFixture) {
      const fixtureName = s.selected?.fixtures?.find(
        (fixture) => fixture.id === s.relatedFixture,
      )?.name;
      $("related-filter").querySelector("span").textContent =
        fixtureName ||
        (s.selected?.key === `fixture:${s.relatedFixture}`
          ? s.selected.title
          : "所选家具");
    }
    $("page-label").textContent = pageSummary(s.total, s.page, s.pageSize);
    $("previous-page").disabled = s.page <= 0;
    $("next-page").disabled = (s.page + 1) * s.pageSize >= s.total;
    const listStamp = JSON.stringify([
      s.rows,
      s.selected?.key,
      s.page,
      s.query,
      s.ready,
    ]);
    if (listStamp !== this.lastList) {
      const focusedKey = document.activeElement?.dataset.key;
      const oldScroll = $("catalog-list").scrollTop;
      const pageChanged =
        this.previousPage !== s.page ||
        this.previousTab !== s.tab ||
        this.previousQuery !== s.query;
      this.previousPage = s.page;
      this.previousTab = s.tab;
      this.previousQuery = s.query;
      this.lastList = listStamp;
      const rows = (s.rows ?? []).map((row) => {
        const card = action(
          "",
          `catalog-card${row.key === s.selected?.key ? " selected" : ""}`,
          () => this.choose(row.key),
        );
        card.dataset.key = row.key;
        card.setAttribute("aria-pressed", String(row.key === s.selected?.key));
        card.append(this.rowArt(row));
        const copy = element("span", "card-copy");
        const title = element("span", "card-title", row.title);
        title.title = row.title;
        copy.append(title, element("span", "card-preview", row.subtitle));
        const meta = element("span", "card-meta");
        meta.append(element("span", null, row.kind));
        const ready = element("span");
        ready.append(
          element(
            "i",
            `availability-dot${row.available ? "" : " unavailable"}`,
          ),
          document.createTextNode(row.available ? "可体验" : "暂不可用"),
        );
        meta.append(ready);
        copy.append(meta);
        card.append(copy, element("span", "card-arrow", "›"));
        return card;
      });
      if (!rows.length) {
        const empty = element("div", "empty-state");
        empty.append(
          element("span", "empty-icon", s.ready ? "⌕" : "♫"),
          element("h2", null, s.ready ? "没有找到相符的内容" : "正在准备内容"),
          element(
            "p",
            null,
            s.ready
              ? "试试其他关键词，或重置角色和可用性筛选。"
              : "正在读取对话、家具与角色资料，请稍候。",
          ),
        );
        if (s.ready)
          empty.append(action("重置筛选", "text-button", () => this.reset()));
        rows.push(empty);
      }
      $("catalog-list").replaceChildren(...rows);
      $("catalog-list").scrollTop = pageChanged ? 0 : oldScroll;
      if (focusedKey)
        [...$("catalog-list").querySelectorAll("[data-key]")]
          .find((n) => n.dataset.key === focusedKey)
          ?.focus({ preventScroll: true });
    }
    const detailStamp = JSON.stringify([
      s.selected,
      s.mode,
      s.character,
      s.availability,
      s.page,
      s.relatedFixture,
    ]);
    if (detailStamp !== this.lastDetail) {
      this.lastDetail = detailStamp;
      this.renderDetail(s);
      this.syncSelectionUrl();
    }
    const status = s.status ?? {};
    const phase = status.phase ?? "idle";
    $("playback-error").textContent = status.error || "";
    $("playback-error").hidden = !status.error;
    const statusStamp = JSON.stringify([
      phase,
      status.activeKey,
      status.canStop,
      status.error,
    ]);
    if (statusStamp !== this.hostStatusStamp) {
      this.hostStatusStamp = statusStamp;
      this.emitHost("status", {
        ...status,
        region: s.region,
        version: s.version,
      });
    }
    $("playback-state").textContent =
      {
        idle: "待播放",
        preparing: "准备中",
        playing: "演出中",
        completed: "播放完毕 · 场景保留中",
        restoring: "恢复中",
        error: "暂不可用",
      }[phase] ?? "待播放";
    $("playback-title").textContent =
      status.activeTitle || status.label || "选择内容，开启一段日常";
    $("playback-title").title = status.label || status.activeTitle || "";
    $("stop-button").disabled = !status.canStop;
    $("restore-button").hidden = !status.canStop;
    $("live-dot").classList.toggle("running", phase === "playing");
    $("stage-label").textContent = status.activeTitle || "你的 MYSEKAI";
    $("stage-loading").hidden = !["preparing", "restoring"].includes(phase);
    $("stage-loading-label").textContent = status.label || "正在准备场景…";
    const playButton = document.querySelector("[data-play]");
    if (playButton) {
      playButton.disabled =
        this.failed || !s.selected?.available || phase === "restoring";
      playButton.textContent =
        ["playing", "completed"].includes(phase) &&
        status.activeKey === s.selected?.key
          ? "↻ 重新播放"
          : s.selected?.kind === "陈设家具"
            ? "▷ 查看家具"
            : "▷ 开始体验";
    }
    for (const button of document.querySelectorAll("[data-mode]"))
      button.disabled = status.canStop;
    if (status.error && status.error !== this.lastError)
      this.notify(status.error);
    this.lastError = status.error;
  }
  renderDetail(s) {
    const row = s.selected;
    if (!row) {
      const empty = element("div", "detail-empty");
      empty.append(element("span", "detail-empty-symbol", "✦"));
      const copy = element("div");
      copy.append(
        element("h2", null, "选择一段你想看的内容"),
        element("p", null, "查看台词和登场角色，再开始体验。"),
      );
      empty.append(copy);
      $("detail-panel").replaceChildren(empty);
      return;
    }
    const fragment = document.createDocumentFragment();
    const header = element("div", "detail-header");
    if (row.image)
      header.append(this.image(row.image, "⌂", "detail-thumbnail"));
    const summary = element("div", "detail-summary");
    const kicker = element("div", "detail-kicker", row.kind);
    summary.append(kicker, element("h2", null, row.title));
    if (row.description)
      summary.append(element("p", "detail-description", row.description));
    header.append(summary);
    const controls = element("div", "detail-play");
    const play = action("▷ 开始体验", "primary-button", () => this.play());
    play.dataset.play = "";
    const modes = element("div", "mode-switch");
    modes.setAttribute("aria-label", "体验方式");
    for (const [value, label] of [
      ["independent", "独立体验"],
      ["current", "当前场景"],
    ]) {
      const button = action(label, value === s.mode ? "active" : "", () =>
        this.send("mode", { value }),
      );
      button.dataset.mode = value;
      button.setAttribute("aria-pressed", String(value === s.mode));
      modes.append(button);
    }
    controls.append(play, modes);
    header.append(controls);
    fragment.append(header);
    fragment.append(
      element(
        "p",
        "mode-note",
        s.mode === "independent"
          ? "自动准备所需角色与家具 · 结束后返回原场景"
          : "使用当前场景中已有的角色与家具",
      ),
    );
    if (!row.available && row.reason)
      fragment.append(element("p", "reason", row.reason));
    if (row.characters?.length) {
      const cast = element("div", "cast");
      row.characters.forEach((c) => {
        const member = element("span", "cast-member");
        member.append(this.avatar(c), document.createTextNode(c.name));
        cast.append(member);
      });
      fragment.append(cast);
    }
    if (row.related?.length) {
      const links = element("div", "related-links");
      row.related.forEach((r) => {
        if (!r.count || r.tab === "furniture") return;
        links.append(
          action(`${r.label}${r.count ? ` · ${r.count}` : ""} →`, "", () => {
            this.abandonInitialSelection();
            clearTimeout(this.queryTimer);
            this.pendingQuery = null;
            this.send("related", { fixtureId: r.fixtureId, tab: r.tab });
            this.updateUrl({
              tab: r.tab,
              q: "",
              content: "",
              fixture: r.fixtureId,
              page: "",
            });
            this.setCollapsed(false);
            $("related-filter").hidden = false;
            $("related-filter").querySelector("span").textContent = row.title;
            $("catalog-search").value = "";
          }),
        );
      });
      fragment.append(links);
    }
    if (row.fixtures?.length && s.tab !== "furniture") {
      const links = element("div", "related-links");
      row.fixtures.forEach((f) =>
        links.append(
          action(`⌂ ${f.name}`, "", () => {
            this.abandonInitialSelection();
            clearTimeout(this.queryTimer);
            this.pendingQuery = null;
            this.send("related", { fixtureId: f.id, tab: "furniture" });
            this.updateUrl({
              tab: "furniture",
              q: "",
              content: "",
              fixture: f.id,
              page: "",
            });
            this.setCollapsed(false);
            $("related-filter").hidden = false;
            $("related-filter").querySelector("span").textContent = f.name;
          }),
        ),
      );
      fragment.append(links);
    }
    if (row.lines?.length) {
      const label = element("div", "detail-section-title");
      label.append(
        element("span", null, "对话内容"),
        element("span", null, `${row.lines.length} 句 · 按原始顺序`),
      );
      fragment.append(label);
      const transcript = element("div", "transcript");
      row.lines.forEach((line, index) => {
        const item = element("div", "dialogue-line");
        item.append(
          element(
            "span",
            "dialogue-number",
            String(index + 1).padStart(2, "0"),
          ),
          element("span", "dialogue-speaker", line.speaker),
          element("p", "dialogue-text", line.text),
        );
        transcript.append(item);
      });
      fragment.append(transcript);
    } else if (s.tab === "activities")
      fragment.append(
        element(
          "p",
          "mode-note",
          "这是一段角色与家具的动作互动，可能包含短气泡，没有普通对话台词。",
        ),
      );
    const share = action("复制这段内容的链接", "text-button", async () => {
      this.syncSelectionUrl(true);
      try {
        await navigator.clipboard.writeText(location.href);
        this.notify("已复制，可直接打开这段内容。");
      } catch {
        this.notify("可以从地址栏复制当前内容链接。");
      }
    });
    fragment.append(share);
    const scroll = $("detail-panel").scrollTop;
    $("detail-panel").replaceChildren(fragment);
    $("detail-panel").scrollTop = scroll;
  }
  play() {
    if (
      !this.snapshot?.selected?.available ||
      this.failed ||
      this.snapshot.status?.phase === "restoring"
    )
      return;
    this.send("play", { key: this.snapshot.selected.key });
    if (matchMedia("(max-width:760px)").matches)
      $("stage").scrollIntoView({ block: "start", behavior: "smooth" });
  }
  reset() {
    this.abandonInitialSelection();
    clearTimeout(this.queryTimer);
    this.pendingQuery = null;
    $("catalog-search").value = "";
    $("character-filter").value = "";
    $("availability-filter").value = "all";
    $("related-filter").hidden = true;
    this.send("tab", { value: this.snapshot?.tab ?? "conversations" });
    this.send("query", { value: "" });
    this.send("character", { value: null });
    this.send("availability", { value: "all" });
    this.updateUrl({
      q: "",
      content: "",
      character: "",
      availability: "",
      page: "",
      fixture: "",
    });
  }
  setCollapsed(collapsed) {
    this.catalogCollapsed = collapsed;
    $("workspace").classList.toggle("catalog-collapsed", collapsed);
    $("expand-button").hidden = !collapsed;
  }
  setFocus(value) {
    $("workspace").classList.toggle("focus-mode", value);
    $("focus-button").setAttribute("aria-pressed", String(value));
    $("focus-button").textContent = value ? "退出专注" : "专注演出";
    this.setCollapsed(value);
  }
  bind() {
    $("settings-button").addEventListener("click", () => {
      if (!this.wasm || this.failed) {
        this.notify("进入 MYSEKAI 后可调整画质、音量与玩家数据。");
        return;
      }
      this.send("settings", { value: "toggle" });
      $("app-canvas").focus({ preventScroll: true });
      this.focused = false;
      this.send("focus", { value: false });
    });
    try {
      document.documentElement.dataset.theme =
        localStorage.getItem("moly-web-theme") ?? "light";
    } catch {
      /* Storage is optional. */
    }
    const params = new URL(location.href).searchParams;
    if (params.get("embed") === "1") {
      document.documentElement.classList.add("embedded");
      if (["light", "dark"].includes(params.get("theme")))
        document.documentElement.dataset.theme = params.get("theme");
      window.addEventListener("message", (event) => {
        if (
          event.source !== window.parent ||
          event.origin !== location.origin ||
          event.data?.source !== "moly-host" ||
          event.data?.schemaVersion !== 1
        )
          return;
        if (
          event.data.type === "theme" &&
          ["light", "dark"].includes(event.data.value)
        )
          document.documentElement.dataset.theme = event.data.value;
        if (event.data.type === "stop") this.send("stop");
        if (event.data.type === "close") this.send("close");
      });
    }
    $("theme-button").addEventListener("click", () => {
      const theme =
        document.documentElement.dataset.theme === "dark" ? "light" : "dark";
      document.documentElement.dataset.theme = theme;
      try {
        localStorage.setItem("moly-web-theme", theme);
      } catch {
        /* Session theme still works. */
      }
    });
    const help = () => {
      if (!$("help-dialog").open) $("help-dialog").showModal();
    };
    $("help-button").addEventListener("click", help);
    $("shortcuts-button").addEventListener("click", help);
    $("collapse-button").addEventListener("click", () =>
      this.setCollapsed(true),
    );
    $("expand-button").addEventListener("click", () => {
      this.setFocus(false);
      $("catalog-search").focus();
    });
    $("focus-button").addEventListener("click", () =>
      this.setFocus(!$("workspace").classList.contains("focus-mode")),
    );
    $("fullscreen-button").addEventListener("click", async () => {
      try {
        if (document.fullscreenElement) await document.exitFullscreen();
        else
          await document.querySelector(".experience-panel").requestFullscreen();
      } catch {
        this.setFocus(true);
        this.notify("已展开演出画面。");
      }
    });
    for (const button of $("catalog-tabs").querySelectorAll("button"))
      button.addEventListener("click", () => {
        this.abandonInitialSelection();
        this.renderTab(button.dataset.tab);
        this.send("tab", { value: button.dataset.tab });
        $("related-filter").hidden = true;
        this.updateUrl({
          tab: button.dataset.tab,
          content: "",
          fixture: "",
          page: "",
        });
      });
    const query = () => {
      clearTimeout(this.queryTimer);
      this.pendingQuery = $("catalog-search").value;
      if (this.composing) return;
      this.queryTimer = setTimeout(() => {
        this.abandonInitialSelection();
        const value = this.pendingQuery ?? "";
        this.send("query", { value });
        this.updateUrl({ q: value, content: "", page: "" });
        this.pendingQuery = null;
      }, 160);
    };
    $("catalog-search").addEventListener("compositionstart", () => {
      this.composing = true;
      clearTimeout(this.queryTimer);
    });
    $("catalog-search").addEventListener("compositionend", () => {
      this.composing = false;
      query();
    });
    $("catalog-search").addEventListener("input", query);
    $("character-filter").addEventListener("change", (event) => {
      this.abandonInitialSelection();
      this.send("character", {
        value: event.target.value ? Number(event.target.value) : null,
      });
      this.updateUrl({ character: event.target.value, content: "", page: "" });
    });
    $("availability-filter").addEventListener("change", (event) => {
      this.abandonInitialSelection();
      this.send("availability", { value: event.target.value });
      this.updateUrl({
        availability: event.target.value === "all" ? "" : event.target.value,
        content: "",
        page: "",
      });
    });
    $("reset-filters").addEventListener("click", () => this.reset());
    $("related-filter")
      .querySelector("button")
      .addEventListener("click", () => this.reset());
    $("previous-page").addEventListener("click", () =>
      this.send("page", { value: Math.max(0, this.snapshot.page - 1) }),
    );
    $("next-page").addEventListener("click", () =>
      this.send("page", { value: this.snapshot.page + 1 }),
    );
    $("stop-button").addEventListener("click", () => this.send("stop"));
    $("restore-button").addEventListener("click", () => this.send("restore"));
    document.addEventListener("focusin", (event) => {
      const domFocused = event.target !== $("app-canvas");
      if (domFocused !== this.focused) {
        this.focused = domFocused;
        this.send("focus", { value: domFocused });
      }
    });
    $("app-canvas").addEventListener("pointerdown", () => {
      $("app-canvas").focus({ preventScroll: true });
      this.focused = false;
      this.send("focus", { value: false });
    });
    for (const name of ["pointerdown", "wheel"])
      document.addEventListener(
        name,
        (event) => {
          if (event.target !== $("app-canvas")) {
            this.focused = true;
            this.send("focus", { value: true });
          }
        },
        { capture: true, passive: true },
      );
    document.addEventListener(
      "keydown",
      (event) => {
        if (event.isComposing || this.composing) return;
        const editing = event.target.matches("input,select,textarea");
        const canvas = event.target === $("app-canvas");
        if ($("help-dialog").open) return;
        if (
          (event.key === "/" && !editing) ||
          ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k")
        ) {
          event.preventDefault();
          event.stopImmediatePropagation();
          this.setFocus(false);
          $("catalog-search").focus();
          $("catalog-search").select();
        } else if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
          event.preventDefault();
          event.stopImmediatePropagation();
          this.play();
        } else if (event.key === "Escape") {
          event.preventDefault();
          event.stopImmediatePropagation();
          if (this.snapshot?.status?.canStop) this.send("stop");
          else this.setFocus(false);
        } else if (
          !editing &&
          !canvas &&
          ["ArrowUp", "ArrowDown"].includes(event.key)
        ) {
          const rows = this.snapshot?.rows ?? [];
          if (rows.length) {
            event.preventDefault();
            event.stopImmediatePropagation();
            const index = rows.findIndex(
              (row) => row.key === this.snapshot.selected?.key,
            );
            const next = Math.max(
              0,
              Math.min(
                rows.length - 1,
                index + (event.key === "ArrowUp" ? -1 : 1),
              ),
            );
            this.choose(rows[next].key);
            [...$("catalog-list").querySelectorAll("[data-key]")]
              .find((n) => n.dataset.key === rows[next].key)
              ?.scrollIntoView({ block: "nearest" });
          }
        } else if (editing || !canvas) event.stopPropagation();
      },
      true,
    );
    // DOM controls never send clicks or scrolling to game window listeners.
    for (const name of [
      "pointerdown",
      "pointerup",
      "mousedown",
      "mouseup",
      "click",
      "wheel",
      "keyup",
    ]) {
      document.addEventListener(name, (event) => {
        if (event.target !== $("app-canvas")) event.stopPropagation();
      });
    }
    window.addEventListener("pagehide", () => {
      clearInterval(this.timer);
      clearTimeout(this.queryTimer);
      clearTimeout(this.toastTimer);
      this.send("close");
    });
  }
}
