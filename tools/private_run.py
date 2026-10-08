#!/usr/bin/env python3
"""Run a copied candidate on owned Xvfb/Openbox with disk audio and a copied profile."""
import argparse
import array
import ctypes as C
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import time


def identity(path):
    info = path.stat()
    return dict(device=info.st_dev, inode=info.st_ino, size=info.st_size, mtime_ns=info.st_mtime_ns)


def equal_files(left, right):
    with left.open("rb") as a, right.open("rb") as b:
        while True:
            chunk = a.read(1024 * 1024)
            if chunk != b.read(1024 * 1024):
                return False
            if not chunk:
                return True


def settings(profile):
    return sorted(p for p in profile.rglob("*") if p.is_file() and not p.is_symlink()
                  and p.suffix in (".cfg", ".json")
                  and not {"saves", "assets"}.intersection(p.relative_to(profile).parts))


class XClient:
    def __init__(self, display):
        self.x = C.CDLL("libX11.so.6")
        self.xt = C.CDLL("libXtst.so.6")
        callback = C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_void_p)
        self.error_handler = callback(lambda display, event: 0)
        self.x.XSetErrorHandler.argtypes = [callback]
        self.x.XSetErrorHandler(self.error_handler)
        signatures = {
            "XOpenDisplay": ([C.c_char_p], C.c_void_p),
            "XDefaultRootWindow": ([C.c_void_p], C.c_ulong),
            "XQueryTree": ([C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)], C.c_int),
            "XFetchName": ([C.c_void_p, C.c_ulong, C.POINTER(C.c_void_p)], C.c_int),
            "XFree": ([C.c_void_p], C.c_int),
            "XCloseDisplay": ([C.c_void_p], C.c_int),
            "XFlush": ([C.c_void_p], C.c_int),
            "XSetInputFocus": ([C.c_void_p, C.c_ulong, C.c_int, C.c_ulong], C.c_int),
            "XStringToKeysym": ([C.c_char_p], C.c_ulong),
            "XKeysymToKeycode": ([C.c_void_p, C.c_ulong], C.c_ubyte),
        }
        for name, (args, result) in signatures.items():
            function = getattr(self.x, name)
            function.argtypes, function.restype = args, result
        self.xt.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        self.xt.XTestFakeRelativeMotionEvent.argtypes = [C.c_void_p, C.c_int, C.c_int, C.c_ulong]
        self.display = self.x.XOpenDisplay(display.encode())
        if not self.display:
            raise RuntimeError("cannot connect to owned private display")
        self.root = self.x.XDefaultRootWindow(self.display)

    def find_window(self, title):
        pending = [self.root]
        while pending:
            window = pending.pop()
            name = C.c_void_p()
            self.x.XFetchName(self.display, window, C.byref(name))
            if name.value:
                value = C.string_at(name).decode(errors="replace")
                self.x.XFree(name)
                if value == title:
                    return window
            root, parent, count = C.c_ulong(), C.c_ulong(), C.c_uint()
            children = C.POINTER(C.c_ulong)()
            if self.x.XQueryTree(self.display, window, C.byref(root), C.byref(parent), C.byref(children), C.byref(count)):
                pending.extend(children[i] for i in range(count.value))
                if children:
                    self.x.XFree(children)
        return None

    def drive(self, window, actions):
        if not actions:
            return
        self.x.XSetInputFocus(self.display, window, 2, 0)
        for action in actions:
            if "mouse" in action:
                self.xt.XTestFakeRelativeMotionEvent(self.display, *action["mouse"], 0)
            if "key" in action:
                key = self.x.XKeysymToKeycode(self.display, self.x.XStringToKeysym(action["key"].encode()))
                if not key:
                    raise ValueError("unknown input key: " + action["key"])
                self.xt.XTestFakeKeyEvent(self.display, key, 1, 0)
                self.x.XFlush(self.display)
                try:
                    time.sleep(action.get("hold_seconds", 0.1))
                finally:
                    self.xt.XTestFakeKeyEvent(self.display, key, 0, 0)
            self.x.XFlush(self.display)
            time.sleep(action.get("wait_seconds", 0))

    def close(self):
        self.x.XCloseDisplay(self.display)


def audio_summary(path, cues):
    result = {"path": str(path), "cues": cues, "captured": path.exists(), "format": "SDL disk S16LE, when configured by the engine"}
    if not path.exists():
        return dict(result, samples=0, non_silent_samples=0, peak=0, rms=0)
    data = array.array("h")
    raw = path.read_bytes()
    data.frombytes(raw[:len(raw) // 2 * 2])
    if os.sys.byteorder != "little":
        data.byteswap()
    return dict(result, samples=len(data), non_silent_samples=sum(v != 0 for v in data),
                peak=max((abs(v) for v in data), default=0),
                rms=(sum(v * v for v in data) / max(1, len(data))) ** 0.5)


def run(binary, profile, evidence, arguments, actions=None, timeout=30, size=(640, 400), title="Quake Anthology Rust", cores=None, stdin_bytes=None, stdin_tty=False):
    binary, profile, evidence = binary.resolve(strict=True), profile.resolve(strict=True), evidence.resolve()
    if evidence.exists():
        raise ValueError("use a fresh evidence directory")
    if "qfiles" in evidence.parts or evidence.is_relative_to(profile):
        raise ValueError("evidence must be outside qfiles and the original profile")
    evidence.mkdir(parents=True, mode=0o700)
    for name in ("home", "runtime", "config", "data", "cache", "home/.local/share/quake-anthology/content"):
        (evidence / name).mkdir(parents=True, exist_ok=True, mode=0o700)
    copied_profile = evidence / "home/.local/share/quake-anthology/content"
    sources = settings(profile)
    originals = {str(p.relative_to(profile)): p.read_bytes() for p in sources}
    for name, data in originals.items():
        target = copied_profile / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    candidate = evidence / "qa-rust"
    shutil.copy2(binary, candidate)
    if not equal_files(binary, candidate):
        raise ValueError("candidate copy differs")
    candidate_identity = identity(binary)
    owned, handles, client = [], [], None
    terminal_fds = []
    result = {"artifact": str(binary), "candidate_identity": candidate_identity,
              "owner_profile_source": str(profile), "copied_owner_settings": sorted(originals),
              "copied_profile": str(copied_profile), "argv": arguments, "gameplay_reached": False,
              "debugger": False, "timing_qualified": False, "cpu_affinity": cores}

    def spawn(name, argv, env=None, pass_fds=(), stdin=None):
        log = (evidence / (name + ".log")).open("w")
        handles.append(log)
        process = subprocess.Popen(argv, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   cwd=evidence, start_new_session=True, pass_fds=pass_fds, stdin=stdin)
        owned.append((name, process))
        (evidence / "owned-pids.json").write_text(json.dumps({n: p.pid for n, p in owned}))
        return process

    try:
        read_fd, write_fd = os.pipe()
        try:
            spawn("xvfb", ["Xvfb", "-displayfd", str(write_fd), "-screen", "0", f"{size[0]}x{size[1]}x24", "-nolisten", "tcp", "-ardelay", "500", "-arinterval", "30"], pass_fds=(write_fd,))
        finally:
            os.close(write_fd)
        try:
            if not select.select([read_fd], [], [], 10)[0]:
                raise RuntimeError("private display startup timed out")
            display = ":" + os.read(read_fd, 64).decode().strip()
        finally:
            os.close(read_fd)
        env = {"PATH": os.environ["PATH"], "LANG": "C.UTF-8", "HOME": str(evidence / "home"),
               "DISPLAY": display, "XDG_RUNTIME_DIR": str(evidence / "runtime"),
               "XDG_CONFIG_HOME": str(evidence / "config"), "XDG_CACHE_HOME": str(evidence / "cache"),
               "XDG_DATA_HOME": str(evidence / "home/.local/share"),
               "SDL_VIDEODRIVER": "x11", "SDL_AUDIODRIVER": "disk",
               "SDL_DISKAUDIOFILE": str(evidence / "audio.raw"),
               "DBUS_SESSION_BUS_ADDRESS": "unix:path=" + str(evidence / "runtime/no-owner-bus"),
               "LP_NUM_THREADS": "2"}
        wm = spawn("openbox", ["openbox", "--sm-disable"], env)
        time.sleep(0.4)
        if wm.poll() is not None:
            raise RuntimeError("private window manager failed")
        client = XClient(display)
        argv = ["env", "-u", "WAYLAND_DISPLAY", "SDL_VIDEODRIVER=x11", "SDL_AUDIODRIVER=disk", str(candidate), *arguments]
        if cores:
            argv = ["taskset", "-c", cores, *argv]
        if stdin_tty:
            import termios
            terminal_master, terminal_slave = os.openpty()
            terminal_fds += [terminal_master, terminal_slave]
            attributes = termios.tcgetattr(terminal_slave)
            attributes[3] &= ~termios.ECHO
            termios.tcsetattr(terminal_slave, termios.TCSANOW, attributes)
            game = spawn("runtime", argv, env, stdin=terminal_slave)
        else:
            game = spawn("runtime", argv, env, stdin=subprocess.PIPE if stdin_bytes is not None else subprocess.DEVNULL)
        deadline = time.monotonic() + min(10, timeout)
        window = None
        ready = False
        while game.poll() is None and time.monotonic() < deadline:
            for line in (evidence / "runtime.log").read_text().splitlines():
                try:
                    event = json.loads(line)
                    ready |= isinstance(event, dict) and event.get("event") == "window_ready"
                except json.JSONDecodeError:
                    pass
            if ready:
                window = client.find_window(title)
                if window:
                    break
            time.sleep(0.05)
        if not window:
            raise RuntimeError("candidate did not report a ready window")
        time.sleep(0.2)
        # Startup can replace the first X window before readiness. Use the
        # current client for capture/input after the engine reports readiness.
        window = client.find_window(title)
        if not window:
            raise RuntimeError("ready window closed before capture")
        result["captured_window"] = hex(window)
        result["window_ready_before_selection"] = ready
        subprocess.run(["import", "-window", hex(window), str(evidence / "window.png")], env=env, check=True, timeout=10)
        client.drive(window, actions or [])
        if stdin_bytes is not None:
            if stdin_tty:
                os.write(terminal_master, stdin_bytes)
            else:
                game.stdin.write(stdin_bytes)
                game.stdin.close()
        result["exit_code"] = game.wait(timeout=timeout)
        result["normal_exit"] = result["exit_code"] == 0
        events = []
        for line in (evidence / "runtime.log").read_text().splitlines():
            try:
                event = json.loads(line)
                if isinstance(event, dict):
                    events.append(event)
            except json.JSONDecodeError:
                pass
        result["gameplay_reached"] = any(v.get("event") == "gameplay_ready" for v in events)
        result["events"] = events
        result["private_containment"] = {"display": display, "video": "owned Xvfb with Openbox",
                                         "audio": "SDL disk", "home": env["HOME"],
                                         "wayland_display_unset": "WAYLAND_DISPLAY" not in env,
                                         "sdl_video_driver": env["SDL_VIDEODRIVER"]}
        result["screenshot"] = str(evidence / "window.png")
        result["result"] = "PASS" if result["normal_exit"] else "FAIL"
    except Exception as error:
        result.update(result="FAIL", error=str(error))
    finally:
        if client:
            client.close()
        for _, process in reversed(owned):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        for handle in handles:
            handle.close()
        for fd in terminal_fds:
            os.close(fd)
        result["remaining_owned_pids"] = [p.pid for _, p in owned if p.poll() is None]
        result["owner_profile_unchanged"] = originals == {str(p.relative_to(profile)): p.read_bytes() for p in settings(profile)}
        result["candidate_unchanged"] = identity(binary) == candidate_identity and equal_files(binary, candidate)
        cues = [v.get("cue") for v in result.get("events", []) if v.get("event") == "sound_cue"]
        audio = audio_summary(evidence / "audio.raw", cues)
        (evidence / "audio-summary.json").write_text(json.dumps(audio, indent=2) + "\n")
        (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--owner-profile", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--input", type=Path)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--width", type=int, default=640)
    parser.add_argument("--height", type=int, default=400)
    parser.add_argument("--cores")
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
    result = run(args.binary, args.owner_profile, args.evidence, arguments,
                 json.loads(args.input.read_text()) if args.input else None,
                 args.timeout, (args.width, args.height), cores=args.cores)
    print(json.dumps({"evidence": str(args.evidence), "result": result["result"],
                      "gameplay_reached": result["gameplay_reached"], "remaining_owned_pids": result["remaining_owned_pids"]}))
    return 0 if result["result"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
