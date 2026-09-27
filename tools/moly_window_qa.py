"""Foreground-checked Windows Moly UI driver. Real input; client-only captures.

Eligible windows must have titles starting with 'moly v'. With multiple running
instances, --pid is required. Pillow is needed only for capture. No networking,
clipboard access, persistent settings or synthetic game-state changes.
"""
from __future__ import annotations
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
from pathlib import Path
import sys
import time

if sys.platform != 'win32':
    raise SystemExit('This acceptance driver is Windows-only.')
u = C.WinDLL('user32', use_last_error=True)
u.SetProcessDPIAware()
u.GetForegroundWindow.restype = W.HWND
u.SetForegroundWindow.argtypes = (W.HWND,)
u.ShowWindow.argtypes = (W.HWND, C.c_int)
u.GetClientRect.argtypes = (W.HWND, C.POINTER(W.RECT))
u.ClientToScreen.argtypes = (W.HWND, C.POINTER(W.POINT))
u.SetWindowPos.argtypes = (W.HWND, W.HWND, C.c_int, C.c_int, C.c_int, C.c_int, W.UINT)
u.GetWindowRect.argtypes = (W.HWND, C.POINTER(W.RECT))
u.GetWindowThreadProcessId.argtypes = (W.HWND, C.POINTER(W.DWORD))
u.GetWindowTextW.argtypes = (W.HWND, W.LPWSTR, C.c_int)
u.IsWindowVisible.argtypes = (W.HWND,)
u.IsWindow.argtypes = (W.HWND,)

class MouseInput(C.Structure):
    _fields_ = [('dx', W.LONG), ('dy', W.LONG), ('mouseData', W.DWORD),
                ('dwFlags', W.DWORD), ('time', W.DWORD), ('extra', W.WPARAM)]
class KeyboardInput(C.Structure):
    _fields_ = [('vk', W.WORD), ('scan', W.WORD), ('flags', W.DWORD),
                ('time', W.DWORD), ('extra', W.WPARAM)]
class HardwareInput(C.Structure):
    _fields_ = [('message', W.DWORD), ('low', W.WORD), ('high', W.WORD)]
class InputUnion(C.Union):
    _fields_ = [('mi', MouseInput), ('ki', KeyboardInput), ('hi', HardwareInput)]
class Input(C.Structure):
    _anonymous_ = ('data',)
    _fields_ = [('type', W.DWORD), ('data', InputUnion)]
u.SendInput.argtypes = (W.UINT, C.POINTER(Input), C.c_int)
u.SendInput.restype = W.UINT


def windows():
    found = []
    @C.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
    def visit(hwnd, _):
        title, pid = C.create_unicode_buffer(512), W.DWORD()
        u.GetWindowTextW(hwnd, title, len(title))
        if title.value.startswith('moly v') and u.IsWindowVisible(hwnd):
            u.GetWindowThreadProcessId(hwnd, C.byref(pid))
            found.append(dict(hwnd=int(hwnd), pid=pid.value, title=title.value))
        return True
    u.EnumWindows(visit, 0)
    return found


def bounds(hwnd):
    rect, origin = W.RECT(), W.POINT()
    if not u.GetClientRect(hwnd, C.byref(rect)) or not u.ClientToScreen(hwnd, C.byref(origin)):
        raise C.WinError(C.get_last_error())
    return origin.x, origin.y, origin.x + rect.right, origin.y + rect.bottom


def key(vk, up=False, scan=0):
    event = Input(type=1, ki=KeyboardInput(vk, scan, (4 if scan else 0) | (2 if up else 0), 0, 0))
    if u.SendInput(1, C.byref(event), C.sizeof(Input)) != 1:
        raise C.WinError(C.get_last_error())


KEYS = dict(ESC=27, ENTER=13, SPACE=32, TAB=9, BACKSPACE=8, UP=38, DOWN=40,
            LEFT=37, RIGHT=39, HOME=36, END=35, PAGEDOWN=34, PAGEUP=33,
            CTRL=17, SHIFT=16, ALT=18, **{f'F{i}': 111 + i for i in range(1, 13)})


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--pid', type=int)
    p.add_argument('--wait', type=float, default=0.3)
    sub = p.add_subparsers(dest='action', required=True)
    sub.add_parser('list')
    sub.add_parser('capture').add_argument('path', type=Path)
    sub.add_parser('key').add_argument('keys', help='F10, ESC, CTRL+A, etc.')
    sub.add_parser('text').add_argument('value')
    for action in ('click', 'scroll'):
        a = sub.add_parser(action)
        a.add_argument('x', type=int)
        a.add_argument('y', type=int)
        if action == 'scroll':
            a.add_argument('steps', type=int, help='positive is up')
    a = sub.add_parser('resize')
    a.add_argument('width', type=int)
    a.add_argument('height', type=int)
    args = p.parse_args()
    found = [w for w in windows() if args.pid is None or w['pid'] == args.pid]
    if args.action == 'list':
        print(json.dumps(found, ensure_ascii=False))
        return
    if len(found) != 1:
        raise RuntimeError(f'Expected one Moly window; specify --pid. Found: {found}')
    hwnd = found[0]['hwnd']
    u.ShowWindow(hwnd, 9)
    u.SetForegroundWindow(hwnd)
    time.sleep(0.25)
    if int(u.GetForegroundWindow() or 0) != hwnd:
        raise RuntimeError('Moly did not obtain foreground focus; no input was sent.')
    x0, y0, x1, y1 = bounds(hwnd)
    if args.action in ('click', 'scroll'):
        if not (0 <= args.x < x1 - x0 and 0 <= args.y < y1 - y0):
            raise ValueError('Pointer coordinate is outside the Moly client area.')
        u.SetCursorPos(x0 + args.x, y0 + args.y)
        if args.action == 'click':
            u.mouse_event(2, 0, 0, 0, 0)
            time.sleep(0.08)
            u.mouse_event(4, 0, 0, 0, 0)
        else:
            u.mouse_event(0x0800, 0, 0, args.steps * 120, 0)
    elif args.action == 'key':
        keys = [KEYS.get(k, ord(k) if len(k) == 1 and k.isascii() else 0)
                for k in args.keys.upper().split('+')]
        if any(not k for k in keys):
            raise ValueError(f'Unsupported key sequence: {args.keys}')
        for vk in keys:
            key(vk)
        time.sleep(0.08)
        for vk in reversed(keys):
            key(vk, up=True)
    elif args.action == 'text':
        data = args.value.encode('utf-16-le')
        for i in range(0, len(data), 2):
            scan = int.from_bytes(data[i:i+2], 'little')
            key(0, scan=scan)
            key(0, up=True, scan=scan)
    elif args.action == 'resize':
        if not (480 <= args.width <= 2560 and 360 <= args.height <= 1440):
            raise ValueError('Allowed client sizes: 480–2560 by 360–1440.')
        outer = W.RECT()
        u.GetWindowRect(hwnd, C.byref(outer))
        width = args.width + outer.right - outer.left - (x1 - x0)
        height = args.height + outer.bottom - outer.top - (y1 - y0)
        if not u.SetWindowPos(hwnd, None, 40, 40, width, height, 0x0004):
            raise C.WinError(C.get_last_error())
    time.sleep(max(0.0, min(args.wait, 10.0)))
    if args.action == 'capture':
        from PIL import ImageGrab
        args.path.parent.mkdir(parents=True, exist_ok=True)
        ImageGrab.grab(bbox=bounds(hwnd), all_screens=True).save(args.path)
    closed = not bool(u.IsWindow(hwnd))
    print(json.dumps({**found[0], 'action': args.action, 'closed': closed,
                      'client': None if closed else bounds(hwnd)}, ensure_ascii=False))


if __name__ == '__main__':
    main()
