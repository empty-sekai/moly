# Recording

A recording captures the window and, optionally, the game's mix, frame by frame, and writes them to one MP4 through [ffmpeg]. While it records, the game clock advances a fixed step per frame, so the video holds exactly the frames that were drawn and the sound is rendered for the same frames. The speed of the encoder and of the machine changes only how long recording takes, not what the recording holds.

## Requirements

- A native run. The browser build has no files and no ffmpeg.
- `ffmpeg` on `PATH`, or its path in `dev.ffmpeg` (`MOLY_FFMPEG`).
- A window whose width and height in physical pixels are even (the video is stored as 4:2:0).
- For the sound, `dev.record_audio` (`MOLY_RECORD_AUDIO`) set when the game starts (see [The sound](#the-sound)).

## Starting and stopping

A native run with `dev.control_port` (`MOLY_CONTROL_PORT`) set answers JSON-RPC 2.0 on that port.

`moly.record.start` starts a recording:

| Parameter | Value | Default |
|---|---|---|
| `path` | The file to write: a file name or a path, made absolute against the working directory. Without an extension it takes `.mp4`. | required |
| `fps` | Frames per second, from 1 to 240; the game clock's step is `1/fps` seconds. | `60` |
| `encoder` | The ffmpeg video encoder: `h264_amf`, `hevc_amf`, `h264_nvenc`, `hevc_nvenc`, `h264_qsv`, `hevc_qsv`, `libx264` or `libx265`. | `h264_amf` |
| `quality` | 0 (best) to 51 (worst): QVBR level for AMF, CRF for x264 and x265, global quality for QSV, constant quality for NVENC. | the encoder's own default |

Unknown parameters are refused. Only one recording runs at a time. The answer is the recording's status (below), plus `audio`, the sound's file, when the sound is recorded.

`moly.record.stop` stops capturing at once and closes the sound. The frames still being read back reach the encoder over the next frames (at most 30). Then:

1. the video's input ends and ffmpeg is waited for;
2. the game clock returns to the update strategy it had before the recording;
3. the video and the sound are joined into `path` (the video copied, the sound encoded to AAC) and the parts are removed.

The answer to `stop` is the status with `stopping: true`. `moly.record.status` answers, while a recording runs or stops:

`{recording: true, stopping, path, fps, encoder, window: [width, height], captured, delivered, frames, failure}`

where `captured` counts the screenshots taken, `delivered` the frames handed to the encoder, and `frames` the frames the encoder has taken. Once the recording has ended it answers `{recording: false, last: {path, frames, seconds}}`, with `failed` in `last` when a frame, a write, the encoder or the join failed. A failed recording keeps its parts beside the file (`<path>.video.mp4`, `<path>.audio.wav`).

For example, against port 8765:

```sh
curl -s http://127.0.0.1:8765 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"moly.record.start","params":{"path":"take","fps":60}}'

# … play the scene …

curl -s http://127.0.0.1:8765 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"moly.record.stop","params":{}}'
curl -s http://127.0.0.1:8765 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"moly.record.status","params":{}}'
```

## What is recorded

- **The picture**: each frame, one screenshot of the primary window, which holds the finished frame. The pixels are passed on as they were read back (RGBA or BGRA, the window's own format); ffmpeg converts them from full-range sRGB to BT.709 limited-range 4:2:0. The encoder starts with the first frame, at that frame's size. A frame out of order, of another size (the window was resized) or without pixels ends the recording, naming the frame.
- **The pace**: a frame is `1/fps` seconds of the game clock, and the strategy is set again on every recorded frame, so other changes to the clock's strategy are overruled until the recording ends.
- **The encoder**: a thread takes the frames from a bounded channel of 8 and writes them into ffmpeg's standard input. While the channel is full the frame waits, so no frame is lost to a slow encoder. Failures carry the end of ffmpeg's own report.

## The sound

`dev.record_audio` (`MOLY_RECORD_AUDIO`) is read when the game starts. With it on:

- The audio output opens a device that plays nothing, instead of the sound card. The engine's mixer is that device's callback.
- Each frame, the frame loop renders the mix up to the real clock, at 48 kHz in stereo: `round(elapsed × 48000)` sample frames since start. While recording the real clock advances by the fixed step too, so each recorded frame carries exactly its share of the sound.
- During a recording the rendered samples are written to `<path>.audio.wav`, 32-bit float.
- The audio sequence's random picks come from the world seed (`dev.seed`) instead of the wall clock, so a take with a seed repeats its picks.

Without the key the sound card plays the mix and a recording has no sound; the video is still written. With the key nothing is heard from the speakers, recording or not.

## Related keys

| Key | Effect |
|---|---|
| `dev.ffmpeg` (`MOLY_FFMPEG`) | The ffmpeg program. Unset: `ffmpeg` on `PATH`. |
| `dev.record_audio` (`MOLY_RECORD_AUDIO`) | Renders the mix in the frame loop and records it. Read at start. |
| `dev.control_port` (`MOLY_CONTROL_PORT`) | The port the control surface listens on. |
| `dev.seed` (`MOLY_SEED`) | The seed of every random stream, for a repeatable take. |

See [Configuration](configuration.md) for the keys' forms and defaults.

[ffmpeg]: https://ffmpeg.org/
