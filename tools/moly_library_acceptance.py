"""Native content-library acceptance through real, foreground-checked UI input.

Start Moly with MOLY_LIBRARY_QA_OUT pointing at a telemetry JSON file, then:
  python tools/moly_library_acceptance.py --pid 1234 --live C:\\qa\\live.json --out C:\\qa\\run --suite story --talk 5657

No entities, requests, catalog entries or assets are injected. The driver uses
only the public UI; failures save the live state and exit nonzero. Screenshots
are evidence for a separate visual review, never an automatic quality claim.
"""
from __future__ import annotations
import argparse
import contextlib
import io
import json
import math
from pathlib import Path
import sys
import time
from typing import Any, Callable
import moly_window_qa as ui

class Acceptance:
    def __init__(self, pid: int, live: Path, out: Path) -> None:
        self.pid, self.live, self.out = pid, live, out
        self.out.mkdir(parents=True, exist_ok=True)
        self.events: list[dict[str, Any]] = []

    def read(self) -> dict[str, Any]:
        for _ in range(10):
            try:
                return json.loads(self.live.read_text(encoding='utf-8'))
            except (OSError, json.JSONDecodeError):
                time.sleep(0.1)
        raise RuntimeError('Moly telemetry is unavailable or incomplete')

    def drive(self, *args: object) -> None:
        argv = sys.argv
        try:
            sys.argv = ['moly_window_qa', '--pid', str(self.pid), '--wait', '0.25', *map(str, args)]
            with contextlib.redirect_stdout(io.StringIO()):
                ui.main()
        finally:
            sys.argv = argv

    def wait(self, check: Callable[[dict[str, Any]], bool], message: str, timeout: float = 10.) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            state = self.read()
            if check(state):
                return state
            if time.time() - self.live.stat().st_mtime > 5:
                raise RuntimeError('Live telemetry stopped updating; the app may have exited')
            time.sleep(0.12)
        raise AssertionError(f'{message}: {json.dumps(self.read(), ensure_ascii=False)[:1800]}')

    def record(self, name: str) -> dict[str, Any]:
        state = self.read()
        self.events.append({'name': name, 'time': time.time(), 'state': state})
        self.save()
        return state

    def save(self) -> None:
        (self.out / 'acceptance.json').write_text(json.dumps(self.events, ensure_ascii=False, indent=2), encoding='utf-8')

    def screenshot(self, name: str) -> None:
        self.drive('capture', self.out / f'{name}.png')
        self.record(name)

    def open(self) -> None:
        if not self.read()['open']:
            self.drive('key', 'F9')
            self.wait(lambda s: s['open'], 'Library did not open')

    def select(self, tab: int, query: str, expected: str | None = None) -> None:
        self.open()
        self.drive('key', f'CTRL+{tab}')
        self.drive('key', 'CTRL+F')
        self.drive('key', 'CTRL+A')
        self.drive('text', query)
        self.drive('key', 'ENTER')
        self.wait(lambda s: s['search'] == query and (expected is None or s['selected'] == expected), 'Exact catalog selection was not reached')

    def play(self, expected: str) -> dict[str, Any]:
        self.drive('key', 'ENTER')
        state = self.wait(lambda s: s['active'] == expected and s['started'] and s['watching'], f'{expected} did not start', 70.)
        session = state.get('independent')
        if state.get('mode') == 'Independent':
            assert session and session['phase'] in ('Staged', 'Experiencing'), 'Independent session did not reach a staged phase'
            assert session['ticket'] > 0 and session['destination'] == state['site'], 'Independent site/ticket mismatch'
        return state

    @staticmethod
    def idle(s: dict[str, Any]) -> bool:
        return (s['active'] is None and s['pending'] is None and not s['player_fixture_active']
                and not s['player_control_owned'] and not s['scene_preview']
                and s.get('independent') is None
                and s['gimmick_owners'] == 0 and s['held_actors'] == 0
                and s['voice_players'] == 0 and s['scoped_sounds'] == 0)

    def stop(self) -> dict[str, Any]:
        state = self.read()
        if state['open']:
            # Watching is entered via the public Continue/F9 flow, not state edits.
            self.drive('key', 'F9')
        self.drive('key', 'ESC')
        return self.wait(self.idle, 'Stop did not release playback ownership')

    def story(self, talk: int, backend: str) -> None:
        key = f'Talk({backend}, {talk})'
        self.select(1, str(talk), key)
        before = self.record('before-story')['player']['position']
        self.play(key)
        self.screenshot('story-start')
        seen: list[tuple[str, str]] = []
        deadline = time.monotonic() + 180.
        finale_shot = False
        while time.monotonic() < deadline:
            state = self.read()
            if state['active'] is None:
                break
            transcript = state['transcript']
            line = (transcript['speaker'], transcript['text'])
            if line[1] and (not seen or seen[-1] != line):
                seen.append(line)
                self.record(f'line-{len(seen):02d}')
            if transcript['visible']:
                # Reveal or advance, but never inject a click into an authored
                # hidden-window finale: its delay and controller must actually run.
                self.drive('click', 1070, 619)
                # The read-only producer runs at 2Hz. Never send a second
                # dialogue click based on its old visible-window snapshot.
                checkpoint = self.live.stat().st_mtime_ns
                self.wait(lambda _: self.live.stat().st_mtime_ns > checkpoint,
                          'No fresh transcript after dialogue input', 3.)
            else:
                if state['gimmick_owners'] and not finale_shot:
                    time.sleep(1.0)
                    self.screenshot('authored-furniture-finale')
                    finale_shot = True
                time.sleep(0.2)
        else:
            raise AssertionError('Story did not complete within its acceptance budget')
        final = self.wait(self.idle, 'Natural completion did not release all owners')
        assert math.dist(before, final['player']['position']) < 0.025, 'Player position was not restored'
        (self.out / 'observed-transcript.json').write_text(json.dumps(seen, ensure_ascii=False, indent=2), encoding='utf-8')
        self.screenshot('story-finished')

    def stress(self, cycles: int) -> None:
        key = 'Talk(General, 3912)'
        self.select(1, '3912', key)
        before = self.record('stress-baseline')['player']['position']
        for index in range(cycles):
            self.play(key)
            stopped = self.stop()
            assert math.dist(before, stopped['player']['position']) < 0.025, 'Player drift after repeated stop'
            self.record(f'play-stop-{index + 1:02d}')
        self.select(2, '423', 'Fixture(423)')
        self.play('Fixture(423)')
        time.sleep(2.)  # Longer than the source player's switch gesture.
        assert self.read()['gimmick_owners'] == 1, 'Loop ended with the switch gesture'
        self.screenshot('persistent-furniture-loop')
        self.stop()
        self.record('loop-stopped')
        # Replacement while the previous owner is alive, not a stopped replay.
        self.select(1, '3912', key)
        self.play(key)
        self.select(1, '6998', 'Talk(Fixture, 6998)')
        self.play('Talk(Fixture, 6998)')
        self.record('replaced-general-with-furniture-story')
        self.stop()
        # Closing the open library must clean up the owner's entire lifecycle.
        self.play('Talk(Fixture, 6998)')
        self.open()
        self.drive('key', 'ESC')
        self.wait(lambda s: self.idle(s) and not s['open'], 'Closing the library left a live owner')
        self.record('close-during-playback')
        self.open()

    def furniture(self, fixture: int, natural: bool) -> None:
        key = f'Fixture({fixture})'
        self.select(2, str(fixture), key)
        before = self.record('before-furniture')['player']['position']
        self.play(key)
        time.sleep(1.5)
        self.screenshot('furniture-active')
        if natural:
            final = self.wait(self.idle, 'One-shot did not finish naturally', 45.)
        else:
            final = self.stop()
        assert math.dist(before, final['player']['position']) < 0.025, 'Furniture playback did not restore player pose'
        self.screenshot('furniture-cleanup')


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--pid', type=int, required=True)
    p.add_argument('--live', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--suite', choices=['story', 'stress', 'furniture'], required=True)
    p.add_argument('--talk', type=int, default=5657)
    p.add_argument('--backend', choices=['General', 'Fixture'], default='Fixture')
    p.add_argument('--cycles', type=int, default=12)
    p.add_argument('--fixture', type=int, default=8)
    p.add_argument('--natural', action='store_true')
    args = p.parse_args()
    run = Acceptance(args.pid, args.live, args.out)
    try:
        if args.suite == 'story': run.story(args.talk, args.backend)
        elif args.suite == 'stress': run.stress(args.cycles)
        else: run.furniture(args.fixture, args.natural)
    except Exception:
        run.record('FAILED')
        run.screenshot('failure-state')
        raise
    run.record('PASSED')
    print(f'PASSED {args.suite}: {args.out}')

if __name__ == '__main__':
    main()
