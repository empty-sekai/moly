export type MolyRegion = "cn" | "jp" | "tw" | "en" | "kr";
export type MolyLocale = "zh-CN" | "zh-TW" | "en-US" | "ja-JP" | "ko-KR";
export type MolyTab =
  "conversations" | "furniture" | "performances" | "activities";
export type MolyKey =
  | `talk:${"general" | "fixture"}:${number}`
  | `fixture:${number}`
  | `activity:${"notalk" | "preaction"}:${number}:${number}`;
export type MolyTheme =
  "light" | "dark" | { mode: "light" | "dark"; accent?: string };
export interface MolyCharacter {
  id: number;
  name: string;
  originalName?: string;
  group?: string | null;
  color?: string | null;
}
export interface MolyFixture {
  id: number;
  name: string;
  image: string | null;
}
export interface MolyEntry {
  preview?: {
    available: boolean;
    tweetId?: number;
    text?: string;
    unit?: number;
    reason?: string;
  };
  key: MolyKey;
  title: string;
  subtitle: string;
  image: string | null;
  characters: MolyCharacter[];
  kind: string;
  available: boolean;
  reason?: string | null;
  reasonCode?: "source_unavailable" | "scene_unavailable" | null;
  fixtureIds: number[];
  unitIds: number[];
  presentation: {
    category:
      | "conversation"
      | "furniture"
      | "fixture_story"
      | "fixture_performance"
      | "activity";
    behavior:
      | "static"
      | "timeline"
      | "loop"
      | "one_shot"
      | "wall"
      | "floor"
      | "custom"
      | "authored";
    primaryAction: "play" | "inspect";
    textMode: "none" | "bubble" | "transcript";
  };
  description?: string;
  lines?: { speaker: string; text: string }[];
  fixtures?: MolyFixture[];
  related?: { tab: MolyTab; fixtureId: number; label: string; count: number }[];
  variant?: { index: number; count: number };
}
export interface MolyStatus {
  preview?: boolean;
  phase: "idle" | "preparing" | "playing" | "completed" | "restoring" | "error";
  label: string;
  error: string | null;
  activeKey: MolyKey | null;
  activeTitle: string | null;
  canStop: boolean;
}
export interface MolyFilters {
  tab?: MolyTab;
  query?: string;
  character?: number | null;
  fixture?: number | null;
  availability?: "all" | "ready" | "here";
  mode?: "independent" | "current";
  page?: number;
  pageSize?: number;
}
export interface MolySnapshot {
  schemaVersion: 1;
  ready: boolean;
  loading: boolean;
  revision: string;
  open: boolean;
  tab: MolyTab;
  query: string;
  character: number | null;
  availability: "all" | "ready" | "here";
  mode: "independent" | "current";
  region: MolyRegion;
  version: string;
  total: number;
  page: number;
  pageSize: number;
  tabs: { id: MolyTab; label: string; count: number }[];
  characters: MolyCharacter[];
  rows: MolyEntry[];
  selected: MolyEntry | null;
  relatedFixture: number | null;
  status: MolyStatus;
  issues: string[];
  scene?: { ready: boolean; actorUnits: number[]; fixtureIds: number[] };
  /**
   * The weather dial, in the runtime's own numbered order. `options` is the
   * complete catalogue the runtime published — never a partial guess — and
   * `setWeather` accepts only IDs from it. `label` is localized by the stage in
   * the locale the host asked for, so the host renders it verbatim.
   */
  weather?: MolyWeather;
}
export interface MolyWeatherOption {
  id: number;
  name: string;
  label?: string;
  /** Relative source artifact path; never interpreted as a weather kind. */
  icon?: string | null;
  /** Stage-verified artwork URL. Missing art stays absent. */
  iconUrl?: string | null;
  metadata?: {
    id: number;
    name: string;
    englishName?: string | null;
    description?: string | null;
    timePeriodType?: string | null;
    brightnessType?: string | null;
    backgroundColorId?: number | null;
    iconAssetbundleName?: string | null;
  } | null;
}
export interface MolyWeather {
  id: number;
  name: string;
  label?: string;
  iconUrl?: string | null;
  options: MolyWeatherOption[];
}

export interface MolyBoot {
  phase: string;
  backend: "webgpu" | "webgl2" | null;
  elapsedMs: number;
  engineDecodedBytes: number;
  transferredBytes: number;
  decodedBytes: number;
  timings: Record<string, number>;
}
export interface MolyError {
  code: string;
  key?: MolyKey;
}
export interface MountOptions {
  onPlayerData?(value: MolyPlayerDataState): void;
  view?: "shell" | "stage";
  src: string;
  assets?: string;
  resourceOrigin?: string;
  region?: MolyRegion;
  version?: string;
  snapshot?: string;
  locale?: MolyLocale;
  theme?: MolyTheme;
  renderer?: "auto" | "webgpu" | "webgl2";
  preload?: boolean;
  sound?: boolean;
  fixture?: number;
  tab?: MolyTab;
  content?: MolyKey;
  filters?: MolyFilters;
  onSnapshot?: (snapshot: MolySnapshot) => void;
  onStatus?: (status: MolyStatus) => void;
  onSelection?: (selection: {
    key: MolyKey | null;
    tab: MolyTab;
    region: MolyRegion;
    fixture: number | null;
  }) => void;
  onBoot?: (boot: MolyBoot) => void;
  onError?: (error: MolyError) => void;
}
export interface MolyMount {
  playerData(value: MolyPlayerDataCommand): void;
  readonly frame: HTMLIFrameElement;
  readonly disposed: boolean;
  readonly snapshot: MolySnapshot | null;
  setTheme(theme: MolyTheme): void;
  setLocale(locale: MolyLocale): void;
  browse(filters: MolyFilters): void;
  /** Switch the phenomenon of the live scene. Unknown IDs are rejected by the runtime. */
  setWeather(phenomenon: number): void;
  setSoundEnabled(enabled: boolean): void;
  select(key: MolyKey): void;
  play(key: MolyKey): void;
  preview(key: MolyKey): void;
  stop(): void;
  restore(): void;
  /** true only when the owner acknowledged restoration (or was already idle). */
  close(): Promise<boolean>;
  dispose(): void;
}
export function mountMoly(
  container: HTMLElement,
  options: MountOptions & {
    view: "stage";
    assets: string;
    /** Pin a release under a shared content-addressed assets store. */
    assetCatalog?: string;
    /** Legacy packs may omit assetCatalog; new publications should always pin it. */
    packs?: boolean;
    region: MolyRegion;
    version: string;
  },
): MolyMount;
export function mountMoly(
  container: HTMLElement,
  options: MountOptions,
): Pick<MolyMount, "frame" | "setTheme" | "stop" | "dispose">;

export interface MolyPlayerDataCommand {
  operation: "preview" | "explore" | "restore" | "cancel";
  region: MolyRegion;
  json?: string;
}
export interface MolyPlayerDataState {
  schemaVersion: 1;
  region: MolyRegion;
  busy: boolean;
  error: boolean;
  status: string;
  canExplore: boolean;
  exploring: boolean;
  summary: { rank: number; sites: number; fixtures: number } | null;
  /**
   * Every record of what a successful import left out, in the runtime's own
   * words (`ImportNotice` in `moly-assets`). Catalog gaps — furniture whose
   * exported model, texture or color texture is absent — drop only that
   * instance and land here; they never fail the import. Consumers localize
   * `code` themselves instead of parsing `status`.
   */
  notices: (
    | { code: "specialFurnitureRetained"; count: number }
    | { code: "surfaceAppearanceRetained"; count: number }
    | { code: "fixtureModelMissing"; count: number; fixtures: number[] }
    | { code: "fixtureTextureMissing"; count: number; fixtures: number[] }
    | { code: "fixtureColorMissing"; count: number; fixtures: number[] }
  )[];
}
