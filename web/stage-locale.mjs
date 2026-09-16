const messages = {
  "zh-CN": {
    title: "让 MYSEKAI 动起来",
    prepare: "加载播放器",
    enter: "进入场景并启用声音",
    waiting: "播放器按需加载，尚未下载游戏资源。",
    loading: "正在加载引擎…",
    initializing: "正在初始化引擎…",
    renderer: "正在启动渲染器…",
    scene: "正在准备场景与公共动作…",
    ready: "准备好了。点击进入后可播放所选内容。",
    retry: "重新加载",
    fallback: "使用 WebGL2 重试",
    failed: "播放器未能完成准备。你的浏览位置已保留。",
    unsupported:
      "此浏览器无法启动 3D 渲染。请使用支持 WebGPU 或 WebGL2 的浏览器。",
    mismatch: "资源快照与当前区服不匹配，已阻止播放。",
    progress: "已接收",
    controls: "拖动调整镜头 · 滚轮缩放 · 点击对话继续 · Esc 停止",
    stalled: "准备耗时较长，可继续等待或重新加载。",
    cached: "已保留的资源会在下次进入时复用。",
    tap: "点击场景以启用键盘操作",
  },
  "en-US": {
    title: "Bring MYSEKAI to life",
    prepare: "Load player",
    enter: "Enter scene and enable sound",
    waiting:
      "The player loads on demand. No game resources have been downloaded yet.",
    loading: "Downloading engine…",
    initializing: "Initializing engine…",
    renderer: "Starting renderer…",
    scene: "Preparing the scene and shared animations…",
    ready: "Ready. Enter the scene to play your selection.",
    retry: "Reload player",
    fallback: "Retry with WebGL2",
    failed:
      "The player could not finish preparing. Your browsing position has been kept.",
    unsupported:
      "3D rendering is unavailable. Use a browser that supports WebGPU or WebGL2.",
    mismatch:
      "The resource snapshot does not match this region. Playback was blocked.",
    progress: "Received",
    controls:
      "Drag to orbit · Scroll to zoom · Click dialogue to continue · Esc to stop",
    stalled:
      "Preparation is taking longer than expected. You can wait or reload.",
    cached: "Retained resources will be reused next time.",
    tap: "Click the scene to enable keyboard controls",
  },
  "ja-JP": {
    title: "MYSEKAI が動き出す",
    prepare: "プレイヤーを読み込む",
    enter: "シーンに入り、音声を有効にする",
    waiting:
      "プレイヤーは必要なときだけ読み込みます。ゲームリソースはまだダウンロードされていません。",
    loading: "エンジンを読み込み中…",
    initializing: "エンジンを初期化中…",
    renderer: "描画を開始中…",
    scene: "シーンと共通モーションを準備中…",
    ready: "準備ができました。シーンに入って選択した内容を再生できます。",
    retry: "再読み込み",
    fallback: "WebGL2 で再試行",
    failed:
      "プレイヤーの準備が完了しませんでした。閲覧位置は保持されています。",
    unsupported:
      "3D 描画を開始できません。WebGPU または WebGL2 対応のブラウザーをご利用ください。",
    mismatch: "リソースの地域が一致しないため、再生を停止しました。",
    progress: "受信済み",
    controls:
      "ドラッグで視点移動 · スクロールでズーム · 会話をクリックして進む · Esc で停止",
    stalled: "準備に時間がかかっています。待機するか、再読み込みしてください。",
    cached: "保持したリソースは次回の起動時に再利用されます。",
    tap: "シーンをクリックするとキーボード操作が有効になります",
  },
  "zh-TW": {
    title: "讓 MYSEKAI 動起來",
    prepare: "載入播放器",
    enter: "進入場景並啟用聲音",
    waiting: "播放器按需載入，尚未下載遊戲資源。",
    loading: "正在載入引擎…",
    initializing: "正在初始化引擎…",
    renderer: "正在啟動繪圖引擎…",
    scene: "正在準備場景與共用動作…",
    ready: "準備好了。點擊進入後可播放所選內容。",
    retry: "重新載入",
    fallback: "使用 WebGL2 重試",
    failed: "播放器未能完成準備。你的瀏覽位置已保留。",
    unsupported:
      "此瀏覽器無法啟動 3D 繪圖，請使用支援 WebGPU 或 WebGL2 的瀏覽器。",
    mismatch: "資源快照與目前地區不符，已阻止播放。",
    progress: "已接收",
    controls: "拖曳調整鏡頭 · 滾輪縮放 · 點擊對話繼續 · Esc 停止",
    stalled: "準備時間較長，可繼續等待或重新載入。",
    cached: "保留的資源會在下次進入時重複使用。",
    tap: "點擊場景以啟用鍵盤操作",
  },
  "ko-KR": {
    title: "움직이는 MYSEKAI",
    prepare: "플레이어 불러오기",
    enter: "장면에 들어가 소리 켜기",
    waiting:
      "플레이어는 필요할 때만 불러옵니다. 게임 리소스는 아직 다운로드되지 않았습니다.",
    loading: "엔진 다운로드 중…",
    initializing: "엔진 초기화 중…",
    renderer: "렌더러 시작 중…",
    scene: "장면과 공통 모션 준비 중…",
    ready: "준비되었습니다. 장면에 들어가 선택한 콘텐츠를 재생하세요.",
    retry: "다시 불러오기",
    fallback: "WebGL2로 다시 시도",
    failed: "플레이어를 준비하지 못했습니다. 탐색 위치는 유지됩니다.",
    unsupported:
      "3D 렌더링을 시작할 수 없습니다. WebGPU 또는 WebGL2를 지원하는 브라우저를 사용하세요.",
    mismatch: "리소스 지역이 일치하지 않아 재생을 차단했습니다.",
    progress: "수신됨",
    controls:
      "드래그로 시점 이동 · 스크롤로 확대 · 대화 클릭으로 진행 · Esc로 중지",
    stalled: "준비가 지연되고 있습니다. 기다리거나 다시 불러오세요.",
    cached: "보관된 리소스는 다음 실행 시 재사용됩니다.",
    tap: "장면을 클릭하면 키보드 조작이 활성화됩니다",
  },
};
export function stageMessages(locale) {
  return messages[locale] ?? messages["en-US"];
}
export const STAGE_LOCALES = Object.keys(messages);
